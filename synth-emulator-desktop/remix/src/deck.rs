//! One playback deck: transport, cue points, beat loops, tempo and sync.
//!
//! Runs on the audio thread. `process` and `handle` never allocate; a
//! replaced track is handed back to the caller so it is not freed here.

use std::sync::Arc;

use crate::track::Track;

pub const HOT_CUES: usize = 6;
/// Tempo fader ranges (fraction): ±8 %, ±16 %, ±50 %.
pub const TEMPO_RANGES: [f32; 3] = [0.08, 0.16, 0.50];
/// Speed change while a nudge key is held.
pub const NUDGE_AMOUNT: f64 = 0.04;
/// Length of the crossfade used to hide any jump in the playhead.
pub const DECLICK_SECS: f32 = 0.004;
/// Beat length used for loops and jumps when the track has no tempo.
pub const FALLBACK_BEAT_SECS: f64 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoopState {
    pub start_secs: f64,
    pub end_secs: f64,
    pub beats: f32,
}

#[derive(Clone, Debug)]
pub enum DeckCmd {
    /// Load and stop. Playhead and cue point go to the grid anchor
    /// (`first_beat_secs`, or 0 without tempo). Clears loop and hot cues.
    Load(Arc<Track>),
    Eject,
    TogglePlay,
    Play,
    Pause,
    /// CDJ-style CUE. Playing: jump back to the cue point and pause.
    /// Paused: set the cue point at the playhead (snapped to the nearest
    /// beat when the deck has tempo and the playhead is within 1/8 beat).
    Cue,
    /// Empty slot: store the playhead (quantized to the nearest beat when
    /// within 1/8 beat). Set slot: jump there and play.
    HotCue(usize),
    ClearHotCue(usize),
    /// Restore persisted hot cues (seconds).
    SetHotCues([Option<f64>; HOT_CUES]),
    /// Loop `beats` long starting at the beat the playhead is in
    /// (floor on the grid; free start without tempo). Pressing the same
    /// length while that loop is active exits it.
    BeatLoop(f32),
    ExitLoop,
    LoopHalve,
    LoopDouble,
    /// Tempo fader -1..=1; speed = 1 + fader * range.
    SetTempoFader(f32),
    /// Tempo speed (1.0 = original), clamped to the widest range. Picks the
    /// range that fits, like a sync does.
    SetTempoSpeed(f64),
    /// One of `TEMPO_RANGES`; keeps the current speed when possible.
    SetTempoRange(f32),
    /// Held nudge: -1, 0 or +1. Multiplies speed by 1 ± NUDGE_AMOUNT.
    Nudge(f32),
    /// Seek to a fraction 0..=1 of the track.
    SeekFraction(f64),
    /// Jump by whole or fractional beats (negative = back).
    BeatJump(f32),
    /// Match effective BPM to `bpm` by moving the tempo fader (widening the
    /// range if needed). With `phase` (0..1, the other deck's beat phase),
    /// also shift the playhead by less than half a beat (or, when that would
    /// cross the start or end of the track, by less than a beat the other way)
    /// so phases match. Inside an active loop the playhead wraps around the
    /// loop instead, and the loop is kept. An ended deck is aligned from its
    /// cue point, so a following Play restarts it there.
    Sync {
        bpm: f64,
        phase: Option<f64>,
    },
    /// Override the base BPM (tap tempo / manual), keeping the anchor.
    SetBpm(f64),
    /// Move the grid anchor so a beat falls exactly on the playhead.
    SetGridAnchorHere,
}

/// Plain-data view of a deck for the UI. `Copy`, so taking it on the audio
/// thread does not allocate.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DeckState {
    pub loaded: bool,
    pub track_id: u64,
    pub playing: bool,
    /// The playhead reached the end and stopped.
    pub ended: bool,
    pub position_secs: f64,
    pub duration_secs: f64,
    pub cue_secs: f64,
    pub hot_cues: [Option<f64>; HOT_CUES],
    pub loop_state: Option<LoopState>,
    pub tempo_fader: f32,
    pub tempo_range: f32,
    /// Speed including nudge (1.0 = original).
    pub speed: f64,
    pub base_bpm: Option<f64>,
    /// `base_bpm * speed`.
    pub bpm: Option<f64>,
    pub first_beat_secs: f64,
    /// 0..1 position inside the current beat.
    pub beat_phase: Option<f64>,
}

/// `TEMPO_RANGES` in f64, so speed arithmetic matches the decimal values
/// exactly. A test checks that the two tables agree.
const RANGES_F64: [f64; 3] = [0.08, 0.16, 0.50];
/// Smallest loop any command may produce, in track frames.
const MIN_LOOP_FRAMES: f64 = 1.0;
/// Shortest loop, in beats, for both BeatLoop and LoopHalve.
const MIN_LOOP_BEATS: f32 = 1.0 / 32.0;
/// Hot-cue and loop snapping tolerance, in beats.
const SNAP_BEATS: f64 = 1.0 / 8.0;

/// Number of declick crossfades that can overlap. A jump that starts while
/// all slots are busy overwrites the quietest one.
const GHOSTS: usize = 4;

/// A playhead left behind by a jump or loop wrap. It keeps sounding and
/// fades out linearly. Gain 0 means the slot is idle.
#[derive(Clone, Copy, Debug)]
struct Ghost {
    pos: f64,
    gain: f32,
}

const IDLE_GHOST: Ghost = Ghost {
    pos: 0.0,
    gain: 0.0,
};

/// Loop region in track frames.
#[derive(Clone, Copy, Debug)]
struct LoopFrames {
    start: f64,
    end: f64,
    /// Requested length in beats, after clamping and exact halving/doubling.
    /// It is kept when the region is clamped at a track bound (LoopDouble,
    /// BeatJump), on purpose: BeatLoop compares it to detect a repeat press.
    beats: f32,
}

pub struct Deck {
    /// Engine sample rate (sanitised in `new`).
    sample_rate: f64,
    /// Declick ramp per output frame: 1 / (DECLICK_SECS * engine rate).
    ramp: f32,
    track: Option<Arc<Track>>,
    /// Live playhead in track frames.
    pos: f64,
    playing: bool,
    /// Playhead reached the end of the track and stopped.
    ended: bool,
    /// Gain of the live voice, 0..=1. Ramps up while playing, down otherwise.
    gain: f32,
    /// Declick crossfades still sounding (see `Ghost`).
    ghosts: [Ghost; GHOSTS],
    /// Cue point, track frames.
    cue: f64,
    /// Hot cue slots, track frames.
    hot: [Option<f64>; HOT_CUES],
    loop_region: Option<LoopFrames>,
    /// Tempo fader -1..=1 (f64 for exact speed arithmetic).
    tempo_fader: f64,
    /// Current tempo range, one of `RANGES_F64`.
    tempo_range: f64,
    /// Held nudge direction: -1, 0 or +1.
    nudge: f64,
    /// Base BPM set by the user; overrides the analysed tempo.
    bpm_override: Option<f64>,
    /// Grid anchor set by the user (track seconds); overrides the analysis.
    anchor_override: Option<f64>,
}

impl Deck {
    pub fn new(sample_rate: f32) -> Self {
        let sample_rate = if sample_rate.is_finite() && sample_rate > 0.0 {
            sample_rate as f64
        } else {
            48_000.0
        };
        let ramp_frames = (DECLICK_SECS as f64 * sample_rate).round().max(1.0);
        Self {
            sample_rate,
            ramp: (1.0 / ramp_frames) as f32,
            track: None,
            pos: 0.0,
            playing: false,
            ended: false,
            gain: 0.0,
            ghosts: [IDLE_GHOST; GHOSTS],
            cue: 0.0,
            hot: [None; HOT_CUES],
            loop_region: None,
            tempo_fader: 0.0,
            tempo_range: RANGES_F64[0],
            nudge: 0.0,
            bpm_override: None,
            anchor_override: None,
        }
    }

    /// Apply a command. Returns the previous track when one was replaced or
    /// ejected, so the caller can free it off the audio thread.
    pub fn handle(&mut self, cmd: DeckCmd) -> Option<Arc<Track>> {
        match cmd {
            DeckCmd::Load(track) => return self.load(track),
            DeckCmd::Eject => return self.eject(),
            DeckCmd::TogglePlay => {
                if self.playing {
                    self.pause();
                } else {
                    self.play();
                }
            }
            DeckCmd::Play => self.play(),
            DeckCmd::Pause => self.pause(),
            DeckCmd::Cue => self.press_cue(),
            DeckCmd::HotCue(i) => self.hot_cue(i),
            DeckCmd::ClearHotCue(i) => {
                if let Some(slot) = self.hot.get_mut(i) {
                    *slot = None;
                }
            }
            DeckCmd::SetHotCues(cues) => self.set_hot_cues(cues),
            DeckCmd::BeatLoop(beats) => self.beat_loop(beats),
            DeckCmd::ExitLoop => self.loop_region = None,
            DeckCmd::LoopHalve => self.loop_halve(),
            DeckCmd::LoopDouble => self.loop_double(),
            DeckCmd::SetTempoFader(fader) => {
                if fader.is_finite() {
                    self.tempo_fader = (fader as f64).clamp(-1.0, 1.0);
                }
            }
            DeckCmd::SetTempoRange(range) => self.set_tempo_range(range),
            DeckCmd::SetTempoSpeed(speed) => {
                if speed.is_finite() {
                    let widest = RANGES_F64[RANGES_F64.len() - 1];
                    self.fit_tempo(speed.clamp(1.0 - widest, 1.0 + widest));
                }
            }
            DeckCmd::Nudge(dir) => {
                self.nudge = if dir > 0.0 {
                    1.0
                } else if dir < 0.0 {
                    -1.0
                } else {
                    0.0
                };
            }
            DeckCmd::SeekFraction(fraction) => self.seek_fraction(fraction),
            DeckCmd::BeatJump(beats) => self.beat_jump(beats),
            DeckCmd::Sync { bpm, phase } => self.sync(bpm, phase),
            DeckCmd::SetBpm(bpm) => {
                if bpm.is_finite() && (30.0..=300.0).contains(&bpm) {
                    self.bpm_override = Some(bpm);
                }
            }
            DeckCmd::SetGridAnchorHere => {
                if self.track.is_some() {
                    self.anchor_override = Some(self.pos_secs());
                }
            }
        }
        None
    }

    /// Next stereo frame at the engine rate.
    #[inline]
    pub fn process(&mut self) -> (f32, f32) {
        let Some(track) = self.track.as_deref() else {
            return (0.0, 0.0);
        };
        let live_on = self.playing || self.gain > 0.0;
        if !live_on && self.ghosts.iter().all(|g| g.gain <= 0.0) {
            return (0.0, 0.0);
        }
        let track_rate = rate_of(track);
        let frames = track.frames() as f64;
        let step = self.step(track_rate);
        let mut left = 0.0f32;
        let mut right = 0.0f32;

        if live_on {
            let (l, r) = read(track, self.pos);
            left += l * self.gain;
            right += r * self.gain;
        }
        for ghost in self.ghosts.iter_mut() {
            if ghost.gain > 0.0 {
                let (l, r) = read(track, ghost.pos);
                left += l * ghost.gain;
                right += r * ghost.gain;
                ghost.pos += step;
                ghost.gain -= self.ramp;
                if ghost.gain <= 0.0 {
                    *ghost = IDLE_GHOST;
                }
            }
        }

        // The playhead keeps moving while the live voice is audible, so a
        // pause fades out at the position it was heard at.
        let advance = self.playing || (self.gain > 0.0 && !self.ended);
        if self.playing {
            self.gain = (self.gain + self.ramp).min(1.0);
        } else {
            self.gain = (self.gain - self.ramp).max(0.0);
        }
        if advance {
            self.pos = (self.pos + step).min(frames);
            // The wrap runs whenever the playhead moves, so a pause that
            // fades past the loop end lands back inside the loop.
            if let Some(region) = self.loop_region {
                if self.pos >= region.end {
                    // Declicked wrap: the old playhead fades out past the loop
                    // end while the live voice fades in at the loop start.
                    let len = region.end - region.start;
                    let target = region.start + (self.pos - region.end).rem_euclid(len);
                    self.park_ghost(self.pos, self.gain);
                    self.pos = target;
                    self.gain = 0.0;
                }
            }
        }

        if self.playing && self.pos >= frames {
            self.ended = true;
            self.playing = false;
            self.pos = frames;
        }

        (finite_or_zero(left), finite_or_zero(right))
    }

    pub fn state(&self) -> DeckState {
        let track = self.track.as_deref();
        let rate = self.rate();
        let duration = self.max_frames() / rate;
        let position = if self.ended {
            duration
        } else {
            self.pos_secs().clamp(0.0, duration)
        };
        let base_bpm = self.base_bpm();
        let speed = self.speed();
        DeckState {
            loaded: track.is_some(),
            track_id: track.map_or(0, |t| t.id),
            playing: self.playing,
            ended: self.ended,
            position_secs: position,
            duration_secs: duration,
            cue_secs: self.cue / rate,
            hot_cues: self.hot.map(|slot| slot.map(|f| f / rate)),
            loop_state: self.loop_region.map(|l| LoopState {
                start_secs: l.start / rate,
                end_secs: l.end / rate,
                beats: l.beats,
            }),
            tempo_fader: self.tempo_fader as f32,
            tempo_range: self.tempo_range as f32,
            speed,
            base_bpm,
            bpm: base_bpm.map(|b| b * speed),
            first_beat_secs: self.anchor_secs(),
            beat_phase: self.beat_phase_at(self.pos_secs()),
        }
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// `base_bpm * speed`, if the track has tempo.
    pub fn effective_bpm(&self) -> Option<f64> {
        self.base_bpm().map(|b| b * self.speed())
    }

    pub fn beat_phase(&self) -> Option<f64> {
        self.beat_phase_at(self.pos_secs())
    }

    /// `base_bpm * tempo speed`: the tempo without a held nudge.
    pub fn tempo_bpm(&self) -> Option<f64> {
        self.base_bpm().map(|b| b * self.tempo_speed())
    }
}

/// Sample rate of a track, guarded against zero.
fn rate_of(track: &Track) -> f64 {
    if track.sample_rate > 0 {
        track.sample_rate as f64
    } else {
        1.0
    }
}

fn finite_or_zero(v: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        0.0
    }
}

/// Catmull-Rom (cubic Hermite) interpolation between `y1` and `y2`.
/// Exact at `t == 0`.
#[inline]
fn hermite(y0: f32, y1: f32, y2: f32, y3: f32, t: f32) -> f32 {
    let c1 = 0.5 * (y2 - y0);
    let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
    let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
    ((c3 * t + c2) * t + c1) * t + y1
}

/// Interpolated stereo read of `track` at a fractional frame position.
/// Silence for positions that are not finite or lie more than two frames
/// outside the track, so the integer index below never overflows.
#[inline]
fn read(track: &Track, pos: f64) -> (f32, f32) {
    let frames = track.frames() as f64;
    if !pos.is_finite() || pos < -2.0 || pos > frames + 2.0 {
        return (0.0, 0.0);
    }
    let base = pos.floor();
    let t = (pos - base) as f32;
    let i = base as isize;
    let (l0, r0) = track.frame(i - 1);
    let (l1, r1) = track.frame(i);
    let (l2, r2) = track.frame(i + 1);
    let (l3, r3) = track.frame(i + 2);
    (hermite(l0, l1, l2, l3, t), hermite(r0, r1, r2, r3, t))
}

impl Deck {
    fn rate(&self) -> f64 {
        self.track.as_deref().map_or(1.0, rate_of)
    }

    /// Track length in frames (0 when nothing is loaded).
    fn max_frames(&self) -> f64 {
        self.track.as_deref().map_or(0.0, |t| t.frames() as f64)
    }

    fn pos_secs(&self) -> f64 {
        self.pos / self.rate()
    }

    /// Tempo part of the speed: `1 + fader * range`.
    pub fn tempo_speed(&self) -> f64 {
        1.0 + self.tempo_fader * self.tempo_range
    }

    /// Full speed including the held nudge.
    fn speed(&self) -> f64 {
        self.tempo_speed() * (1.0 + NUDGE_AMOUNT * self.nudge)
    }

    /// Playhead advance per output frame.
    fn step(&self, track_rate: f64) -> f64 {
        let step = self.speed() * track_rate / self.sample_rate;
        if step.is_finite() {
            step
        } else {
            0.0
        }
    }

    /// Base BPM: user override first, then the analysed tempo.
    fn base_bpm(&self) -> Option<f64> {
        let track = self.track.as_deref()?;
        self.bpm_override
            .or(track.analysis.bpm)
            .filter(|b| b.is_finite() && *b > 0.0)
    }

    fn beat_len_secs(&self) -> Option<f64> {
        self.base_bpm().map(|b| 60.0 / b)
    }

    /// Beat length used for loops and jumps.
    fn grid_beat_secs(&self) -> f64 {
        self.beat_len_secs().unwrap_or(FALLBACK_BEAT_SECS)
    }

    /// Grid anchor in track seconds (0 when nothing is loaded).
    fn anchor_secs(&self) -> f64 {
        let anchor = self
            .anchor_override
            .or_else(|| self.track.as_deref().map(|t| t.analysis.first_beat_secs))
            .unwrap_or(0.0);
        if anchor.is_finite() {
            anchor
        } else {
            0.0
        }
    }

    fn beat_phase_at(&self, secs: f64) -> Option<f64> {
        let len = self.beat_len_secs()?;
        let phase = ((secs - self.anchor_secs()) / len).rem_euclid(1.0);
        if !phase.is_finite() {
            return None;
        }
        Some(if phase >= 1.0 { 0.0 } else { phase })
    }

    /// Floor a frame position to a grid of `step_beats` beats anchored on the
    /// grid anchor (free when there is no tempo).
    fn grid_floor(&self, frames: f64, step_beats: f64) -> f64 {
        let Some(len) = self.beat_len_secs() else {
            return frames;
        };
        let rate = self.rate();
        let anchor = self.anchor_secs();
        let unit = step_beats * len;
        let n = ((frames / rate - anchor) / unit + 1e-9).floor();
        (anchor + n * unit) * rate
    }

    /// Snap a frame position to the nearest beat when within 1/8 beat.
    fn quantise(&self, frames: f64) -> f64 {
        let Some(len) = self.beat_len_secs() else {
            return frames;
        };
        let rate = self.rate();
        let secs = frames / rate;
        let anchor = self.anchor_secs();
        let grid = anchor + ((secs - anchor) / len).round() * len;
        if (secs - grid).abs() <= SNAP_BEATS * len {
            (grid * rate).clamp(0.0, self.max_frames())
        } else {
            frames
        }
    }

    /// Keep a playhead that is still audible as a crossfade ghost. Silent
    /// voices (gain 0) are not stored. When every slot is busy the quietest
    /// one is overwritten.
    fn park_ghost(&mut self, pos: f64, gain: f32) {
        if gain.is_nan() || gain <= 0.0 || !pos.is_finite() {
            return;
        }
        let mut slot = 0;
        let mut quietest = f32::INFINITY;
        for (i, ghost) in self.ghosts.iter().enumerate() {
            if ghost.gain <= 0.0 {
                slot = i;
                break;
            }
            if ghost.gain < quietest {
                quietest = ghost.gain;
                slot = i;
            }
        }
        self.ghosts[slot] = Ghost { pos, gain };
    }

    /// A jump to a point outside the active loop exits the loop, as in other
    /// DJ software. A jump inside the loop keeps it.
    fn exit_loop_if_outside(&mut self, target: f64) {
        let target = target.clamp(0.0, self.max_frames());
        if let Some(region) = self.loop_region {
            if target < region.start || target >= region.end {
                self.loop_region = None;
            }
        }
    }

    /// Declicked jump: the old playhead fades out while the live voice
    /// fades in from silence (it stays silent when the deck is paused).
    fn jump_to(&mut self, target: f64) {
        if !target.is_finite() {
            return;
        }
        let target = target.clamp(0.0, self.max_frames());
        self.park_ghost(self.pos, self.gain);
        self.pos = target;
        self.gain = 0.0;
        self.ended = false;
    }

    /// Forget everything tied to the previous track. Tempo fader and range
    /// are kept.
    fn clear_track_state(&mut self) {
        self.playing = false;
        self.ended = false;
        self.gain = 0.0;
        self.ghosts = [IDLE_GHOST; GHOSTS];
        self.loop_region = None;
        self.hot = [None; HOT_CUES];
        self.nudge = 0.0;
        self.bpm_override = None;
        self.anchor_override = None;
    }

    fn load(&mut self, track: Arc<Track>) -> Option<Arc<Track>> {
        let old = self.track.replace(track);
        self.clear_track_state();
        let start = if self.base_bpm().is_some() {
            self.anchor_secs()
        } else {
            0.0
        };
        self.pos = (start * self.rate()).clamp(0.0, self.max_frames());
        self.cue = self.pos;
        old
    }

    fn eject(&mut self) -> Option<Arc<Track>> {
        let old = self.track.take();
        self.clear_track_state();
        self.pos = 0.0;
        self.cue = 0.0;
        old
    }

    fn play(&mut self) {
        if self.track.is_none() || self.playing {
            return;
        }
        if self.ended {
            let cue = self.cue;
            self.jump_to(cue);
        }
        self.playing = true;
    }

    fn pause(&mut self) {
        self.playing = false;
    }

    fn press_cue(&mut self) {
        if self.track.is_none() {
            return;
        }
        if self.playing {
            let cue = self.cue;
            self.exit_loop_if_outside(cue);
            self.jump_to(cue);
            self.playing = false;
        } else {
            self.cue = self.quantise(self.pos);
        }
    }

    fn hot_cue(&mut self, index: usize) {
        if self.track.is_none() || index >= HOT_CUES {
            return;
        }
        match self.hot[index] {
            Some(target) => {
                self.exit_loop_if_outside(target);
                self.jump_to(target);
                self.playing = true;
            }
            None => {
                let stored = self.quantise(self.pos);
                self.hot[index] = Some(stored);
            }
        }
    }

    fn set_hot_cues(&mut self, cues: [Option<f64>; HOT_CUES]) {
        if self.track.is_none() {
            return;
        }
        let rate = self.rate();
        let max = self.max_frames();
        for (slot, cue) in self.hot.iter_mut().zip(cues) {
            *slot = cue
                .filter(|s| s.is_finite() && *s >= 0.0)
                .map(|s| (s * rate).min(max));
        }
    }

    fn beat_loop(&mut self, beats: f32) {
        if self.track.is_none() || !beats.is_finite() {
            return;
        }
        let beats = beats.clamp(MIN_LOOP_BEATS, 64.0);
        // Pressing the same requested length again exits the loop, even when
        // the loop was clamped at the track end.
        if let Some(region) = self.loop_region {
            if (region.beats - beats).abs() < 1e-6 {
                self.loop_region = None;
                return;
            }
        }
        let max = self.max_frames();
        let len = beats as f64 * self.grid_beat_secs() * self.rate();
        // Whole-beat loops start on the beat grid. Sub-beat loops start on a
        // grid of their own length, so the playhead is inside the loop.
        let step_beats = if beats < 1.0 { beats as f64 } else { 1.0 };
        let start = self.grid_floor(self.pos, step_beats).clamp(0.0, max);
        let end = (start + len).min(max);
        if end - start <= 0.0 {
            return;
        }
        self.loop_region = Some(LoopFrames { start, end, beats });
    }

    fn loop_halve(&mut self) {
        let Some(region) = self.loop_region else {
            return;
        };
        // Stop at the shortest loop BeatLoop makes, and at a loop no shorter
        // than the declick crossfade: each wrap of a shorter loop fades its
        // live voice in from silence and the loop fades out and buzzes.
        let len = (region.end - region.start) * 0.5;
        let min_frames = (f64::from(DECLICK_SECS) * self.sample_rate).max(MIN_LOOP_FRAMES);
        if len.is_nan() || len < min_frames || region.beats * 0.5 < MIN_LOOP_BEATS {
            return;
        }
        let end = region.start + len;
        self.loop_region = Some(LoopFrames {
            start: region.start,
            end,
            beats: region.beats * 0.5,
        });
        if self.pos >= end {
            let target = region.start + (self.pos - end).rem_euclid(len);
            self.jump_to(target);
        }
    }

    fn loop_double(&mut self) {
        let Some(region) = self.loop_region else {
            return;
        };
        let end = (region.start + (region.end - region.start) * 2.0).min(self.max_frames());
        if end.is_nan() || end <= region.start {
            return;
        }
        self.loop_region = Some(LoopFrames {
            start: region.start,
            end,
            beats: region.beats * 2.0,
        });
    }

    fn set_tempo_range(&mut self, range: f32) {
        let Some(index) = TEMPO_RANGES.iter().position(|r| (*r - range).abs() < 1e-6) else {
            return;
        };
        // Keep the current speed, clamping the fader if it no longer fits.
        let speed = self.tempo_speed();
        self.tempo_range = RANGES_F64[index];
        self.tempo_fader = ((speed - 1.0) / self.tempo_range).clamp(-1.0, 1.0);
    }

    fn seek_fraction(&mut self, fraction: f64) {
        if self.track.is_none() || !fraction.is_finite() {
            return;
        }
        let fraction = fraction.clamp(0.0, 1.0);
        let target = fraction * self.max_frames();
        self.exit_loop_if_outside(target);
        self.jump_to(target);
    }

    fn beat_jump(&mut self, beats: f32) {
        if self.track.is_none() || !beats.is_finite() {
            return;
        }
        let max = self.max_frames();
        let delta = beats as f64 * self.grid_beat_secs() * self.rate();
        let old = self.pos;
        let target = (old + delta).clamp(0.0, max);
        let applied = target - old;
        if applied.is_nan() || applied.abs() <= 0.0 {
            return;
        }
        if let Some(region) = self.loop_region {
            let len = region.end - region.start;
            let fits = region.start + applied >= 0.0 && region.end + applied <= max;
            let (start, end) = if fits {
                (region.start + applied, region.end + applied)
            } else {
                // The loop would cross a track bound. Keep its length (a loop
                // already clamped at the track end keeps its size, and its
                // beats label stays true) and re-anchor it inside the track so
                // it still holds the target.
                let start = (region.start + applied).clamp(0.0, (max - len).max(0.0));
                (start, start + len)
            };
            self.loop_region = if end - start >= MIN_LOOP_FRAMES {
                Some(LoopFrames {
                    start,
                    end,
                    beats: region.beats,
                })
            } else {
                None
            };
        }
        self.jump_to(target);
    }

    /// Choose the tempo range and fader that realise `speed`.
    fn fit_tempo(&mut self, speed: f64) {
        let dev = (speed - 1.0).abs();
        if dev > self.tempo_range {
            self.tempo_range = RANGES_F64
                .into_iter()
                .find(|r| dev <= *r)
                .unwrap_or(RANGES_F64[2]);
        }
        self.tempo_fader = ((speed - 1.0) / self.tempo_range).clamp(-1.0, 1.0);
    }

    fn sync(&mut self, bpm: f64, phase: Option<f64>) {
        let Some(base) = self.base_bpm() else {
            return;
        };
        if !(bpm.is_finite() && bpm > 0.0) {
            return;
        }
        // Half and double time are candidates too.
        let candidates = [bpm, bpm * 2.0, bpm * 0.5];
        let mut target = candidates[0];
        let mut best = f64::INFINITY;
        for candidate in candidates {
            let dev = (candidate / base - 1.0).abs();
            if dev < best {
                best = dev;
                target = candidate;
            }
        }
        self.fit_tempo(target / base);

        let Some(other) = phase.filter(|p| p.is_finite()) else {
            return;
        };
        // An ended deck restarts from its cue (see `play`), so its phase is
        // aligned from there. Aligning from the end would clear `ended` and
        // leave nothing to play.
        let from = if self.ended { self.cue } else { self.pos };
        let own = self.beat_phase_at(from / self.rate()).unwrap_or(0.0);
        let mut delta = (other - own).rem_euclid(1.0);
        if delta > 0.5 {
            delta -= 1.0;
        }
        if self.ended || delta.abs() > 0.0 {
            let beat = (60.0 / base) * self.rate();
            let mut target = from + delta * beat;
            if let Some(region) = self.loop_region {
                // A phase nudge is not a jump: inside a loop the playhead wraps
                // around it, and the loop is kept. The wrap is by whole beats
                // when the loop holds one or more, so the grid phase survives
                // loops that are not a whole number of beats; a sub-beat loop
                // wraps by its own length.
                let len = region.end - region.start;
                if len > 0.0 {
                    let step = if len >= beat {
                        ((len / beat) + 1e-9).floor() * beat
                    } else {
                        len
                    };
                    target = region.start + (target - region.start).rem_euclid(step);
                }
            } else if target < 0.0 {
                target += beat;
            } else if target > self.max_frames() {
                target -= beat;
            }
            self.jump_to(target);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::Analysis;

    const SR: u32 = 48_000;

    fn analysis(bpm: Option<f64>, first_beat_secs: f64) -> Analysis {
        Analysis {
            bpm,
            first_beat_secs,
            confidence: 1.0,
        }
    }

    /// Stereo track of `secs` seconds; `gen(i)` gives frame `i` on both channels.
    fn make_track(
        sr: u32,
        secs: f64,
        bpm: Option<f64>,
        first_beat: f64,
        gen: impl Fn(usize) -> f32,
    ) -> Arc<Track> {
        let frames = (secs * sr as f64).round() as usize;
        let mut samples = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            let v = gen(i);
            samples.push(v);
            samples.push(v);
        }
        Arc::new(Track::from_samples(
            "test",
            samples,
            sr,
            analysis(bpm, first_beat),
        ))
    }

    fn sine(sr: u32, hz: f64, amp: f32) -> impl Fn(usize) -> f32 {
        move |i| amp * (2.0 * std::f64::consts::PI * hz * i as f64 / sr as f64).sin() as f32
    }

    /// Deterministic, non-periodic material.
    fn noise(i: usize) -> f32 {
        let x = (i.wrapping_mul(2_654_435_761) % 2003) as f32;
        x / 1001.5 - 1.0
    }

    fn loaded(track: Arc<Track>) -> Deck {
        let mut deck = Deck::new(SR as f32);
        deck.handle(DeckCmd::Load(track));
        deck
    }

    fn seek_secs(deck: &mut Deck, secs: f64) {
        let dur = deck.state().duration_secs;
        deck.handle(DeckCmd::SeekFraction(secs / dur));
    }

    fn run(deck: &mut Deck, frames: usize) {
        for _ in 0..frames {
            deck.process();
        }
    }

    fn max_step(track: &Track) -> f32 {
        let left: Vec<f32> = track.samples.iter().step_by(2).copied().collect();
        left.windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max)
    }

    fn assert_state_finite(s: &DeckState) {
        assert!(s.position_secs.is_finite());
        assert!(s.duration_secs.is_finite());
        assert!(s.cue_secs.is_finite());
        assert!(s.speed.is_finite());
        assert!(s.tempo_fader.is_finite());
        assert!(s.tempo_range.is_finite());
        assert!(s.first_beat_secs.is_finite());
        assert!(s.bpm.is_none_or(f64::is_finite));
        assert!(s.beat_phase.is_none_or(f64::is_finite));
        assert!(s.hot_cues.iter().flatten().all(|c| c.is_finite()));
        if let Some(l) = s.loop_state {
            assert!(l.start_secs.is_finite() && l.end_secs.is_finite());
        }
    }

    #[test]
    fn range_table_matches_tempo_ranges() {
        for (exact, public) in RANGES_F64.iter().zip(TEMPO_RANGES.iter()) {
            assert_eq!(*exact as f32, *public);
        }
    }

    #[test]
    fn speed_one_plays_track_samples_exactly_after_fade_in() {
        let track = make_track(SR, 1.0, Some(120.0), 0.0, noise);
        let mut deck = loaded(track.clone());
        deck.handle(DeckCmd::Play);
        // The fade-in is DECLICK_SECS (192 frames at 48 kHz); skip past it.
        let fade_frames = 200;
        run(&mut deck, fade_frames);
        for i in fade_frames..fade_frames + 500 {
            let out = deck.process();
            assert_eq!(out, track.frame(i as isize), "frame {i}");
        }
    }

    #[test]
    fn tempo_fader_sets_playback_speed() {
        let track = make_track(SR, 3.0, Some(120.0), 0.0, sine(SR, 220.0, 0.3));
        let mut deck = loaded(track);
        deck.handle(DeckCmd::SetTempoRange(0.16));
        deck.handle(DeckCmd::SetTempoFader(0.5));
        deck.handle(DeckCmd::Play);
        run(&mut deck, SR as usize);
        let s = deck.state();
        assert!(
            (s.position_secs - 1.08).abs() < 0.001,
            "{}",
            s.position_secs
        );
        assert!((s.speed - 1.08).abs() < 1e-9);
        assert!((s.bpm.unwrap() - 129.6).abs() < 1e-6);
    }

    #[test]
    fn track_rate_is_respected_when_engine_rate_differs() {
        let track = make_track(44_100, 3.0, Some(120.0), 0.0, sine(44_100, 220.0, 0.3));
        let mut deck = loaded(track);
        deck.handle(DeckCmd::Play);
        run(&mut deck, SR as usize);
        let s = deck.state();
        assert!((s.position_secs - 1.0).abs() < 0.001, "{}", s.position_secs);
        assert!((s.duration_secs - 3.0).abs() < 1e-9);
    }

    #[test]
    fn beat_loop_wraps_inside_region_and_exits_on_repeat() {
        let track = make_track(SR, 12.0, Some(120.0), 0.0, sine(SR, 220.0, 0.3));
        let mut deck = loaded(track);
        seek_secs(&mut deck, 1.3);
        deck.handle(DeckCmd::BeatLoop(4.0));
        let lp = deck.state().loop_state.expect("loop should start");
        assert!((lp.start_secs - 1.0).abs() < 1e-9);
        assert!((lp.end_secs - 3.0).abs() < 1e-9);
        deck.handle(DeckCmd::Play);
        for _ in 0..(10 * SR as usize) {
            deck.process();
            let p = deck.state().position_secs;
            assert!((1.0..3.0).contains(&p), "position {p} left the loop");
        }
        assert!(deck.is_playing());
        deck.handle(DeckCmd::BeatLoop(4.0));
        assert!(deck.state().loop_state.is_none());
    }

    #[test]
    fn hot_cue_stores_quantised_point_then_jumps_and_plays() {
        let track = make_track(SR, 6.0, Some(120.0), 0.0, sine(SR, 220.0, 0.3));
        let mut deck = loaded(track);
        seek_secs(&mut deck, 1.02);
        deck.handle(DeckCmd::HotCue(0));
        let stored = deck.state().hot_cues[0].expect("slot should be set");
        assert!((stored - 1.0).abs() < 1e-9, "stored {stored}");
        deck.handle(DeckCmd::Play);
        run(&mut deck, 2 * SR as usize);
        deck.handle(DeckCmd::HotCue(0));
        let s = deck.state();
        assert!(s.playing);
        assert!((s.position_secs - 1.0).abs() < 1e-9, "{}", s.position_secs);
        // Out-of-range slots are ignored.
        deck.handle(DeckCmd::HotCue(HOT_CUES));
        assert_eq!(deck.state().hot_cues[0], Some(stored));
    }

    #[test]
    fn cue_while_playing_returns_to_cue_and_pauses() {
        let track = make_track(SR, 4.0, Some(120.0), 0.0, sine(SR, 220.0, 0.3));
        let mut deck = loaded(track);
        deck.handle(DeckCmd::Play);
        run(&mut deck, SR as usize);
        assert!((deck.state().position_secs - 1.0).abs() < 1e-6);
        deck.handle(DeckCmd::Cue);
        let s = deck.state();
        assert!(!s.playing);
        assert!(s.position_secs.abs() < 1e-9);
        run(&mut deck, 1000);
        assert!(
            deck.state().position_secs.abs() < 1e-9,
            "paused deck must not move"
        );
    }

    #[test]
    fn cue_while_paused_snaps_only_within_eighth_beat() {
        let track = make_track(SR, 4.0, Some(120.0), 0.0, sine(SR, 220.0, 0.3));
        let mut deck = loaded(track);
        // 0.05 s is 0.1 beat from 2.0 at 120 BPM, inside the 1/8 beat window.
        seek_secs(&mut deck, 2.05);
        deck.handle(DeckCmd::Cue);
        assert!((deck.state().cue_secs - 2.0).abs() < 1e-9);
        // 0.07 s is 0.14 beat away, outside the window: no snap.
        seek_secs(&mut deck, 2.07);
        deck.handle(DeckCmd::Cue);
        assert!((deck.state().cue_secs - 2.07).abs() < 1e-6);
    }

    #[test]
    fn sync_matches_bpm_with_tempo_fader() {
        let track = make_track(SR, 4.0, Some(120.0), 0.0, sine(SR, 220.0, 0.3));
        let mut deck = loaded(track);
        deck.handle(DeckCmd::Sync {
            bpm: 126.0,
            phase: None,
        });
        let s = deck.state();
        assert!((s.speed - 1.05).abs() < 1e-9, "speed {}", s.speed);
        assert!((s.tempo_range - 0.08).abs() < 1e-6);
        assert!((s.tempo_fader - 0.625).abs() < 1e-6);
    }

    #[test]
    fn sync_picks_double_time_when_it_is_closer() {
        let track = make_track(SR, 4.0, Some(140.0), 0.0, sine(SR, 220.0, 0.3));
        let mut deck = loaded(track);
        deck.handle(DeckCmd::Sync {
            bpm: 70.0,
            phase: None,
        });
        let s = deck.state();
        assert!((s.speed - 1.0).abs() < 1e-9, "speed {}", s.speed);
        assert!(s.tempo_fader.abs() < 1e-6);
    }

    #[test]
    fn sync_phase_shifts_playhead_to_target_phase() {
        let track = make_track(SR, 4.0, Some(120.0), 0.0, sine(SR, 220.0, 0.3));
        let mut deck = loaded(track);
        // 0.1 s at 120 BPM is beat phase 0.2.
        seek_secs(&mut deck, 0.1);
        assert!((deck.beat_phase().unwrap() - 0.2).abs() < 1e-9);
        deck.handle(DeckCmd::Sync {
            bpm: 120.0,
            phase: Some(0.7),
        });
        // Paused, so the phase is exactly where the declicked jump put it.
        let phase = deck.beat_phase().unwrap();
        assert!((phase - 0.7).abs() < 0.01, "phase {phase}");
    }

    #[test]
    fn hot_cue_jump_is_declicked() {
        let track = make_track(SR, 3.0, None, 0.0, sine(SR, 100.0, 0.5));
        let sine_step = max_step(&track);
        let mut deck = loaded(track);
        deck.handle(DeckCmd::SetHotCues([
            Some(0.61234),
            None,
            None,
            None,
            None,
            None,
        ]));
        deck.handle(DeckCmd::Play);
        run(&mut deck, SR as usize);
        let mut out = Vec::new();
        for _ in 0..1000 {
            out.push(deck.process().0);
        }
        deck.handle(DeckCmd::HotCue(0));
        assert!((deck.state().position_secs - 0.61234).abs() < 1e-6);
        for _ in 0..2000 {
            out.push(deck.process().0);
        }
        let worst = out[999..]
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(
            worst <= 1.5 * sine_step,
            "jump step {worst} exceeds 1.5 x sine step {sine_step}"
        );
    }

    #[test]
    fn end_of_track_stops_and_stays_silent() {
        let track = make_track(SR, 0.5, None, 0.0, sine(SR, 100.0, 0.5));
        let mut deck = loaded(track);
        deck.handle(DeckCmd::Play);
        run(&mut deck, 24_000);
        let s = deck.state();
        assert!(!s.playing);
        assert!(s.ended);
        assert!((s.position_secs - 0.5).abs() < 1e-9);
        for _ in 0..1000 {
            assert_eq!(deck.process(), (0.0, 0.0));
        }
    }

    /// Tiny xorshift PRNG so the fuzz run is deterministic.
    struct Prng(u64);

    impl Prng {
        fn next_u64(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }

        fn pick(&mut self, n: u64) -> u64 {
            self.next_u64() % n
        }

        fn unit(&mut self) -> f64 {
            (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
        }

        fn range(&mut self, lo: f64, hi: f64) -> f64 {
            lo + (hi - lo) * self.unit()
        }

        /// Mostly sane values, with NaN and infinities mixed in.
        fn value(&mut self) -> f64 {
            match self.pick(14) {
                0 => f64::NAN,
                1 => f64::INFINITY,
                2 => f64::NEG_INFINITY,
                3 => 0.0,
                _ => self.range(-5.0, 400.0),
            }
        }
    }

    fn random_cmd(p: &mut Prng, a: &Arc<Track>, b: &Arc<Track>, allow_load: bool) -> DeckCmd {
        let v32 = p.value() as f32;
        let v64 = p.value();
        let slot = |p: &mut Prng| {
            if p.pick(2) == 0 {
                None
            } else {
                Some(p.value())
            }
        };
        match p.pick(26) {
            0 if allow_load => DeckCmd::Load(if p.pick(2) == 0 { a.clone() } else { b.clone() }),
            1 if allow_load => DeckCmd::Eject,
            2 => DeckCmd::TogglePlay,
            3 => DeckCmd::Play,
            4 => DeckCmd::Pause,
            5 => DeckCmd::Cue,
            6 => DeckCmd::HotCue(p.pick(HOT_CUES as u64 + 2) as usize),
            7 => DeckCmd::ClearHotCue(p.pick(HOT_CUES as u64 + 2) as usize),
            8 => DeckCmd::SetHotCues(std::array::from_fn(|_| slot(p))),
            9 => DeckCmd::BeatLoop(v32),
            10 => DeckCmd::ExitLoop,
            11 => DeckCmd::LoopHalve,
            12 => DeckCmd::LoopDouble,
            13 => DeckCmd::SetTempoFader(v32),
            14 => DeckCmd::SetTempoRange(TEMPO_RANGES[p.pick(3) as usize]),
            15 => DeckCmd::Nudge(v32),
            16 => DeckCmd::SeekFraction(v64),
            17 => DeckCmd::BeatJump(v32),
            18 => DeckCmd::Sync {
                bpm: v64,
                phase: slot(p),
            },
            19 => DeckCmd::SetBpm(v64),
            20 => DeckCmd::SetGridAnchorHere,
            21 => DeckCmd::SetTempoRange(v32),
            22 => DeckCmd::Play,
            23 => DeckCmd::BeatLoop(p.range(0.0, 8.0) as f32),
            24 => DeckCmd::SetTempoSpeed(v64),
            _ => DeckCmd::Pause,
        }
    }

    #[test]
    fn random_commands_never_produce_non_finite_output() {
        let a = make_track(SR, 3.0, Some(120.0), 0.2, sine(SR, 220.0, 0.3));
        let b = make_track(44_100, 1.5, None, 0.0, noise);
        let mut empty = Deck::new(SR as f32);
        let mut p = Prng(0x9E37_79B9_7F4A_7C15);
        for _ in 0..400 {
            let cmd = random_cmd(&mut p, &a, &b, false);
            empty.handle(cmd);
            for _ in 0..p.pick(40) {
                let (l, r) = empty.process();
                assert!(l.is_finite() && r.is_finite());
            }
            assert_state_finite(&empty.state());
        }
        assert!(!empty.state().loaded);

        let mut deck = loaded(a.clone());
        let mut p = Prng(0xD1B5_4A32_D192_ED03);
        for _ in 0..400 {
            let cmd = random_cmd(&mut p, &a, &b, true);
            deck.handle(cmd);
            for _ in 0..p.pick(40) {
                let (l, r) = deck.process();
                assert!(l.is_finite() && r.is_finite());
            }
            assert_state_finite(&deck.state());
            assert!(deck.effective_bpm().is_none_or(f64::is_finite));
            assert!(deck.beat_phase().is_none_or(|x| (0.0..1.0).contains(&x)));
        }
    }

    #[test]
    fn pause_fades_out_at_the_heard_position_then_freezes() {
        let track = make_track(SR, 3.0, Some(120.0), 0.0, sine(SR, 220.0, 0.3));
        let mut deck = loaded(track);
        deck.handle(DeckCmd::Play);
        run(&mut deck, SR as usize / 2);
        deck.handle(DeckCmd::Pause);
        let s = deck.state();
        assert!(!s.playing, "playing must drop at once");
        // The fade takes DECLICK_SECS; the playhead moves while it fades.
        let mut max_out = 0.0f32;
        for _ in 0..(DECLICK_SECS * SR as f32) as usize + 4 {
            let (l, _) = deck.process();
            max_out = max_out.max(l.abs());
        }
        assert!(max_out > 0.0);
        let frozen = deck.state().position_secs;
        run(&mut deck, 1000);
        assert_eq!(deck.process(), (0.0, 0.0));
        assert_eq!(deck.state().position_secs, frozen);
        assert!(!deck.is_playing());
    }

    #[test]
    fn load_returns_previous_track_and_resets_per_track_state() {
        let a = make_track(SR, 2.0, Some(120.0), 0.0, sine(SR, 220.0, 0.3));
        let b = make_track(SR, 2.0, None, 0.0, noise);
        let mut deck = loaded(a.clone());
        deck.handle(DeckCmd::SetTempoRange(0.16));
        deck.handle(DeckCmd::SetTempoFader(0.25));
        deck.handle(DeckCmd::BeatLoop(2.0));
        deck.handle(DeckCmd::Play);
        let old = deck.handle(DeckCmd::Load(b.clone())).expect("old track");
        assert_eq!(old.id, a.id);
        let s = deck.state();
        assert_eq!(s.track_id, b.id);
        assert!(!s.playing);
        assert!(s.loop_state.is_none());
        assert!(s.hot_cues.iter().all(Option::is_none));
        assert_eq!(s.position_secs, 0.0);
        // Tempo fader and range survive a load.
        assert!((s.tempo_fader - 0.25).abs() < 1e-6);
        assert!((s.tempo_range - 0.16).abs() < 1e-6);
    }

    #[test]
    fn eject_returns_track_and_goes_silent() {
        let a = make_track(SR, 2.0, Some(120.0), 0.0, sine(SR, 220.0, 0.3));
        let mut deck = loaded(a.clone());
        deck.handle(DeckCmd::Play);
        run(&mut deck, 1000);
        let old = deck.handle(DeckCmd::Eject).expect("ejected track");
        assert_eq!(old.id, a.id);
        assert!(!deck.state().loaded);
        assert_eq!(deck.process(), (0.0, 0.0));
        assert!(deck.handle(DeckCmd::Eject).is_none());
    }

    #[test]
    fn beat_jump_moves_playhead_and_active_loop_together() {
        let track = make_track(SR, 12.0, Some(120.0), 0.0, sine(SR, 220.0, 0.3));
        let mut deck = loaded(track);
        seek_secs(&mut deck, 1.3);
        deck.handle(DeckCmd::BeatLoop(4.0));
        deck.handle(DeckCmd::BeatJump(1.0));
        let s = deck.state();
        assert!((s.position_secs - 1.8).abs() < 1e-9, "{}", s.position_secs);
        let lp = s.loop_state.expect("loop moves with the playhead");
        assert!((lp.start_secs - 1.5).abs() < 1e-9);
        assert!((lp.end_secs - 3.5).abs() < 1e-9);
        deck.handle(DeckCmd::BeatJump(-100.0));
        assert_eq!(deck.state().position_secs, 0.0);
    }

    #[test]
    fn loop_halve_and_double_keep_start_and_wrap_when_passed() {
        let track = make_track(SR, 12.0, Some(120.0), 0.0, sine(SR, 220.0, 0.3));
        let mut deck = loaded(track);
        // Without a loop both are ignored.
        deck.handle(DeckCmd::LoopHalve);
        deck.handle(DeckCmd::LoopDouble);
        assert!(deck.state().loop_state.is_none());

        seek_secs(&mut deck, 1.3);
        deck.handle(DeckCmd::BeatLoop(4.0));
        deck.handle(DeckCmd::LoopHalve);
        let lp = deck.state().loop_state.expect("loop");
        assert!((lp.start_secs - 1.0).abs() < 1e-9);
        assert!((lp.end_secs - 2.0).abs() < 1e-9);
        assert!((lp.beats - 2.0).abs() < 1e-6);
        deck.handle(DeckCmd::LoopDouble);
        let lp = deck.state().loop_state.expect("loop");
        assert!((lp.end_secs - 3.0).abs() < 1e-9);

        // Playhead at 2.5 s is past the halved end at 2.0 s: it wraps to 1.5 s.
        seek_secs(&mut deck, 2.5);
        deck.handle(DeckCmd::LoopHalve);
        let s = deck.state();
        assert!((s.position_secs - 1.5).abs() < 1e-9, "{}", s.position_secs);

        deck.handle(DeckCmd::ExitLoop);
        assert!(deck.state().loop_state.is_none());
    }

    #[test]
    fn loop_clamps_to_track_end_and_wraps_there() {
        // 2.5 s track: a 4-beat loop from 2.0 s would run past the end.
        let track = make_track(SR, 2.5, Some(120.0), 0.0, sine(SR, 220.0, 0.3));
        let mut deck = loaded(track);
        seek_secs(&mut deck, 2.0);
        deck.handle(DeckCmd::BeatLoop(4.0));
        let lp = deck.state().loop_state.expect("loop");
        assert!((lp.end_secs - 2.5).abs() < 1e-9);
        deck.handle(DeckCmd::Play);
        for _ in 0..(SR as usize) {
            deck.process();
        }
        let s = deck.state();
        assert!(
            s.playing && !s.ended,
            "a clamped loop never reaches the end"
        );
        assert!((2.0..2.5).contains(&s.position_secs));
    }

    #[test]
    fn tempo_range_change_keeps_speed_and_clamps_fader() {
        let track = make_track(SR, 4.0, Some(120.0), 0.0, sine(SR, 220.0, 0.3));
        let mut deck = loaded(track);
        deck.handle(DeckCmd::SetTempoFader(0.5));
        assert!((deck.state().speed - 1.04).abs() < 1e-9);
        deck.handle(DeckCmd::SetTempoRange(0.16));
        let s = deck.state();
        assert!((s.speed - 1.04).abs() < 1e-9);
        assert!((s.tempo_fader - 0.25).abs() < 1e-6);
        // Values that are not in the table are ignored.
        deck.handle(DeckCmd::SetTempoRange(0.07));
        assert!((deck.state().tempo_range - 0.16).abs() < 1e-6);
        // A range that cannot hold the speed clamps the fader.
        deck.handle(DeckCmd::SetTempoFader(1.0));
        deck.handle(DeckCmd::SetTempoRange(0.08));
        let s = deck.state();
        assert!((s.tempo_fader - 1.0).abs() < 1e-6);
        assert!((s.speed - 1.08).abs() < 1e-9);
        // Fader input is clamped to -1..=1.
        deck.handle(DeckCmd::SetTempoFader(7.0));
        assert!((deck.state().tempo_fader - 1.0).abs() < 1e-6);
    }

    #[test]
    fn sync_widens_range_and_can_choose_half_time() {
        let track = make_track(SR, 4.0, Some(120.0), 0.0, sine(SR, 220.0, 0.3));
        let mut deck = loaded(track);
        // 140 vs 120 is 0.167 away: wider than 0.16, so the 0.50 range is used.
        deck.handle(DeckCmd::Sync {
            bpm: 140.0,
            phase: None,
        });
        let s = deck.state();
        assert!((s.tempo_range - 0.5).abs() < 1e-6);
        assert!((s.speed - 140.0 / 120.0).abs() < 1e-9);
        assert!((s.tempo_fader - (140.0 / 120.0 - 1.0) / 0.5).abs() < 1e-6);

        // 200 is far from 120, but 100 (half time) is closer.
        deck.handle(DeckCmd::Sync {
            bpm: 200.0,
            phase: None,
        });
        let s = deck.state();
        assert!((s.speed - 100.0 / 120.0).abs() < 1e-9, "speed {}", s.speed);
    }

    #[test]
    fn set_tempo_speed_fits_the_range_and_clamps() {
        let mut deck = Deck::new(SR as f32);
        deck.handle(DeckCmd::Load(make_track(SR, 3.0, Some(120.0), 0.0, noise)));
        deck.handle(DeckCmd::SetTempoSpeed(1.05));
        let s = deck.state();
        assert!((s.speed - 1.05).abs() < 1e-9);
        assert!((s.tempo_range - 0.08).abs() < 1e-6);
        assert_eq!(deck.tempo_bpm(), Some(126.0));

        deck.handle(DeckCmd::SetTempoSpeed(1.12));
        assert!((deck.state().tempo_range - 0.16).abs() < 1e-6, "widens");
        assert!((deck.tempo_speed() - 1.12).abs() < 1e-9);

        deck.handle(DeckCmd::SetTempoSpeed(9.0));
        assert!((deck.tempo_speed() - 1.5).abs() < 1e-9, "clamped to +50 %");
        deck.handle(DeckCmd::SetTempoSpeed(0.1));
        assert!((deck.tempo_speed() - 0.5).abs() < 1e-9, "clamped to -50 %");
        deck.handle(DeckCmd::SetTempoSpeed(f64::NAN));
        assert!((deck.tempo_speed() - 0.5).abs() < 1e-9, "NaN ignored");
    }

    #[test]
    fn all_hot_cue_slots_store_and_clear() {
        let mut deck = Deck::new(SR as f32);
        deck.handle(DeckCmd::Load(make_track(SR, 3.0, None, 0.0, noise)));
        for i in 0..HOT_CUES {
            deck.handle(DeckCmd::SeekFraction(i as f64 / 10.0));
            deck.handle(DeckCmd::HotCue(i));
            deck.handle(DeckCmd::Pause);
        }
        let s = deck.state();
        assert!(s.hot_cues.iter().all(Option::is_some), "{:?}", s.hot_cues);
        deck.handle(DeckCmd::ClearHotCue(HOT_CUES - 1));
        assert_eq!(deck.state().hot_cues[HOT_CUES - 1], None);
        assert!(deck.state().hot_cues[HOT_CUES - 2].is_some());
    }

    #[test]
    fn sync_needs_tempo() {
        let track = make_track(SR, 2.0, None, 0.0, sine(SR, 220.0, 0.3));
        let mut deck = loaded(track);
        deck.handle(DeckCmd::Sync {
            bpm: 126.0,
            phase: Some(0.3),
        });
        let s = deck.state();
        assert_eq!(s.speed, 1.0);
        assert!(s.base_bpm.is_none());
    }

    #[test]
    fn nudge_scales_speed_while_held() {
        let track = make_track(SR, 4.0, Some(120.0), 0.0, sine(SR, 220.0, 0.3));
        let mut deck = loaded(track);
        deck.handle(DeckCmd::Nudge(1.0));
        assert!((deck.state().speed - (1.0 + NUDGE_AMOUNT)).abs() < 1e-12);
        deck.handle(DeckCmd::Nudge(-3.0));
        assert!((deck.state().speed - (1.0 - NUDGE_AMOUNT)).abs() < 1e-12);
        deck.handle(DeckCmd::Nudge(f32::NAN));
        assert_eq!(deck.state().speed, 1.0);
    }

    #[test]
    fn set_bpm_is_range_checked_and_overrides_tempo() {
        let track = make_track(SR, 4.0, Some(120.0), 0.0, sine(SR, 220.0, 0.3));
        let mut deck = loaded(track);
        deck.handle(DeckCmd::SetBpm(f64::NAN));
        deck.handle(DeckCmd::SetBpm(500.0));
        deck.handle(DeckCmd::SetBpm(29.0));
        assert_eq!(deck.state().base_bpm, Some(120.0));
        deck.handle(DeckCmd::SetBpm(90.0));
        let s = deck.state();
        assert_eq!(s.base_bpm, Some(90.0));
        assert!((s.bpm.unwrap() - 90.0).abs() < 1e-9);
    }

    #[test]
    fn grid_anchor_here_puts_a_beat_under_the_playhead() {
        let track = make_track(SR, 4.0, Some(120.0), 0.0, sine(SR, 220.0, 0.3));
        let mut deck = loaded(track);
        seek_secs(&mut deck, 1.1);
        deck.handle(DeckCmd::SetGridAnchorHere);
        let s = deck.state();
        assert!((s.first_beat_secs - 1.1).abs() < 1e-9);
        assert!(s.beat_phase.unwrap().abs() < 1e-9);
    }

    #[test]
    fn commands_without_a_track_are_ignored() {
        let mut deck = Deck::new(SR as f32);
        deck.handle(DeckCmd::Play);
        deck.handle(DeckCmd::Cue);
        deck.handle(DeckCmd::HotCue(0));
        deck.handle(DeckCmd::BeatLoop(4.0));
        deck.handle(DeckCmd::SeekFraction(0.5));
        deck.handle(DeckCmd::BeatJump(2.0));
        deck.handle(DeckCmd::Sync {
            bpm: 120.0,
            phase: None,
        });
        deck.handle(DeckCmd::SetGridAnchorHere);
        let s = deck.state();
        assert!(!s.loaded && !s.playing && !s.ended);
        assert!(s.loop_state.is_none());
        assert!(s.hot_cues.iter().all(Option::is_none));
        assert_eq!(s.position_secs, 0.0);
        assert_eq!(deck.process(), (0.0, 0.0));
        assert!(deck.beat_phase().is_none());
        assert!(deck.effective_bpm().is_none());
    }

    #[test]
    fn play_on_an_ended_deck_restarts_from_the_cue_point() {
        let track = make_track(SR, 0.5, None, 0.0, sine(SR, 100.0, 0.5));
        let mut deck = loaded(track);
        deck.handle(DeckCmd::Play);
        run(&mut deck, 24_000);
        assert!(deck.state().ended);
        deck.handle(DeckCmd::Play);
        let s = deck.state();
        assert!(s.playing && !s.ended);
        assert!(s.position_secs.abs() < 1e-9);
    }

    /// Worst sample-to-sample step, as a multiple of the sine's own max step,
    /// for a HotCue jump followed by a second jump `k` frames later.
    fn two_jump_worst_ratio(k: usize, first: f64, second: f64) -> f32 {
        let track = make_track(SR, 3.0, None, 0.0, sine(SR, 100.0, 0.5));
        let sine_step = max_step(&track);
        let mut deck = loaded(track);
        deck.handle(DeckCmd::SetHotCues([
            Some(first),
            Some(second),
            None,
            None,
            None,
            None,
        ]));
        deck.handle(DeckCmd::Play);
        run(&mut deck, SR as usize);
        let mut out = Vec::new();
        for _ in 0..1000 {
            out.push(deck.process().0);
        }
        deck.handle(DeckCmd::HotCue(0));
        for _ in 0..k {
            out.push(deck.process().0);
        }
        deck.handle(DeckCmd::HotCue(1));
        for _ in 0..2000 {
            out.push(deck.process().0);
        }
        let worst = out[999..]
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        worst / sine_step
    }

    #[test]
    fn overlapping_jumps_keep_steps_under_twice_the_sine_step() {
        for k in [1, 64, 128, 191] {
            let ratio = two_jump_worst_ratio(k, 0.61234, 1.23456);
            assert!(ratio < 2.0, "k={k}: step is {ratio} x the sine step");
        }
    }

    #[test]
    fn many_overlapping_jumps_stay_finite() {
        let track = make_track(SR, 3.0, None, 0.0, sine(SR, 100.0, 0.5));
        let mut deck = loaded(track);
        deck.handle(DeckCmd::SetHotCues([
            Some(0.3),
            Some(0.9),
            Some(1.4),
            Some(2.1),
            None,
            None,
        ]));
        deck.handle(DeckCmd::Play);
        for i in 0..40 {
            deck.handle(DeckCmd::HotCue(i % HOT_CUES));
            for _ in 0..(i % 5) {
                let (l, r) = deck.process();
                assert!(l.is_finite() && r.is_finite());
            }
        }
        assert!(deck.is_playing());
    }

    #[test]
    fn read_is_silent_outside_the_guard_band() {
        let track = make_track(SR, 0.1, None, 0.0, |_| 0.5);
        assert_eq!(read(&track, f64::NAN), (0.0, 0.0));
        assert_eq!(read(&track, f64::INFINITY), (0.0, 0.0));
        assert_eq!(read(&track, 1e300), (0.0, 0.0));
        assert_eq!(read(&track, -1e300), (0.0, 0.0));
        assert_eq!(read(&track, -2.5), (0.0, 0.0));
        assert_eq!(read(&track, track.frames() as f64 + 2.5), (0.0, 0.0));
        let (l, _) = read(&track, 100.0);
        assert!((l - 0.5).abs() < 1e-6);
    }

    #[test]
    fn hot_cue_outside_a_loop_exits_it() {
        let track = make_track(SR, 12.0, Some(120.0), 0.0, sine(SR, 220.0, 0.3));
        let mut deck = loaded(track);
        seek_secs(&mut deck, 1.3);
        deck.handle(DeckCmd::BeatLoop(4.0));
        deck.handle(DeckCmd::SetHotCues([
            Some(6.0),
            Some(2.0),
            None,
            None,
            None,
            None,
        ]));
        deck.handle(DeckCmd::HotCue(0));
        assert!(deck.state().loop_state.is_none());
        run(&mut deck, 1000);
        let s = deck.state();
        assert!(
            (s.position_secs - 6.0208).abs() < 0.001,
            "{}",
            s.position_secs
        );
        assert!(s.loop_state.is_none());
    }

    #[test]
    fn hot_cue_inside_a_loop_keeps_it() {
        let track = make_track(SR, 12.0, Some(120.0), 0.0, sine(SR, 220.0, 0.3));
        let mut deck = loaded(track);
        seek_secs(&mut deck, 1.3);
        deck.handle(DeckCmd::BeatLoop(4.0));
        deck.handle(DeckCmd::SetHotCues([
            None,
            Some(2.0),
            None,
            None,
            None,
            None,
        ]));
        deck.handle(DeckCmd::HotCue(1));
        let lp = deck.state().loop_state.expect("loop kept");
        assert!((lp.start_secs - 1.0).abs() < 1e-9);
        assert!((lp.end_secs - 3.0).abs() < 1e-9);
    }

    #[test]
    fn seek_outside_a_loop_exits_it() {
        let track = make_track(SR, 12.0, Some(120.0), 0.0, sine(SR, 220.0, 0.3));
        let mut deck = loaded(track);
        seek_secs(&mut deck, 1.3);
        deck.handle(DeckCmd::BeatLoop(4.0));
        deck.handle(DeckCmd::SeekFraction(0.75));
        let s = deck.state();
        assert!((s.position_secs - 9.0).abs() < 1e-6, "{}", s.position_secs);
        assert!(s.loop_state.is_none());
        deck.handle(DeckCmd::Play);
        run(&mut deck, 1000);
        let s = deck.state();
        assert!(
            (s.position_secs - 9.0208).abs() < 0.001,
            "{}",
            s.position_secs
        );
        assert!(s.loop_state.is_none());
    }

    #[test]
    fn repeat_press_exits_a_loop_clamped_at_the_track_end() {
        let track = make_track(SR, 2.5, Some(120.0), 0.0, sine(SR, 220.0, 0.3));
        let mut deck = loaded(track);
        seek_secs(&mut deck, 2.0);
        deck.handle(DeckCmd::BeatLoop(4.0));
        let lp = deck.state().loop_state.expect("loop");
        assert!((lp.end_secs - 2.5).abs() < 1e-9);
        deck.handle(DeckCmd::BeatLoop(4.0));
        assert!(deck.state().loop_state.is_none());
    }

    #[test]
    fn pause_fading_past_the_loop_end_lands_back_in_the_loop() {
        let track = make_track(SR, 12.0, Some(120.0), 0.0, sine(SR, 220.0, 0.3));
        let mut deck = loaded(track);
        seek_secs(&mut deck, 1.3);
        deck.handle(DeckCmd::BeatLoop(4.0));
        seek_secs(&mut deck, 2.9955);
        deck.handle(DeckCmd::Play);
        // Fade-in done, playhead about 24 frames before the loop end at 3.0 s.
        run(&mut deck, 193);
        deck.handle(DeckCmd::Pause);
        for _ in 0..300 {
            deck.process();
            let p = deck.state().position_secs;
            assert!((1.0..3.0).contains(&p), "position {p} left the loop");
        }
        assert!(!deck.is_playing());
    }

    #[test]
    fn sub_beat_loop_contains_the_playhead_without_a_jump() {
        let track = make_track(SR, 12.0, Some(120.0), 0.0, sine(SR, 220.0, 0.3));
        let mut deck = loaded(track);
        seek_secs(&mut deck, 1.3);
        deck.handle(DeckCmd::BeatLoop(0.25));
        let lp = deck.state().loop_state.expect("loop");
        assert!((lp.start_secs - 1.25).abs() < 1e-9, "{}", lp.start_secs);
        assert!((lp.end_secs - 1.375).abs() < 1e-9, "{}", lp.end_secs);
        deck.handle(DeckCmd::Play);
        let mut prev = deck.state().position_secs;
        for _ in 0..1000 {
            deck.process();
            let p = deck.state().position_secs;
            assert!((1.25..1.375).contains(&p), "position {p} left the loop");
            assert!(
                (p - prev - 1.0 / SR as f64).abs() < 1e-9,
                "playhead jumped from {prev} to {p}"
            );
            prev = p;
        }
    }

    #[test]
    fn sync_phase_is_reached_from_the_very_start_of_the_track() {
        // A freshly loaded deck sits on its first beat, at 0 s. A phase that
        // needs a shift backwards must wrap forward, not clamp at 0.
        let track = make_track(SR, 4.0, Some(120.0), 0.0, sine(SR, 220.0, 0.3));
        let mut deck = loaded(track);
        assert!(deck.state().position_secs.abs() < 1e-9);
        deck.handle(DeckCmd::Sync {
            bpm: 120.0,
            phase: Some(0.6),
        });
        let phase = deck.beat_phase().unwrap();
        assert!((phase - 0.6).abs() < 0.01, "phase {phase}");
    }

    #[test]
    fn sync_on_an_ended_deck_aligns_from_the_cue_and_play_restarts_there() {
        let track = make_track(SR, 1.0, Some(120.0), 0.0, sine(SR, 220.0, 0.3));
        let mut deck = loaded(track);
        deck.handle(DeckCmd::Play);
        run(&mut deck, SR as usize + 100);
        assert!(deck.state().ended);
        // Phase 0.3 at 120 BPM from the cue at 0 s is 0.15 s.
        deck.handle(DeckCmd::Sync {
            bpm: 120.0,
            phase: Some(0.3),
        });
        let s = deck.state();
        assert!(!s.ended, "a sync must not leave the deck ended");
        assert!((s.position_secs - 0.15).abs() < 1e-6, "{}", s.position_secs);
        // Play then continues from there rather than doing nothing.
        deck.handle(DeckCmd::Play);
        run(&mut deck, SR as usize / 4);
        let s = deck.state();
        assert!(s.playing && !s.ended);
        assert!((s.position_secs - 0.4).abs() < 1e-3, "{}", s.position_secs);
    }

    #[test]
    fn sync_inside_a_loop_wraps_around_it_and_keeps_the_loop() {
        let track = make_track(SR, 12.0, Some(120.0), 0.0, sine(SR, 220.0, 0.3));
        let mut deck = loaded(track);
        seek_secs(&mut deck, 1.3);
        deck.handle(DeckCmd::BeatLoop(4.0));
        seek_secs(&mut deck, 1.05);
        deck.handle(DeckCmd::Play);
        // The playhead is at phase 0.1. Phase 0.9 needs a shift of -0.2 beats
        // (-0.1 s), which would leave the loop at 0.95 s. It wraps to 2.95 s.
        deck.handle(DeckCmd::Sync {
            bpm: 120.0,
            phase: Some(0.9),
        });
        let s = deck.state();
        let lp = s.loop_state.expect("the loop survives a sync");
        assert!((lp.start_secs - 1.0).abs() < 1e-9);
        assert!((lp.end_secs - 3.0).abs() < 1e-9);
        assert!((s.position_secs - 2.95).abs() < 1e-6, "{}", s.position_secs);
        let phase = s.beat_phase.unwrap();
        assert!((phase - 0.9).abs() < 1e-6, "phase {phase}");
    }

    #[test]
    fn sync_inside_a_loop_of_fractional_beats_keeps_the_grid_phase() {
        let track = make_track(SR, 12.0, Some(120.0), 0.0, sine(SR, 220.0, 0.3));
        let mut deck = loaded(track);
        seek_secs(&mut deck, 1.3);
        // 1.5 beats from 1.0 s: the loop is 1.0 .. 1.75 s.
        deck.handle(DeckCmd::BeatLoop(1.5));
        seek_secs(&mut deck, 1.7);
        // Phase 0.4 to 0.85: a shift of +0.45 beats lands 1.85 beats into the
        // loop. Wrapping by the loop's 1.5 beats would miss the phase.
        deck.handle(DeckCmd::Sync {
            bpm: 120.0,
            phase: Some(0.85),
        });
        let s = deck.state();
        let lp = s.loop_state.expect("loop kept");
        assert!((lp.start_secs - 1.0).abs() < 1e-9);
        assert!((lp.end_secs - 1.75).abs() < 1e-9);
        let phase = s.beat_phase.unwrap();
        assert!((phase - 0.85).abs() < 1e-6, "phase {phase}");
    }

    #[test]
    fn loop_halve_stops_at_the_shortest_loop() {
        let track = make_track(SR, 12.0, Some(120.0), 0.0, sine(SR, 220.0, 0.5));
        let mut deck = loaded(track);
        seek_secs(&mut deck, 1.3);
        deck.handle(DeckCmd::BeatLoop(1.0));
        for _ in 0..10 {
            deck.handle(DeckCmd::LoopHalve);
        }
        let lp = deck.state().loop_state.expect("loop");
        // Halving stops at 1/32 beat, 750 frames (15.6 ms) at 120 BPM.
        assert!((lp.beats - 1.0 / 32.0).abs() < 1e-6, "beats {}", lp.beats);
        let frames = (lp.end_secs - lp.start_secs) * f64::from(SR);
        assert!((frames - 750.0).abs() < 1e-3, "loop is {frames} frames");

        // At that length the declick crossfade at each wrap fits inside the
        // loop, so the loop keeps its level instead of fading to a buzz.
        deck.handle(DeckCmd::Play);
        run(&mut deck, 1_000);
        let mut sum_sq = 0.0f64;
        for _ in 0..4_800 {
            let (l, _) = deck.process();
            sum_sq += f64::from(l) * f64::from(l);
        }
        let rms = (sum_sq / 4_800.0).sqrt();
        assert!(rms > 0.25, "a 1/32-beat loop buzzes quietly: rms {rms}");
    }

    #[test]
    fn beat_jump_keeps_the_loop_length_at_a_track_bound() {
        let track = make_track(SR, 12.0, Some(120.0), 0.0, sine(SR, 220.0, 0.3));
        let mut deck = loaded(track);
        seek_secs(&mut deck, 9.0);
        deck.handle(DeckCmd::BeatLoop(4.0));
        // The jump would carry the 2 s loop past the track end. It keeps its
        // length and its label, and it still holds the playhead.
        deck.handle(DeckCmd::BeatJump(4.0));
        let s = deck.state();
        let lp = s.loop_state.expect("loop kept");
        assert!((lp.end_secs - lp.start_secs - 2.0).abs() < 1e-9, "{lp:?}");
        assert_eq!(lp.beats, 4.0);
        assert!((s.position_secs - 11.0).abs() < 1e-9);
        assert!(lp.start_secs <= s.position_secs && s.position_secs < lp.end_secs);
        // A jump back that fits moves the loop with the playhead.
        deck.handle(DeckCmd::BeatJump(-8.0));
        let lp = deck.state().loop_state.expect("loop kept");
        assert!((lp.start_secs - 6.0).abs() < 1e-9);
        assert!((lp.end_secs - 8.0).abs() < 1e-9);

        // A loop already clamped at the track end keeps its (shorter) length.
        seek_secs(&mut deck, 10.6);
        deck.handle(DeckCmd::BeatLoop(4.0));
        let lp = deck.state().loop_state.expect("loop");
        assert!((lp.end_secs - 12.0).abs() < 1e-9);
        deck.handle(DeckCmd::BeatJump(4.0));
        let lp = deck.state().loop_state.expect("loop kept");
        assert!((lp.end_secs - lp.start_secs - 1.5).abs() < 1e-9, "{lp:?}");
        assert_eq!(lp.beats, 4.0);
    }
}

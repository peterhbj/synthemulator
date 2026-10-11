//! Two decks into a DJ mixer: per-channel EQ, filter and fader, crossfader,
//! automatic transitions, master filter, echo and limiter.

use std::sync::Arc;

use crate::deck::{Deck, DeckCmd, DeckState};
use crate::dsp::{
    crossfader_gains, Band, CrossfaderCurve, DjFilter, Echo, Limiter, Smoother, ThreeBandEq,
    EQ_MAX_GAIN,
};
use crate::track::Track;

pub const DECKS: usize = 2;

#[derive(Clone, Debug)]
pub enum RemixCmd {
    Deck(usize, DeckCmd),
    /// Linear gain 0..=EQ_MAX_GAIN (1 = flat).
    SetEq {
        deck: usize,
        band: Band,
        gain: f32,
    },
    SetKill {
        deck: usize,
        band: Band,
        on: bool,
    },
    /// -1..=1, see `DjFilter`.
    SetFilter {
        deck: usize,
        pos: f32,
    },
    /// Channel fader 0..=1.
    SetVolume {
        deck: usize,
        gain: f32,
    },
    /// 0 = deck A, 1 = deck B.
    SetCrossfader(f32),
    SetCrossfaderCurve(CrossfaderCurve),
    /// Blend to deck `to` over `beats` beats of the playing deck (or
    /// `fallback_secs` without tempo). Starts `to` if paused, syncing tempo
    /// and phase first when both decks have tempo, and swaps the bass at the
    /// midpoint so two kick drums never stack.
    AutoTransition {
        to: usize,
        beats: f32,
        fallback_secs: f32,
    },
    CancelAutoTransition,
    /// Sync `deck` to the other deck (tempo and phase).
    SyncDeck(usize),
    /// -1..=1 on the master bus.
    SetMasterFilter(f32),
    /// Echo send 0..=1, feedback 0..=0.95, time in beats of the master deck
    /// (120 BPM without tempo).
    SetEcho {
        mix: f32,
        feedback: f32,
        beats: f32,
    },
    /// Master gain 0..=1.5.
    SetMasterGain(f32),
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AutoTransitionState {
    pub to: usize,
    /// 0..=1
    pub progress: f32,
}

/// `Copy` view of the mixer for the UI.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RemixState {
    pub decks: [DeckState; DECKS],
    /// Post-fader peak per channel, 0..1+, decaying.
    pub levels: [f32; DECKS],
    pub master_level: f32,
    pub limiter_reduction: f32,
    pub eq: [[f32; 3]; DECKS],
    pub kills: [[bool; 3]; DECKS],
    pub filter: [f32; DECKS],
    pub volume: [f32; DECKS],
    pub crossfader: f32,
    pub curve: CrossfaderCurve,
    pub auto: Option<AutoTransitionState>,
    /// Deck currently carrying the mix (playing and loudest), if any.
    pub master_deck: Option<usize>,
    pub master_filter: f32,
    pub echo_mix: f32,
    pub echo_feedback: f32,
    pub echo_beats: f32,
    pub master_gain: f32,
}

/// Crossfader position at start: centre, both decks at -3 dB.
const DEFAULT_CROSSFADER: f32 = 0.5;
/// Channel fader smoothing time constant.
const FADER_SMOOTH_SECS: f32 = 0.015;
/// Crossfader gain smoothing time constant.
const CROSSFADE_SMOOTH_SECS: f32 = 0.005;
/// Master gain smoothing time constant.
const MASTER_SMOOTH_SECS: f32 = 0.020;
/// Peak meters fall by 1/e in this many seconds.
const METER_DECAY_SECS: f32 = 0.3;
/// Longest echo delay the buffer holds, and the shortest the engine asks for.
const ECHO_MAX_SECS: f32 = 4.0;
const ECHO_MIN_SECS: f32 = 0.01;
const ECHO_DEFAULT_BEATS: f32 = 0.75;
const ECHO_DEFAULT_FEEDBACK: f32 = 0.4;
const ECHO_MIN_BEATS: f32 = 1.0 / 16.0;
const ECHO_MAX_BEATS: f32 = 4.0;
const ECHO_MAX_FEEDBACK: f32 = 0.95;
const MASTER_GAIN_MAX: f32 = 1.5;
/// Tempo used by the echo when the master deck has none.
const FALLBACK_BPM: f64 = 120.0;
/// The echo time is rechecked every this many frames.
const ECHO_CHECK_INTERVAL: u32 = 64;
/// Echo time changes of at most this many seconds are ignored.
const ECHO_RETUNE_SECS: f32 = 0.001;
/// Transition length bounds, seconds. The fallback has its own floor; the
/// beat-based length is only bounded so the progress step stays finite.
const TRANSITION_MIN_SECS: f64 = 0.01;
const TRANSITION_FALLBACK_MIN_SECS: f64 = 0.5;
const TRANSITION_MAX_SECS: f64 = 120.0;
const TRANSITION_MIN_BEATS: f32 = 1.0 / 16.0;
const TRANSITION_MAX_BEATS: f32 = 64.0;
/// Master-deck scores closer than this count as a tie (lower index wins).
const MASTER_TIE_EPS: f32 = 1e-6;

/// Clamp `v` to `lo..=hi`, or `None` for NaN (the caller keeps its old value).
fn clamp_value(v: f32, lo: f32, hi: f32) -> Option<f32> {
    if v.is_nan() {
        None
    } else {
        Some(v.clamp(lo, hi))
    }
}

/// Echo delay for `beats` at `bpm`, clamped to the echo's range.
fn echo_time_secs(beats: f32, bpm: f64) -> f32 {
    let secs = (f64::from(beats) * 60.0 / bpm) as f32;
    secs.clamp(ECHO_MIN_SECS, ECHO_MAX_SECS)
}

/// One deck's path into the mixer: deck, EQ, filter, fader and crossfader gain.
struct Channel {
    deck: Deck,
    eq: ThreeBandEq,
    filter: DjFilter,
    /// Channel fader, gliding toward `volume`.
    fader: Smoother,
    /// Crossfader gain, gliding toward `crossfader_gains`.
    crossfade: Smoother,
    /// User channel fader 0..=1.
    volume: f32,
    /// User EQ gain per band (the low band is scaled by `bass_mult` in the EQ).
    eq_gain: [f32; 3],
    kills: [bool; 3],
    /// Multiplier on the low EQ gain: 1 normally, 0 or 1 while a transition swaps the bass.
    bass_mult: f32,
    /// User filter position -1..=1.
    filter_pos: f32,
    /// Post-fader peak, decaying.
    level: f32,
}

impl Channel {
    fn new(sample_rate: f32, crossfade_gain: f32) -> Self {
        Self {
            deck: Deck::new(sample_rate),
            eq: ThreeBandEq::new(sample_rate),
            filter: DjFilter::new(sample_rate),
            fader: Smoother::new(sample_rate, FADER_SMOOTH_SECS, 1.0),
            crossfade: Smoother::new(sample_rate, CROSSFADE_SMOOTH_SECS, crossfade_gain),
            volume: 1.0,
            eq_gain: [1.0; 3],
            kills: [false; 3],
            bass_mult: 1.0,
            filter_pos: 0.0,
            level: 0.0,
        }
    }

    /// Push the effective (multiplier-aware) gain of `band` into the EQ.
    fn push_eq(&mut self, band: Band) {
        let mut gain = self.eq_gain[band.index()];
        if band == Band::Low {
            gain *= self.bass_mult;
        }
        self.eq.set_gain(band, gain);
    }

    fn set_bass(&mut self, mult: f32) {
        self.bass_mult = mult;
        self.push_eq(Band::Low);
    }
}

/// A running automatic transition.
#[derive(Clone, Copy, Debug)]
struct Transition {
    from: usize,
    to: usize,
    /// Crossfader position when the transition started, and where it ends.
    start: f32,
    end: f32,
    /// Output frames run so far, and the frames the transition lasts. Progress
    /// is their ratio; counting whole frames makes the end land exactly.
    elapsed: f64,
    total: f64,
    /// Past the midpoint: the bass has swapped to the incoming deck.
    second_half: bool,
}

pub struct RemixEngine {
    /// Engine sample rate (sanitised in `new`).
    sample_rate: f32,
    channels: [Channel; DECKS],
    /// Crossfader position 0..=1. Moved by automatic transitions.
    crossfader: f32,
    curve: CrossfaderCurve,
    /// Crossfader gain targets must be recomputed before the next frame.
    crossfade_dirty: bool,
    transition: Option<Transition>,
    master_filter: DjFilter,
    master_filter_pos: f32,
    echo: Echo,
    echo_mix: f32,
    echo_feedback: f32,
    echo_beats: f32,
    /// Delay currently set on `echo`, seconds.
    echo_secs: f32,
    echo_countdown: u32,
    /// Its target is the user master gain.
    master_gain: Smoother,
    limiter: Limiter,
    master_level: f32,
    /// Per-frame multiplier of the peak meters.
    meter_decay: f32,
}

impl RemixEngine {
    pub fn new(sample_rate: f32) -> Self {
        let fs = if sample_rate.is_finite() && sample_rate >= 1_000.0 {
            sample_rate
        } else {
            48_000.0
        };
        let (gain_a, gain_b) = crossfader_gains(DEFAULT_CROSSFADER, CrossfaderCurve::Smooth);
        let mut echo = Echo::new(fs, ECHO_MAX_SECS);
        let echo_secs = echo_time_secs(ECHO_DEFAULT_BEATS, FALLBACK_BPM);
        echo.set_time_secs(echo_secs);
        echo.set_feedback(ECHO_DEFAULT_FEEDBACK);
        echo.set_mix(0.0);
        // The setters glide; clear() snaps every smoother to the targets set above.
        echo.clear();
        Self {
            sample_rate: fs,
            channels: [Channel::new(fs, gain_a), Channel::new(fs, gain_b)],
            crossfader: DEFAULT_CROSSFADER,
            curve: CrossfaderCurve::Smooth,
            crossfade_dirty: false,
            transition: None,
            master_filter: DjFilter::new(fs),
            master_filter_pos: 0.0,
            echo,
            echo_mix: 0.0,
            echo_feedback: ECHO_DEFAULT_FEEDBACK,
            echo_beats: ECHO_DEFAULT_BEATS,
            echo_secs,
            echo_countdown: 0,
            master_gain: Smoother::new(fs, MASTER_SMOOTH_SECS, 1.0),
            limiter: Limiter::new(fs),
            master_level: 0.0,
            meter_decay: (-1.0 / (METER_DECAY_SECS * fs)).exp(),
        }
    }

    /// Apply a command. Returns a replaced track so the caller frees it off
    /// the audio thread.
    pub fn handle(&mut self, cmd: RemixCmd) -> Option<Arc<Track>> {
        match cmd {
            RemixCmd::Deck(deck, cmd) => {
                return match self.channels.get_mut(deck) {
                    Some(ch) => ch.deck.handle(cmd),
                    // Not a deck: hand a loaded track back so it is freed off the audio thread.
                    None => match cmd {
                        DeckCmd::Load(track) => Some(track),
                        _ => None,
                    },
                };
            }
            RemixCmd::SetEq { deck, band, gain } => {
                if let (Some(ch), Some(g)) = (
                    self.channels.get_mut(deck),
                    clamp_value(gain, 0.0, EQ_MAX_GAIN),
                ) {
                    ch.eq_gain[band.index()] = g;
                    ch.push_eq(band);
                }
            }
            RemixCmd::SetKill { deck, band, on } => {
                if let Some(ch) = self.channels.get_mut(deck) {
                    ch.kills[band.index()] = on;
                    ch.eq.set_kill(band, on);
                }
            }
            RemixCmd::SetFilter { deck, pos } => {
                if let (Some(ch), Some(p)) =
                    (self.channels.get_mut(deck), clamp_value(pos, -1.0, 1.0))
                {
                    ch.filter_pos = p;
                    ch.filter.set_position(p);
                }
            }
            RemixCmd::SetVolume { deck, gain } => {
                if let (Some(ch), Some(v)) =
                    (self.channels.get_mut(deck), clamp_value(gain, 0.0, 1.0))
                {
                    ch.volume = v;
                    ch.fader.set_target(v);
                }
            }
            RemixCmd::SetCrossfader(pos) => {
                if let Some(p) = clamp_value(pos, 0.0, 1.0) {
                    self.cancel_transition();
                    self.crossfader = p;
                    self.crossfade_dirty = true;
                }
            }
            RemixCmd::SetCrossfaderCurve(curve) => {
                self.curve = curve;
                self.crossfade_dirty = true;
            }
            RemixCmd::AutoTransition {
                to,
                beats,
                fallback_secs,
            } => {
                self.start_transition(to, beats, fallback_secs);
            }
            RemixCmd::CancelAutoTransition => self.cancel_transition(),
            RemixCmd::SyncDeck(deck) => self.sync_deck(deck),
            RemixCmd::SetMasterFilter(pos) => {
                if let Some(p) = clamp_value(pos, -1.0, 1.0) {
                    self.master_filter_pos = p;
                    self.master_filter.set_position(p);
                }
            }
            RemixCmd::SetEcho {
                mix,
                feedback,
                beats,
            } => {
                if let Some(m) = clamp_value(mix, 0.0, 1.0) {
                    self.echo_mix = m;
                    self.echo.set_mix(m);
                }
                if let Some(f) = clamp_value(feedback, 0.0, ECHO_MAX_FEEDBACK) {
                    self.echo_feedback = f;
                    self.echo.set_feedback(f);
                }
                if let Some(b) = clamp_value(beats, ECHO_MIN_BEATS, ECHO_MAX_BEATS) {
                    self.echo_beats = b;
                }
            }
            RemixCmd::SetMasterGain(gain) => {
                if let Some(g) = clamp_value(gain, 0.0, MASTER_GAIN_MAX) {
                    self.master_gain.set_target(g);
                }
            }
        }
        None
    }

    /// Next stereo master frame.
    #[inline]
    pub fn process(&mut self) -> (f32, f32) {
        self.advance_transition();
        if self.crossfade_dirty {
            self.update_crossfade_targets();
        }
        self.echo_countdown += 1;
        if self.echo_countdown >= ECHO_CHECK_INTERVAL {
            self.echo_countdown = 0;
            self.update_echo_time();
        }

        let decay = self.meter_decay;
        let mut bus_l = 0.0f32;
        let mut bus_r = 0.0f32;
        for ch in self.channels.iter_mut() {
            let (l, r) = ch.deck.process();
            let (l, r) = ch.eq.process(l, r);
            let (l, r) = ch.filter.process(l, r);
            let gain = ch.fader.next() * ch.crossfade.next();
            let (l, r) = (l * gain, r * gain);
            ch.level = (ch.level * decay).max(l.abs()).max(r.abs());
            bus_l += l;
            bus_r += r;
        }

        let (l, r) = self.master_filter.process(bus_l, bus_r);
        let (l, r) = self.echo.process(l, r);
        let master = self.master_gain.next();
        let (l, r) = self.limiter.process(l * master, r * master);
        self.master_level = (self.master_level * decay).max(l.abs()).max(r.abs());
        (l, r)
    }

    pub fn state(&self) -> RemixState {
        RemixState {
            decks: std::array::from_fn(|d| self.channels[d].deck.state()),
            levels: std::array::from_fn(|d| self.channels[d].level),
            master_level: self.master_level,
            limiter_reduction: self.limiter.reduction(),
            eq: std::array::from_fn(|d| self.channels[d].eq_gain),
            kills: std::array::from_fn(|d| self.channels[d].kills),
            filter: std::array::from_fn(|d| self.channels[d].filter_pos),
            volume: std::array::from_fn(|d| self.channels[d].volume),
            crossfader: self.crossfader,
            curve: self.curve,
            auto: self.transition.map(|t| AutoTransitionState {
                to: t.to,
                progress: (t.elapsed / t.total) as f32,
            }),
            master_deck: self.master_deck(),
            master_filter: self.master_filter_pos,
            echo_mix: self.echo_mix,
            echo_feedback: self.echo_feedback,
            echo_beats: self.echo_beats,
            master_gain: self.master_gain.target(),
        }
    }
}

impl RemixEngine {
    /// Deck carrying the mix: the playing deck with the largest
    /// `volume * crossfader gain`, ties to the lower index.
    fn master_deck(&self) -> Option<usize> {
        let (ga, gb) = crossfader_gains(self.crossfader, self.curve);
        let gains = [ga, gb];
        let mut best: Option<(usize, f32)> = None;
        for (d, ch) in self.channels.iter().enumerate() {
            if !ch.deck.is_playing() {
                continue;
            }
            let score = ch.volume * gains[d];
            let better = match best {
                Some((_, s)) => score > s + MASTER_TIE_EPS,
                None => true,
            };
            if better {
                best = Some((d, score));
            }
        }
        best.map(|(d, _)| d)
    }

    /// Recompute the crossfader gain targets from the current position.
    fn update_crossfade_targets(&mut self) {
        let (ga, gb) = crossfader_gains(self.crossfader, self.curve);
        for (ch, gain) in self.channels.iter_mut().zip([ga, gb]) {
            ch.crossfade.set_target(gain);
        }
        self.crossfade_dirty = false;
    }

    /// Retune the echo to the master deck's tempo, if the time moved.
    fn update_echo_time(&mut self) {
        let bpm = self
            .master_deck()
            .and_then(|d| self.channels[d].deck.effective_bpm())
            .filter(|b| b.is_finite() && *b > 0.0)
            .unwrap_or(FALLBACK_BPM);
        let secs = echo_time_secs(self.echo_beats, bpm);
        if (secs - self.echo_secs).abs() > ECHO_RETUNE_SECS {
            self.echo_secs = secs;
            self.echo.set_time_secs(secs);
        }
    }

    /// Beat phase of `deck` for a sync, when it is playing and has tempo.
    fn phase_of(&self, deck: usize) -> Option<f64> {
        let d = &self.channels[deck].deck;
        if d.is_playing() {
            d.beat_phase()
        } else {
            None
        }
    }

    fn sync_deck(&mut self, deck: usize) {
        if deck >= DECKS {
            return;
        }
        let other = 1 - deck;
        let Some(bpm) = self.channels[other].deck.effective_bpm() else {
            return;
        };
        let phase = self.phase_of(other);
        self.channels[deck]
            .deck
            .handle(DeckCmd::Sync { bpm, phase });
    }

    fn start_transition(&mut self, to: usize, beats: f32, fallback_secs: f32) {
        if to >= DECKS || !beats.is_finite() || !fallback_secs.is_finite() {
            return;
        }
        if !self.channels[to].deck.state().loaded {
            return;
        }
        let from = 1 - to;
        // A new transition replaces a running one, starting where the mix is now.
        self.cancel_transition();

        let from_bpm = self.channels[from]
            .deck
            .effective_bpm()
            .filter(|b| b.is_finite() && *b > 0.0);
        let to_has_tempo = self.channels[to].deck.state().base_bpm.is_some();
        if !self.channels[to].deck.is_playing() {
            if let (Some(bpm), true) = (from_bpm, to_has_tempo) {
                let phase = self.phase_of(from);
                self.channels[to].deck.handle(DeckCmd::Sync { bpm, phase });
            }
            self.channels[to].deck.handle(DeckCmd::Play);
        }

        let secs = match from_bpm {
            Some(bpm) if self.channels[from].deck.is_playing() => {
                let beats = beats.clamp(TRANSITION_MIN_BEATS, TRANSITION_MAX_BEATS);
                (f64::from(beats) * 60.0 / bpm).clamp(TRANSITION_MIN_SECS, TRANSITION_MAX_SECS)
            }
            _ => f64::from(fallback_secs).clamp(TRANSITION_FALLBACK_MIN_SECS, TRANSITION_MAX_SECS),
        };
        // Bass is off on the incoming deck until the midpoint.
        self.channels[to].set_bass(0.0);
        self.transition = Some(Transition {
            from,
            to,
            start: self.crossfader,
            end: if to == 0 { 0.0 } else { 1.0 },
            elapsed: 0.0,
            total: secs * f64::from(self.sample_rate),
            second_half: false,
        });
        self.crossfade_dirty = true;
    }

    /// Stop a running transition where it is; the bass multipliers go back to 1.
    fn cancel_transition(&mut self) {
        if let Some(t) = self.transition.take() {
            self.channels[t.to].set_bass(1.0);
            self.channels[t.from].set_bass(1.0);
        }
    }

    /// Advance the transition by one frame, moving the crossfader and swapping the bass at the midpoint.
    fn advance_transition(&mut self) {
        let Some(mut t) = self.transition else {
            return;
        };
        t.elapsed += 1.0;
        if t.elapsed >= t.total {
            self.transition = None;
            self.crossfader = t.end;
            self.channels[t.to].set_bass(1.0);
            self.channels[t.from].set_bass(1.0);
            self.crossfade_dirty = true;
            return;
        }
        let progress = t.elapsed / t.total;
        self.crossfader = t.start + (t.end - t.start) * progress as f32;
        self.crossfade_dirty = true;
        if !t.second_half && progress >= 0.5 {
            t.second_half = true;
            self.channels[t.to].set_bass(1.0);
            self.channels[t.from].set_bass(0.0);
        }
        self.transition = Some(t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::Analysis;
    use crate::deck::{HOT_CUES, TEMPO_RANGES};
    use crate::dsp::LIMITER_CEILING;
    use std::f64::consts::TAU;

    const SR: u32 = 48_000;

    fn analysis(bpm: Option<f64>) -> Analysis {
        Analysis {
            bpm,
            first_beat_secs: 0.0,
            confidence: 1.0,
        }
    }

    /// Stereo track of `secs` seconds; `gen(i)` is frame `i` on both channels.
    fn track(secs: f64, bpm: Option<f64>, gen: impl Fn(usize) -> f32) -> Arc<Track> {
        let frames = (secs * f64::from(SR)).round() as usize;
        let mut samples = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            let v = gen(i);
            samples.push(v);
            samples.push(v);
        }
        Arc::new(Track::from_samples("test", samples, SR, analysis(bpm)))
    }

    fn sine(hz: f64, amp: f32) -> impl Fn(usize) -> f32 {
        move |i| (f64::from(amp) * (TAU * hz * i as f64 / f64::from(SR)).sin()) as f32
    }

    fn load_play(eng: &mut RemixEngine, deck: usize, track: Arc<Track>) {
        eng.handle(RemixCmd::Deck(deck, DeckCmd::Load(track)));
        eng.handle(RemixCmd::Deck(deck, DeckCmd::Play));
    }

    fn render(eng: &mut RemixEngine, frames: usize) -> Vec<(f32, f32)> {
        (0..frames).map(|_| eng.process()).collect()
    }

    /// Amplitude of the `hz` component of the left channel, by correlation.
    /// Exact when the window holds a whole number of cycles (use whole seconds).
    fn amp_at(frames: &[(f32, f32)], hz: f64) -> f64 {
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (n, (l, _)) in frames.iter().enumerate() {
            let ph = TAU * hz * n as f64 / f64::from(SR);
            re += f64::from(*l) * ph.cos();
            im += f64::from(*l) * ph.sin();
        }
        2.0 * re.hypot(im) / frames.len() as f64
    }

    fn db(amp: f64) -> f64 {
        20.0 * amp.max(1e-12).log10()
    }

    /// Current low-band multiplier of each deck.
    fn bass_mults(eng: &RemixEngine) -> [f32; DECKS] {
        std::array::from_fn(|d| eng.channels[d].bass_mult)
    }

    /// Index and value of the largest absolute left sample in `range`.
    fn peak_in(frames: &[(f32, f32)], range: std::ops::Range<usize>) -> (usize, f32) {
        range
            .map(|i| (i, frames[i].0))
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .unwrap()
    }

    fn assert_state_finite(s: &RemixState) {
        for d in &s.decks {
            assert!(d.position_secs.is_finite());
            assert!(d.duration_secs.is_finite());
            assert!(d.cue_secs.is_finite());
            assert!(d.speed.is_finite());
            assert!(d.tempo_fader.is_finite());
            assert!(d.tempo_range.is_finite());
            assert!(d.first_beat_secs.is_finite());
            assert!(d.bpm.is_none_or(f64::is_finite));
            assert!(d.beat_phase.is_none_or(f64::is_finite));
            assert!(d.hot_cues.iter().flatten().all(|c| c.is_finite()));
        }
        assert!(s.levels.iter().all(|v| v.is_finite() && *v >= 0.0));
        assert!(s.master_level.is_finite() && s.master_level >= 0.0);
        assert!(s.limiter_reduction.is_finite());
        assert!(s
            .eq
            .iter()
            .flatten()
            .all(|g| g.is_finite() && (0.0..=EQ_MAX_GAIN).contains(g)));
        assert!(s
            .filter
            .iter()
            .all(|f| f.is_finite() && (-1.0..=1.0).contains(f)));
        assert!(s
            .volume
            .iter()
            .all(|v| v.is_finite() && (0.0..=1.0).contains(v)));
        assert!(s.crossfader.is_finite() && (0.0..=1.0).contains(&s.crossfader));
        assert!(s.master_filter.is_finite() && (-1.0..=1.0).contains(&s.master_filter));
        assert!(s.echo_mix.is_finite() && (0.0..=1.0).contains(&s.echo_mix));
        assert!(
            s.echo_feedback.is_finite() && (0.0..=ECHO_MAX_FEEDBACK).contains(&s.echo_feedback)
        );
        assert!(
            s.echo_beats.is_finite() && (ECHO_MIN_BEATS..=ECHO_MAX_BEATS).contains(&s.echo_beats)
        );
        assert!(s.master_gain.is_finite() && (0.0..=MASTER_GAIN_MAX).contains(&s.master_gain));
        if let Some(a) = s.auto {
            assert!((0.0..=1.0).contains(&a.progress));
        }
    }

    #[test]
    fn defaults_match_the_spec() {
        let eng = RemixEngine::new(SR as f32);
        let s = eng.state();
        assert_eq!(s.volume, [1.0; DECKS]);
        assert_eq!(s.crossfader, 0.5);
        assert_eq!(s.curve, CrossfaderCurve::Smooth);
        assert_eq!(s.eq, [[1.0; 3]; DECKS]);
        assert_eq!(s.kills, [[false; 3]; DECKS]);
        assert_eq!(s.filter, [0.0; DECKS]);
        assert_eq!(s.master_filter, 0.0);
        assert_eq!(s.echo_mix, 0.0);
        assert_eq!(s.echo_feedback, 0.4);
        assert_eq!(s.echo_beats, 0.75);
        assert_eq!(s.master_gain, 1.0);
        assert_eq!(s.limiter_reduction, 1.0);
        assert_eq!(s.master_deck, None);
        assert_eq!(s.auto, None);
        assert_eq!(s.levels, [0.0; DECKS]);
    }

    #[test]
    fn silence_when_nothing_is_loaded() {
        let mut eng = RemixEngine::new(SR as f32);
        for (l, r) in render(&mut eng, 1_000) {
            assert_eq!((l, r), (0.0, 0.0));
        }
        assert_eq!(eng.state().master_level, 0.0);
    }

    #[test]
    fn non_finite_sample_rate_falls_back_to_48k() {
        let mut eng = RemixEngine::new(f32::NAN);
        load_play(&mut eng, 0, track(1.0, Some(120.0), sine(440.0, 0.5)));
        let out = render(&mut eng, 4_800);
        assert!(out.iter().all(|(l, r)| l.is_finite() && r.is_finite()));
        assert!(out.iter().any(|(l, _)| l.abs() > 0.1));
    }

    #[test]
    fn crossfader_ends_pass_only_their_own_deck() {
        // (crossfader, deck A tone, deck B tone): at 0 only A is heard, at 1 only B.
        for (pos, a_hz, b_hz) in [(0.0f32, 440.0, 1000.0), (1.0, 440.0, 1000.0)] {
            let mut eng = RemixEngine::new(SR as f32);
            load_play(&mut eng, 0, track(3.0, Some(120.0), sine(a_hz, 0.5)));
            load_play(&mut eng, 1, track(3.0, Some(120.0), sine(b_hz, 0.5)));
            eng.handle(RemixCmd::SetCrossfader(pos));
            render(&mut eng, SR as usize / 2); // let the 5 ms gain smoother settle
            let out = render(&mut eng, SR as usize);
            let (heard, silent) = if pos == 0.0 {
                (a_hz, b_hz)
            } else {
                (b_hz, a_hz)
            };
            let heard_amp = amp_at(&out, heard);
            let silent_amp = amp_at(&out, silent);
            assert!(heard_amp > 0.45, "pos {pos}: heard {heard_amp}");
            assert!(
                db(silent_amp) < db(heard_amp) - 60.0,
                "pos {pos}: other deck at {} dB",
                db(silent_amp) - db(heard_amp)
            );
        }
    }

    #[test]
    fn auto_transition_starts_syncs_and_swaps_bass_at_the_midpoint() {
        let mut eng = RemixEngine::new(SR as f32);
        // Deck A: 120 BPM, a 1 kHz tone with no bass in it.
        load_play(&mut eng, 0, track(12.0, Some(120.0), sine(1000.0, 0.5)));
        // Deck B: 126 BPM. A 63 Hz tone at 120/126 speed is heard at 60 Hz.
        eng.handle(RemixCmd::Deck(
            1,
            DeckCmd::Load(track(12.0, Some(126.0), sine(63.0, 0.5))),
        ));
        render(&mut eng, SR as usize / 4);

        eng.handle(RemixCmd::AutoTransition {
            to: 1,
            beats: 8.0,
            fallback_secs: 1.0,
        });
        let s = eng.state();
        assert!(s.decks[1].playing, "B should start playing");
        assert!(
            (s.decks[1].speed - 120.0 / 126.0).abs() < 1e-9,
            "speed {}",
            s.decks[1].speed
        );
        assert_eq!(
            s.auto,
            Some(AutoTransitionState {
                to: 1,
                progress: 0.0
            })
        );
        assert_eq!(s.decks[0].bpm.map(|b| b.round()), Some(120.0));

        // The transition runs 0.0 .. 4.0 s from the handle, with the midpoint at 2.0 s.
        // Let the EQ settle, measure 0.25 .. 1.25 s (first half), skip to 2.25 s, measure
        // 2.25 .. 3.25 s (second half).
        render(&mut eng, SR as usize / 4);
        let before = render(&mut eng, SR as usize);
        render(&mut eng, SR as usize);
        let after = render(&mut eng, SR as usize);
        let b60 = amp_at(&before, 60.0);
        let a60 = amp_at(&after, 60.0);
        assert!(
            b60 < 0.01,
            "B's bass leaked before the midpoint: amplitude {b60}"
        );
        assert!(
            a60 > 0.2,
            "B's bass missing after the midpoint: amplitude {a60}"
        );

        // At 3.25 s: progress 0.8125, and the crossfader runs from 0.5 (its default) to 1.0.
        let mid = eng.state();
        let auto = mid.auto.expect("transition still running");
        assert!(
            (auto.progress - 0.8125).abs() < 1e-4,
            "progress {}",
            auto.progress
        );
        assert!(
            (mid.crossfader - 0.90625).abs() < 1e-4,
            "crossfader {}",
            mid.crossfader
        );
        assert_eq!(mid.eq[1][0], 1.0, "user low EQ untouched by the swap");

        // Past 4.0 s the transition is over.
        render(&mut eng, SR as usize);
        let end = eng.state();
        assert_eq!(end.auto, None);
        assert_eq!(end.crossfader, 1.0);
        assert_eq!(bass_mults(&eng), [1.0; DECKS]);
        assert!(end.decks[1].playing);
    }

    #[test]
    fn auto_transition_back_to_deck_a_ends_at_zero() {
        let mut eng = RemixEngine::new(SR as f32);
        load_play(&mut eng, 1, track(6.0, Some(120.0), sine(440.0, 0.5)));
        eng.handle(RemixCmd::SetCrossfader(1.0));
        eng.handle(RemixCmd::Deck(
            0,
            DeckCmd::Load(track(6.0, Some(120.0), sine(330.0, 0.5))),
        ));
        eng.handle(RemixCmd::AutoTransition {
            to: 0,
            beats: 4.0,
            fallback_secs: 1.0,
        });
        assert!(eng.state().decks[0].playing);
        // 4 beats at 120 BPM is 2.0 s.
        render(&mut eng, 2 * SR as usize + 100);
        let s = eng.state();
        assert_eq!(s.auto, None);
        assert_eq!(s.crossfader, 0.0);
    }

    #[test]
    fn auto_transition_is_ignored_for_missing_or_out_of_range_targets() {
        let mut eng = RemixEngine::new(SR as f32);
        load_play(&mut eng, 0, track(2.0, Some(120.0), sine(440.0, 0.5)));
        eng.handle(RemixCmd::AutoTransition {
            to: 1,
            beats: 8.0,
            fallback_secs: 1.0,
        });
        assert_eq!(eng.state().auto, None, "deck B is empty");
        eng.handle(RemixCmd::AutoTransition {
            to: DECKS,
            beats: 8.0,
            fallback_secs: 1.0,
        });
        assert_eq!(eng.state().auto, None, "no deck 2");
    }

    #[test]
    fn set_crossfader_cancels_a_running_transition() {
        let mut eng = RemixEngine::new(SR as f32);
        load_play(&mut eng, 0, track(10.0, Some(120.0), sine(440.0, 0.5)));
        eng.handle(RemixCmd::Deck(
            1,
            DeckCmd::Load(track(10.0, Some(120.0), sine(660.0, 0.5))),
        ));
        eng.handle(RemixCmd::AutoTransition {
            to: 1,
            beats: 8.0,
            fallback_secs: 1.0,
        });
        render(&mut eng, SR as usize / 2);
        assert!(eng.state().auto.is_some());
        assert!(bass_mults(&eng)[1] < 1.0, "bass swap is active");

        eng.handle(RemixCmd::SetCrossfader(0.3));
        let s = eng.state();
        assert_eq!(s.auto, None);
        assert_eq!(s.crossfader, 0.3);
        assert_eq!(bass_mults(&eng), [1.0; DECKS]);
        render(&mut eng, SR as usize / 2);
        assert_eq!(
            eng.state().crossfader,
            0.3,
            "nothing moves the crossfader any more"
        );
    }

    #[test]
    fn cancel_auto_transition_keeps_the_crossfader_where_it_is() {
        let mut eng = RemixEngine::new(SR as f32);
        load_play(&mut eng, 0, track(10.0, Some(120.0), sine(440.0, 0.5)));
        eng.handle(RemixCmd::Deck(
            1,
            DeckCmd::Load(track(10.0, Some(120.0), sine(660.0, 0.5))),
        ));
        eng.handle(RemixCmd::AutoTransition {
            to: 1,
            beats: 8.0,
            fallback_secs: 1.0,
        });
        render(&mut eng, SR as usize);
        // The crossfader starts at its default 0.5 and heads for 1.0; one second
        // of an 8-beat (4 s) transition is a quarter of the way.
        let at_cancel = eng.state().crossfader;
        assert!((at_cancel - 0.625).abs() < 1e-3, "crossfader {at_cancel}");
        eng.handle(RemixCmd::CancelAutoTransition);
        render(&mut eng, SR as usize / 2);
        let s = eng.state();
        assert_eq!(s.auto, None);
        assert_eq!(s.crossfader, at_cancel);
        assert_eq!(bass_mults(&eng), [1.0; DECKS]);
    }

    #[test]
    fn output_never_exceeds_the_limiter_ceiling_at_full_tilt() {
        let mut eng = RemixEngine::new(SR as f32);
        let square = |i: usize| if i % 480 < 240 { 1.0 } else { -1.0 };
        load_play(&mut eng, 0, track(2.0, Some(120.0), square));
        load_play(&mut eng, 1, track(2.0, Some(120.0), square));
        for deck in 0..DECKS {
            for band in Band::ALL {
                eng.handle(RemixCmd::SetEq {
                    deck,
                    band,
                    gain: EQ_MAX_GAIN,
                });
            }
        }
        eng.handle(RemixCmd::SetMasterGain(MASTER_GAIN_MAX));

        let mut peak = 0.0f32;
        let mut min_reduction = 1.0f32;
        for i in 0..SR as usize {
            let (l, r) = eng.process();
            assert!(l.is_finite() && r.is_finite());
            assert!(
                l.abs() <= LIMITER_CEILING && r.abs() <= LIMITER_CEILING,
                "frame {i}: {l} {r}"
            );
            peak = peak.max(l.abs()).max(r.abs());
            if i % 64 == 0 {
                min_reduction = min_reduction.min(eng.state().limiter_reduction);
            }
        }
        assert!(
            peak > 0.5,
            "the test signal should reach the limiter, peak {peak}"
        );
        assert!(
            min_reduction < 0.9,
            "the limiter should have engaged, min {min_reduction}"
        );
    }

    #[test]
    fn master_deck_reflects_the_crossfader_side() {
        let mut eng = RemixEngine::new(SR as f32);
        assert_eq!(eng.state().master_deck, None);
        load_play(&mut eng, 0, track(3.0, Some(120.0), sine(440.0, 0.5)));
        load_play(&mut eng, 1, track(3.0, Some(120.0), sine(660.0, 0.5)));
        for (pos, expect) in [(0.0, 0), (0.25, 0), (0.75, 1), (1.0, 1)] {
            eng.handle(RemixCmd::SetCrossfader(pos));
            assert_eq!(eng.state().master_deck, Some(expect), "crossfader {pos}");
        }
        // Centre with the Smooth curve is a tie in theory: the lower index wins.
        eng.handle(RemixCmd::SetCrossfader(0.5));
        assert_eq!(eng.state().master_deck, Some(0));
        // Sharp at centre is an exact tie too.
        eng.handle(RemixCmd::SetCrossfaderCurve(CrossfaderCurve::Sharp));
        assert_eq!(eng.state().master_deck, Some(0));

        // A zero fader drops that deck out of the running.
        eng.handle(RemixCmd::SetCrossfaderCurve(CrossfaderCurve::Smooth));
        eng.handle(RemixCmd::SetVolume { deck: 0, gain: 0.0 });
        eng.handle(RemixCmd::SetCrossfader(0.25));
        assert_eq!(eng.state().master_deck, Some(1));
        eng.handle(RemixCmd::SetVolume { deck: 0, gain: 1.0 });

        // Only playing decks count.
        eng.handle(RemixCmd::Deck(0, DeckCmd::Pause));
        eng.handle(RemixCmd::SetCrossfader(0.0));
        assert_eq!(eng.state().master_deck, Some(1));
        eng.handle(RemixCmd::Deck(1, DeckCmd::Pause));
        assert_eq!(eng.state().master_deck, None);
    }

    #[test]
    fn echo_repeats_one_master_beat_later_at_master_tempo() {
        // Impulse at frame 48 000 (1 s). Echo `beats` at `bpm` should repeat it
        // `beats * 60 / bpm` seconds later. The EQ spreads the impulse a little,
        // so compare the echo with the dry peak: same shape, delayed.
        for (bpm, beats, delay_frames) in [(120.0, 0.5f32, 12_000usize), (60.0, 0.5, 24_000)] {
            let mut eng = RemixEngine::new(SR as f32);
            let impulse = track(3.0, Some(bpm), |i| if i == 48_000 { 1.0 } else { 0.0 });
            load_play(&mut eng, 0, impulse);
            eng.handle(RemixCmd::SetEcho {
                mix: 1.0,
                feedback: 0.0,
                beats,
            });
            let out = render(&mut eng, 2 * SR as usize);
            let (dry_idx, dry) = peak_in(&out, 48_000 - 300..48_000 + 300);
            let expect = 48_000 + delay_frames;
            let (echo_idx, echo) = peak_in(&out, expect - 300..expect + 300);
            assert!(dry.abs() > 0.2, "bpm {bpm}: dry peak {dry}");
            let spacing = echo_idx as f64 - dry_idx as f64;
            let err_ms = (spacing - delay_frames as f64) / f64::from(SR) * 1000.0;
            assert!(
                err_ms.abs() <= 3.0,
                "bpm {bpm}: echo {spacing} frames after dry ({err_ms} ms)"
            );
            // The amplitude is not part of the requirement: the Echo's f32 delay
            // smoother settles about a sample off the target, so the copy is read
            // at a fractional offset and loses some peak height.
            assert!(echo.abs() > 0.1, "bpm {bpm}: echo {echo} vs dry {dry}");
        }
    }

    #[test]
    fn echo_falls_back_to_120_bpm_without_a_playing_master() {
        let mut eng = RemixEngine::new(SR as f32);
        eng.handle(RemixCmd::SetEcho {
            mix: 0.5,
            feedback: 0.4,
            beats: 1.0,
        });
        render(&mut eng, 64);
        assert!((eng.echo_secs - 0.5).abs() < 1e-6, "echo {}", eng.echo_secs);
    }

    #[test]
    fn incoming_values_are_clamped_and_nan_is_ignored() {
        let mut eng = RemixEngine::new(SR as f32);
        eng.handle(RemixCmd::SetVolume { deck: 0, gain: 3.0 });
        assert_eq!(eng.state().volume[0], 1.0);
        eng.handle(RemixCmd::SetVolume {
            deck: 0,
            gain: -1.0,
        });
        assert_eq!(eng.state().volume[0], 0.0);
        eng.handle(RemixCmd::SetVolume {
            deck: 0,
            gain: f32::NAN,
        });
        assert_eq!(eng.state().volume[0], 0.0);

        eng.handle(RemixCmd::SetEq {
            deck: 1,
            band: Band::High,
            gain: 10.0,
        });
        assert_eq!(eng.state().eq[1][2], EQ_MAX_GAIN);
        eng.handle(RemixCmd::SetEq {
            deck: 1,
            band: Band::High,
            gain: -3.0,
        });
        assert_eq!(eng.state().eq[1][2], 0.0);
        eng.handle(RemixCmd::SetEq {
            deck: 1,
            band: Band::High,
            gain: f32::NAN,
        });
        assert_eq!(eng.state().eq[1][2], 0.0);

        eng.handle(RemixCmd::SetFilter { deck: 0, pos: 5.0 });
        assert_eq!(eng.state().filter[0], 1.0);
        eng.handle(RemixCmd::SetFilter { deck: 0, pos: -5.0 });
        assert_eq!(eng.state().filter[0], -1.0);
        eng.handle(RemixCmd::SetFilter {
            deck: 0,
            pos: f32::NAN,
        });
        assert_eq!(eng.state().filter[0], -1.0);

        eng.handle(RemixCmd::SetCrossfader(2.0));
        assert_eq!(eng.state().crossfader, 1.0);
        eng.handle(RemixCmd::SetCrossfader(-1.0));
        assert_eq!(eng.state().crossfader, 0.0);
        eng.handle(RemixCmd::SetCrossfader(f32::NAN));
        assert_eq!(eng.state().crossfader, 0.0);

        eng.handle(RemixCmd::SetEcho {
            mix: 4.0,
            feedback: 3.0,
            beats: 100.0,
        });
        let s = eng.state();
        assert_eq!(
            (s.echo_mix, s.echo_feedback, s.echo_beats),
            (1.0, 0.95, 4.0)
        );
        eng.handle(RemixCmd::SetEcho {
            mix: f32::NAN,
            feedback: -1.0,
            beats: 0.0,
        });
        let s = eng.state();
        assert_eq!(
            (s.echo_mix, s.echo_feedback, s.echo_beats),
            (1.0, 0.0, 1.0 / 16.0)
        );

        eng.handle(RemixCmd::SetMasterGain(9.0));
        assert_eq!(eng.state().master_gain, 1.5);
        eng.handle(RemixCmd::SetMasterGain(f32::NAN));
        assert_eq!(eng.state().master_gain, 1.5);
        eng.handle(RemixCmd::SetMasterFilter(-7.0));
        assert_eq!(eng.state().master_filter, -1.0);

        eng.handle(RemixCmd::SetKill {
            deck: 0,
            band: Band::Mid,
            on: true,
        });
        assert_eq!(eng.state().kills[0], [false, true, false]);

        // Out-of-range deck indices are ignored, not panics.
        eng.handle(RemixCmd::SetVolume { deck: 7, gain: 0.5 });
        eng.handle(RemixCmd::SetEq {
            deck: 7,
            band: Band::Low,
            gain: 0.5,
        });
        eng.handle(RemixCmd::SetKill {
            deck: 7,
            band: Band::Low,
            on: true,
        });
        eng.handle(RemixCmd::SetFilter { deck: 7, pos: 0.5 });
        eng.handle(RemixCmd::SyncDeck(7));
        assert_eq!(eng.state().volume, [0.0, 1.0]);
    }

    #[test]
    fn sync_deck_matches_tempo_of_the_other_deck() {
        let mut eng = RemixEngine::new(SR as f32);
        load_play(&mut eng, 0, track(3.0, Some(120.0), sine(440.0, 0.5)));
        eng.handle(RemixCmd::Deck(
            1,
            DeckCmd::Load(track(3.0, Some(126.0), sine(660.0, 0.5))),
        ));
        eng.handle(RemixCmd::SyncDeck(1));
        let s = eng.state();
        assert!((s.decks[1].speed - 120.0 / 126.0).abs() < 1e-9);
        assert!(!s.decks[1].playing, "sync does not start the deck");

        // Syncing to a deck without tempo changes nothing, and a deck without
        // tempo cannot be synced.
        eng.handle(RemixCmd::Deck(
            0,
            DeckCmd::Load(track(3.0, None, sine(440.0, 0.5))),
        ));
        let before = eng.state().decks[1].speed;
        eng.handle(RemixCmd::SyncDeck(1));
        assert_eq!(eng.state().decks[1].speed, before);
        eng.handle(RemixCmd::SyncDeck(0));
        assert_eq!(eng.state().decks[0].speed, 1.0);
    }

    #[test]
    fn deck_commands_are_forwarded_and_replaced_tracks_come_back() {
        let mut eng = RemixEngine::new(SR as f32);
        let first = track(1.0, Some(120.0), sine(440.0, 0.5));
        let second = track(1.0, Some(120.0), sine(330.0, 0.5));
        assert!(eng
            .handle(RemixCmd::Deck(0, DeckCmd::Load(first.clone())))
            .is_none());
        let replaced = eng.handle(RemixCmd::Deck(0, DeckCmd::Load(second.clone())));
        assert!(Arc::ptr_eq(&replaced.expect("old track returned"), &first));
        assert!(eng.state().decks[0].loaded);
        assert_eq!(eng.state().decks[0].track_id, second.id);

        // A load to a deck that does not exist hands the track straight back.
        let stray = track(0.1, None, |_| 0.0);
        let back = eng.handle(RemixCmd::Deck(DECKS, DeckCmd::Load(stray.clone())));
        assert!(Arc::ptr_eq(&back.expect("track returned"), &stray));
    }

    #[test]
    fn meter_peaks_follow_the_fader_and_decay() {
        let mut eng = RemixEngine::new(SR as f32);
        load_play(&mut eng, 0, track(3.0, Some(120.0), sine(440.0, 0.5)));
        render(&mut eng, SR as usize / 2);
        // 0.5 amplitude at the centre crossfader gain (-3 dB).
        let level = eng.state().levels[0];
        assert!((0.34..=0.36).contains(&level), "level {level}");
        assert!(eng.state().master_level > 0.3);

        eng.handle(RemixCmd::Deck(0, DeckCmd::Pause));
        render(&mut eng, SR as usize);
        let decayed = eng.state().levels[0];
        assert!(
            decayed < 0.02,
            "meter should fall by ~1/e per 0.3 s, now {decayed}"
        );
    }

    #[test]
    fn master_gain_scales_the_output() {
        let mut eng = RemixEngine::new(SR as f32);
        load_play(&mut eng, 0, track(3.0, Some(120.0), sine(440.0, 0.25)));
        render(&mut eng, SR as usize / 2);
        let base = amp_at(&render(&mut eng, SR as usize / 2), 440.0);
        eng.handle(RemixCmd::SetMasterGain(0.5));
        render(&mut eng, SR as usize / 2);
        let half = amp_at(&render(&mut eng, SR as usize / 2), 440.0);
        assert!((half / base - 0.5).abs() < 0.01, "ratio {}", half / base);
    }

    #[test]
    fn master_filter_darkens_the_bus() {
        let mut eng = RemixEngine::new(SR as f32);
        load_play(&mut eng, 0, track(3.0, Some(120.0), sine(8_000.0, 0.5)));
        render(&mut eng, SR as usize / 2);
        let open = amp_at(&render(&mut eng, SR as usize / 2), 8_000.0);
        eng.handle(RemixCmd::SetMasterFilter(-1.0));
        render(&mut eng, SR as usize / 2);
        let closed = amp_at(&render(&mut eng, SR as usize / 2), 8_000.0);
        assert!(closed < open * 0.1, "open {open}, closed {closed}");
    }

    #[test]
    fn eq_kill_removes_the_band_from_the_mix() {
        let mut eng = RemixEngine::new(SR as f32);
        load_play(&mut eng, 0, track(3.0, Some(120.0), sine(60.0, 0.5)));
        render(&mut eng, SR as usize / 2);
        let open = amp_at(&render(&mut eng, SR as usize / 2), 60.0);
        eng.handle(RemixCmd::SetKill {
            deck: 0,
            band: Band::Low,
            on: true,
        });
        render(&mut eng, SR as usize / 2);
        let killed = amp_at(&render(&mut eng, SR as usize / 2), 60.0);
        assert!(killed < open * 0.02, "open {open}, killed {killed}");
    }

    #[test]
    fn curve_change_does_not_cancel_a_transition() {
        let mut eng = RemixEngine::new(SR as f32);
        load_play(&mut eng, 0, track(10.0, Some(120.0), sine(440.0, 0.5)));
        eng.handle(RemixCmd::Deck(
            1,
            DeckCmd::Load(track(10.0, Some(120.0), sine(660.0, 0.5))),
        ));
        eng.handle(RemixCmd::AutoTransition {
            to: 1,
            beats: 8.0,
            fallback_secs: 1.0,
        });
        eng.handle(RemixCmd::SetCrossfaderCurve(CrossfaderCurve::Sharp));
        assert!(eng.state().auto.is_some());
        assert_eq!(eng.state().curve, CrossfaderCurve::Sharp);
    }

    /// Tiny deterministic PRNG (xorshift64*).
    struct Prng(u64);

    impl Prng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.0 = x;
            x.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }

        fn pick(&mut self, n: u64) -> u64 {
            self.next() % n
        }

        fn unit(&mut self) -> f32 {
            (self.next() >> 40) as f32 / (1u64 << 24) as f32
        }

        /// Mostly ordinary values, with NaN, infinities and extremes mixed in.
        fn value(&mut self) -> f32 {
            match self.pick(20) {
                0 => f32::NAN,
                1 => f32::INFINITY,
                2 => f32::NEG_INFINITY,
                3 => 1.0e30,
                4 => -1.0e30,
                _ => self.unit() * 8.0 - 4.0,
            }
        }

        fn opt_f64(&mut self) -> Option<f64> {
            match self.pick(3) {
                0 => None,
                _ => Some(f64::from(self.value())),
            }
        }
    }

    fn random_deck_cmd(p: &mut Prng, tracks: &[Arc<Track>; 2]) -> DeckCmd {
        match p.pick(21) {
            0 => DeckCmd::Load(tracks[p.pick(2) as usize].clone()),
            1 => DeckCmd::Eject,
            2 => DeckCmd::TogglePlay,
            3 => DeckCmd::Play,
            4 => DeckCmd::Pause,
            5 => DeckCmd::Cue,
            6 => DeckCmd::HotCue(p.pick(HOT_CUES as u64 + 2) as usize),
            7 => DeckCmd::ClearHotCue(p.pick(HOT_CUES as u64 + 2) as usize),
            8 => DeckCmd::SetHotCues([p.opt_f64(), p.opt_f64(), p.opt_f64(), p.opt_f64()]),
            9 => DeckCmd::BeatLoop(p.value()),
            10 => DeckCmd::ExitLoop,
            11 => DeckCmd::LoopHalve,
            12 => DeckCmd::LoopDouble,
            13 => DeckCmd::SetTempoFader(p.value()),
            14 => DeckCmd::SetTempoRange(TEMPO_RANGES[p.pick(3) as usize]),
            15 => DeckCmd::Nudge(p.value()),
            16 => DeckCmd::SeekFraction(f64::from(p.value())),
            17 => DeckCmd::BeatJump(p.value()),
            18 => DeckCmd::Sync {
                bpm: f64::from(p.value()) * 40.0 + 100.0,
                phase: p.opt_f64(),
            },
            19 => DeckCmd::SetBpm(f64::from(p.value()) * 100.0),
            _ => DeckCmd::SetGridAnchorHere,
        }
    }

    fn random_cmd(p: &mut Prng, tracks: &[Arc<Track>; 2]) -> RemixCmd {
        let deck = p.pick(3) as usize;
        let band = Band::ALL[p.pick(3) as usize];
        match p.pick(13) {
            0 => RemixCmd::Deck(deck, random_deck_cmd(p, tracks)),
            1 => RemixCmd::SetEq {
                deck,
                band,
                gain: p.value(),
            },
            2 => RemixCmd::SetKill {
                deck,
                band,
                on: p.pick(2) == 0,
            },
            3 => RemixCmd::SetFilter {
                deck,
                pos: p.value(),
            },
            4 => RemixCmd::SetVolume {
                deck,
                gain: p.value(),
            },
            5 => RemixCmd::SetCrossfader(p.value()),
            6 => RemixCmd::SetCrossfaderCurve(if p.pick(2) == 0 {
                CrossfaderCurve::Smooth
            } else {
                CrossfaderCurve::Sharp
            }),
            7 => RemixCmd::AutoTransition {
                to: p.pick(3) as usize,
                beats: p.value(),
                fallback_secs: p.value(),
            },
            8 => RemixCmd::CancelAutoTransition,
            9 => RemixCmd::SyncDeck(deck),
            10 => RemixCmd::SetMasterFilter(p.value()),
            11 => RemixCmd::SetEcho {
                mix: p.value(),
                feedback: p.value(),
                beats: p.value(),
            },
            _ => RemixCmd::SetMasterGain(p.value()),
        }
    }

    #[test]
    fn random_commands_never_panic_or_produce_non_finite_output() {
        let tracks = [
            track(1.5, Some(120.0), sine(220.0, 0.8)),
            track(1.0, None, sine(90.0, 0.8)),
        ];
        let mut p = Prng(0x9E37_79B9_7F4A_7C15);
        let mut eng = RemixEngine::new(SR as f32);
        for _ in 0..400 {
            let cmd = random_cmd(&mut p, &tracks);
            // A returned track is dropped here, like the caller would free it.
            let _ = eng.handle(cmd);
            for _ in 0..p.pick(200) {
                let (l, r) = eng.process();
                assert!(l.is_finite() && r.is_finite());
                assert!(l.abs() <= LIMITER_CEILING && r.abs() <= LIMITER_CEILING);
            }
            assert_state_finite(&eng.state());
        }
    }
}

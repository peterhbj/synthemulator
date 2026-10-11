//! One playback deck: transport, cue points, beat loops, tempo and sync.
//!
//! Runs on the audio thread. `process` and `handle` never allocate; a
//! replaced track is handed back to the caller so it is not freed here.

use std::sync::Arc;

use crate::track::Track;

pub const HOT_CUES: usize = 4;
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
    /// also shift the playhead by less than half a beat so phases match.
    Sync { bpm: f64, phase: Option<f64> },
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

pub struct Deck {
    sample_rate: f32,
    track: Option<Arc<Track>>,
}

impl Deck {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            track: None,
        }
    }

    /// Apply a command. Returns the previous track when one was replaced or
    /// ejected, so the caller can free it off the audio thread.
    pub fn handle(&mut self, cmd: DeckCmd) -> Option<Arc<Track>> {
        let _ = (cmd, self.sample_rate, &self.track);
        todo!("deck agent")
    }

    /// Next stereo frame at the engine rate.
    #[inline]
    pub fn process(&mut self) -> (f32, f32) {
        todo!("deck agent")
    }

    pub fn state(&self) -> DeckState {
        todo!("deck agent")
    }

    pub fn is_playing(&self) -> bool {
        todo!("deck agent")
    }

    /// `base_bpm * speed`, if the track has tempo.
    pub fn effective_bpm(&self) -> Option<f64> {
        todo!("deck agent")
    }

    pub fn beat_phase(&self) -> Option<f64> {
        todo!("deck agent")
    }
}

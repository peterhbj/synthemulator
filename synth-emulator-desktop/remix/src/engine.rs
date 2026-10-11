//! Two decks into a DJ mixer: per-channel EQ, filter and fader, crossfader,
//! automatic transitions, master filter, echo and limiter.

use std::sync::Arc;

use crate::deck::{Deck, DeckCmd, DeckState};
use crate::dsp::{Band, CrossfaderCurve};
use crate::track::Track;

pub const DECKS: usize = 2;

#[derive(Clone, Debug)]
pub enum RemixCmd {
    Deck(usize, DeckCmd),
    /// Linear gain 0..=EQ_MAX_GAIN (1 = flat).
    SetEq { deck: usize, band: Band, gain: f32 },
    SetKill { deck: usize, band: Band, on: bool },
    /// -1..=1, see `DjFilter`.
    SetFilter { deck: usize, pos: f32 },
    /// Channel fader 0..=1.
    SetVolume { deck: usize, gain: f32 },
    /// 0 = deck A, 1 = deck B.
    SetCrossfader(f32),
    SetCrossfaderCurve(CrossfaderCurve),
    /// Blend to deck `to` over `beats` beats of the playing deck (or
    /// `fallback_secs` without tempo). Starts `to` if paused, syncing tempo
    /// and phase first when both decks have tempo, and swaps the bass at the
    /// midpoint so two kick drums never stack.
    AutoTransition { to: usize, beats: f32, fallback_secs: f32 },
    CancelAutoTransition,
    /// Sync `deck` to the other deck (tempo and phase).
    SyncDeck(usize),
    /// -1..=1 on the master bus.
    SetMasterFilter(f32),
    /// Echo send 0..=1, feedback 0..=0.95, time in beats of the master deck
    /// (120 BPM without tempo).
    SetEcho { mix: f32, feedback: f32, beats: f32 },
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

pub struct RemixEngine {
    sample_rate: f32,
    decks: [Deck; DECKS],
}

impl RemixEngine {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            decks: [Deck::new(sample_rate), Deck::new(sample_rate)],
        }
    }

    /// Apply a command. Returns a replaced track so the caller frees it off
    /// the audio thread.
    pub fn handle(&mut self, cmd: RemixCmd) -> Option<Arc<Track>> {
        let _ = (cmd, self.sample_rate, &self.decks);
        todo!("engine agent")
    }

    /// Next stereo master frame.
    #[inline]
    pub fn process(&mut self) -> (f32, f32) {
        todo!("engine agent")
    }

    pub fn state(&self) -> RemixState {
        todo!("engine agent")
    }
}

// Keep the imports referenced until the engine is implemented.
#[allow(dead_code)]
fn _contract(_: Band, _: CrossfaderCurve, _: DeckCmd) {}

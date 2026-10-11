//! Live remix engine for Helix's Remix tab: file decoding, tempo analysis,
//! decks and the two-channel DJ mixer.
//!
//! Nothing here touches the UI or an audio device, so every piece is
//! unit-testable. The app owns one [`RemixEngine`] on the audio thread and
//! talks to it with [`RemixCmd`]s, the same way the synth uses `Command`.

pub mod analysis;
pub mod decode;
pub mod deck;
pub mod dsp;
pub mod engine;
pub mod track;

pub use analysis::{build_overview, detect_tempo, Analysis, Overview, OverviewBin, TapTempo};
pub use decode::{decode_file, is_supported, load_track, resample_stereo, LoadError, SUPPORTED_EXTENSIONS};
pub use deck::{Deck, DeckCmd, DeckState, LoopState, HOT_CUES, TEMPO_RANGES};
pub use dsp::{Band, CrossfaderCurve};
pub use engine::{RemixCmd, RemixEngine, RemixState, DECKS};
pub use track::Track;

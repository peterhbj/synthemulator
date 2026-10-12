//! Settings that survive a restart: music folder, XY pad assignments,
//! mixer preferences and per-track memory (hot cues, tempo override).
//!
//! Stored as JSON in the user's config folder:
//! Linux `$XDG_CONFIG_HOME/helix` (or `~/.config/helix`),
//! Windows `%APPDATA%\Helix`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::xy::XyTarget;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub music_dir: Option<PathBuf>,
    pub xy: XySettings,
    /// Beats for the automatic transition (Space).
    pub auto_beats: f32,
    pub sharp_crossfader: bool,
    /// Keyed by the track's full path.
    pub tracks: HashMap<String, TrackMemo>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            music_dir: None,
            xy: XySettings::default(),
            auto_beats: 16.0,
            sharp_crossfader: false,
            tracks: HashMap::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct XySettings {
    /// X and Y targets while the Synth tab is open.
    pub synth: [XyTarget; 2],
    /// X and Y targets while the Remix tab is open.
    pub remix: [XyTarget; 2],
    /// Lifting the finger puts the parameters back where they were.
    pub spring_back: bool,
    /// Relative mode only: fraction of the pad per 100 px of motion.
    pub sensitivity: f32,
}

impl Default for XySettings {
    fn default() -> Self {
        Self {
            synth: [XyTarget::SynthCutoff, XyTarget::SynthResonance],
            remix: [XyTarget::MasterFilter, XyTarget::EchoMix],
            spring_back: true,
            sensitivity: 0.35,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TrackMemo {
    #[serde(deserialize_with = "de_hot_cues")]
    pub hot_cues: [Option<f64>; remix::HOT_CUES],
    /// Manual / tapped tempo that replaces the detected one.
    pub bpm: Option<f64>,
}

/// Saved files may hold fewer slots (four before the extra ones). Missing
/// slots stay empty, instead of the whole memo failing to load.
fn de_hot_cues<'de, D: serde::Deserializer<'de>>(d: D) -> Result<[Option<f64>; remix::HOT_CUES], D::Error> {
    let saved = Vec::<Option<f64>>::deserialize(d)?;
    let mut cues = [None; remix::HOT_CUES];
    for (slot, cue) in cues.iter_mut().zip(saved) {
        *slot = cue;
    }
    Ok(cues)
}

impl TrackMemo {
    pub fn is_empty(&self) -> bool {
        self.bpm.is_none() && self.hot_cues.iter().all(Option::is_none)
    }
}

pub fn config_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join("Helix"))
    }
    #[cfg(not(windows))]
    {
        if let Some(x) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
            return Some(PathBuf::from(x).join("helix"));
        }
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config").join("helix"))
    }
}

/// Best guess for the user's music folder.
pub fn default_music_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from);
    #[cfg(not(windows))]
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let home = home?;
    for name in ["Music", "Músicas", "Musicas", "Música"] {
        let p = home.join(name);
        if p.is_dir() {
            return Some(p);
        }
    }
    Some(home)
}

impl Settings {
    fn path() -> Option<PathBuf> {
        config_dir().map(|d| d.join("settings.json"))
    }

    /// Missing or unreadable settings fall back to defaults; a corrupt file
    /// is kept aside as `settings.json.bad` instead of being overwritten.
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        Self::load_from(&path)
    }

    pub fn load_from(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(s) => s,
                Err(e) => {
                    crate::log::warn(&format!("settings ignored ({e})"));
                    let _ = std::fs::rename(path, path.with_extension("json.bad"));
                    Self::default()
                }
            },
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path().ok_or("sem pasta de configuração")?;
        self.save_to(&path)
    }

    /// Atomic write: temp file then rename.
    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())
    }

    pub fn memo(&self, track: &Path) -> TrackMemo {
        self.tracks
            .get(&track.to_string_lossy().into_owned())
            .cloned()
            .unwrap_or_default()
    }

    /// Store (or forget, when empty) a track's memo. Returns true on change.
    pub fn set_memo(&mut self, track: &Path, memo: TrackMemo) -> bool {
        let key = track.to_string_lossy().into_owned();
        if memo.is_empty() {
            return self.tracks.remove(&key).is_some();
        }
        if self.tracks.get(&key) == Some(&memo) {
            return false;
        }
        self.tracks.insert(key, memo);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("helix-settings-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("settings.json")
    }

    #[test]
    fn round_trip() {
        let path = temp_file("rt");
        let mut s = Settings {
            music_dir: Some(PathBuf::from("/music")),
            ..Settings::default()
        };
        s.xy.remix = [XyTarget::Crossfader, XyTarget::EchoFeedback];
        let mut memo = TrackMemo {
            bpm: Some(124.0),
            ..TrackMemo::default()
        };
        memo.hot_cues[1] = Some(12.5);
        assert!(s.set_memo(Path::new("/music/a.mp3"), memo.clone()));
        assert!(!s.set_memo(Path::new("/music/a.mp3"), memo.clone()));
        s.save_to(&path).unwrap();
        let back = Settings::load_from(&path);
        assert_eq!(back, s);
        assert_eq!(back.memo(Path::new("/music/a.mp3")), memo);
    }

    #[test]
    fn missing_fields_take_defaults() {
        let path = temp_file("partial");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"auto_beats": 8.0}"#).unwrap();
        let s = Settings::load_from(&path);
        assert_eq!(s.auto_beats, 8.0);
        assert_eq!(s.xy, XySettings::default());
    }

    #[test]
    fn memo_saved_with_four_hot_cue_slots_still_loads() {
        let old = r#"{"bpm": 124.0, "hot_cues": [1.5, null, 3.0, null]}"#;
        let memo: TrackMemo = serde_json::from_str(old).expect("old memo loads");
        assert_eq!(memo.bpm, Some(124.0));
        assert_eq!(memo.hot_cues[0], Some(1.5));
        assert_eq!(memo.hot_cues[2], Some(3.0));
        assert!(memo.hot_cues[4..].iter().all(Option::is_none));
    }

    #[test]
    fn corrupt_file_is_kept_aside() {
        let path = temp_file("bad");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{not json").unwrap();
        let s = Settings::load_from(&path);
        assert_eq!(s, Settings::default());
        assert!(path.with_extension("json.bad").exists());
    }

    #[test]
    fn empty_memo_is_forgotten() {
        let mut s = Settings::default();
        let memo = TrackMemo {
            bpm: Some(120.0),
            ..TrackMemo::default()
        };
        s.set_memo(Path::new("x"), memo);
        assert!(s.set_memo(Path::new("x"), TrackMemo::default()));
        assert!(s.tracks.is_empty());
    }
}

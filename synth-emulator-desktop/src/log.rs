//! Diagnostics that also reach Windows users: release builds there have no
//! console, so every message is appended to `%APPDATA%\Helix\helix.log` as
//! well as printed to stderr.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

/// Repeats of the same message closer together than this are dropped.
const THROTTLE_MS: u64 = 2_000;

/// Rate limit for a message that can repeat on every audio callback or poll.
pub struct Throttle {
    /// Milliseconds since start when this message was last printed; 0 = never.
    last_ms: AtomicU64,
}

impl Default for Throttle {
    fn default() -> Self {
        Self::new()
    }
}

impl Throttle {
    pub const fn new() -> Self {
        Self {
            last_ms: AtomicU64::new(0),
        }
    }

    /// Like [`warn`], but drops the message if this throttle printed one less
    /// than `THROTTLE_MS` ago.
    pub fn warn(&self, msg: &str) {
        static START: OnceLock<Instant> = OnceLock::new();
        let now = START.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1;
        let last = self.last_ms.load(Ordering::Relaxed);
        if due(last, now) {
            self.last_ms.store(now, Ordering::Relaxed);
            warn(msg);
        }
    }
}

/// Whether a message last printed at `last` (0 = never) is due at `now`.
fn due(last: u64, now: u64) -> bool {
    last == 0 || now.saturating_sub(last) >= THROTTLE_MS
}

/// Largest log file kept; past this it starts over.
#[cfg(windows)]
const MAX_LOG_BYTES: u64 = 512 * 1024;

/// Print `msg` to stderr and, on Windows, append it to the log file.
pub fn warn(msg: &str) {
    eprintln!("helix: {msg}");
    #[cfg(windows)]
    append(msg);
}

#[cfg(windows)]
fn append(msg: &str) {
    use std::io::Write;

    let Some(dir) = crate::settings::config_dir() else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("helix.log");
    let truncate = std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LOG_BYTES);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(!truncate)
        .truncate(truncate)
        .open(&path);
    if let Ok(mut f) = file {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let _ = writeln!(f, "[{secs}] {msg}");
    }
}

/// Route panic messages through [`warn`] too, then run the default hook.
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        warn(&format!("panic: {info}"));
        default(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_message_is_always_due() {
        assert!(due(0, 1));
    }

    #[test]
    fn repeats_inside_the_window_are_dropped() {
        assert!(!due(1_000, 2_000));
        assert!(!due(1_000, 2_999));
    }

    #[test]
    fn repeats_after_the_window_print_again() {
        assert!(due(1_000, 3_000));
    }
}

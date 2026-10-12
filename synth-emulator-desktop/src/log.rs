//! Diagnostics that also reach Windows users: release builds there have no
//! console, so every message is appended to `%APPDATA%\Helix\helix.log` as
//! well as printed to stderr.

/// Largest log file kept; past this it starts over.
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

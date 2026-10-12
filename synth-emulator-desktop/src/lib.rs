pub mod audio;
pub mod log;
pub mod settings;
pub mod synth;
pub mod ui;
pub mod xy;

pub fn run() -> eframe::Result<()> {
    log::install_panic_hook();
    let result = ui::run();
    if let Err(e) = &result {
        // A window that never opens is otherwise silent on Windows.
        log::warn(&format!("could not start: {e}"));
    }
    result
}

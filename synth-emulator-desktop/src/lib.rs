pub mod audio;
pub mod settings;
pub mod synth;
pub mod ui;
pub mod xy;

pub fn run() -> eframe::Result<()> {
    ui::run()
}

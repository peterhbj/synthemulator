pub mod audio;
pub mod synth;
pub mod ui;

pub fn run() -> eframe::Result<()> {
    ui::run()
}

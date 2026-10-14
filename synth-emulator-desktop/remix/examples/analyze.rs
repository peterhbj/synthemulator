//! Print the analysis of each file given on the command line.
use std::path::Path;

fn main() {
    for arg in std::env::args().skip(1) {
        let path = Path::new(&arg);
        match remix::decode::load_track(path, 48_000) {
            Ok(t) => println!(
                "{:>8} conf {:.2}  {:>4} {:>3} {:+4.0}c conf {:.2}  {}",
                t.analysis.bpm.map(|b| format!("{b:.2}")).unwrap_or("--".into()),
                t.analysis.confidence,
                t.key.key.map(|k| k.name()).unwrap_or("--".into()),
                t.key.key.map(|k| k.camelot()).unwrap_or("--".into()),
                t.key.tuning_cents,
                t.key.confidence,
                t.title
            ),
            Err(e) => println!("erro {e}  {arg}"),
        }
    }
}

//! `mlo` — binary entry point.

fn main() {
    if let Err(e) = mlo_tui::cli::run() {
        eprintln!("mlo: {e}");
        std::process::exit(1);
    }
}
// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Some(code) = orag_desktop::fake::run_if_requested() {
        std::process::exit(code);
    }
    orag_desktop::app::run();
}

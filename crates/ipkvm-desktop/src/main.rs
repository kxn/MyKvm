#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    if let Err(error) = ipkvm_desktop::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

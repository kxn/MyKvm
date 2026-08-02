fn main() {
    if let Err(error) = ipkvm_desktop::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

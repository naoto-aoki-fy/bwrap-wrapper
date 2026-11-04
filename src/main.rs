fn main() {
    if let Err(error) = bwrap_wrapper_rs::runtime::run() {
        eprintln!("bwrap-wrapper: {error}");
        std::process::exit(1);
    }
}

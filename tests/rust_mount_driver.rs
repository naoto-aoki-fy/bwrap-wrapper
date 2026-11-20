//! Run the unchanged Rust reference with a synthetic mount table.
use bwrap_wrapper_rs::{mounts, runtime};

fn main() {
    let mount_file = std::env::var_os("BWRAP_TEST_MOUNTS").expect("synthetic mount table");
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    if let Err(error) = runtime::run_with_args(&args, || mounts::read_file(&mount_file)) {
        eprintln!("bwrap-wrapper: {error}");
        std::process::exit(1);
    }
}

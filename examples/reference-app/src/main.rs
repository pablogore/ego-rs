//! Thin entry point — see `lib.rs` for the illustrative CORE-016 pipeline.

use reference_app::{build_runtime, AppConfig};

fn main() {
    let config = AppConfig::default();

    match build_runtime(&config) {
        Ok(_runtime) => println!("reference-app: runtime constructed from AppConfig"),
        Err(err) => eprintln!("reference-app: invalid configuration: {err}"),
    }
}

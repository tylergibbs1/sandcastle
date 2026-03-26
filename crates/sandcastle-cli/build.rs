use std::path::Path;
use std::process::Command;

fn main() {
    let wasm_path = Path::new("../../guest/target/wasm32-wasip1/release/sandcastle_guest_js.wasm");

    // Tell Cargo to re-run if the WASM changes
    println!("cargo:rerun-if-changed={}", wasm_path.display());

    if !wasm_path.exists() {
        eprintln!("Guest WASM not found at {}. Building it...", wasm_path.display());

        let status = Command::new("cargo")
            .args(["build", "--target", "wasm32-wasip1", "--release"])
            .current_dir("../../guest")
            .status();

        match status {
            Ok(s) if s.success() => {
                eprintln!("Guest WASM built successfully.");
            }
            Ok(s) => {
                panic!(
                    "Failed to build guest WASM (exit code: {}). \
                     Run manually: cd guest && cargo build --target wasm32-wasip1 --release",
                    s
                );
            }
            Err(e) => {
                panic!(
                    "Failed to run cargo for guest build: {e}. \
                     Ensure wasm32-wasip1 target is installed: rustup target add wasm32-wasip1"
                );
            }
        }
    }
}

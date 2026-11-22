//! A command for Fenders: it runs once per `fenders run`, prints to its
//! logs, and its exit code is the run's.
//!
//! Build: cargo build --release --target wasm32-wasip2
//! Output: target/wasm32-wasip2/release/fenders-command.wasm
//!
//! A run takes no arguments or input: give it data in environment variables
//! (`fenders deploy … -e KEY=VALUE`, `--secret-env KEY=SECRET`).

fn main() {
    let name = std::env::var("NAME").unwrap_or_else(|_| "Fenders".into());
    println!("Hello, {name}.");
    // Every run is told the time it was triggered for (its schedule's, or now).
    if let Ok(t) = std::env::var("FENDERS_TRIGGER_TIME") {
        println!("Triggered for {t}.");
    }
}

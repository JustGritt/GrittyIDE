//! Writes `fixtures/large-repo` (git-ignored). Usage:
//!
//! ```text
//! cargo run -p gritty-workspace --example generate_large_repo [-- <output dir>]
//! ```

use std::path::PathBuf;
use std::time::Instant;

use gritty_workspace::fixtures::generate_large_repo;

fn main() {
    let output = std::env::args().nth(1).map_or_else(
        || {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("fixtures")
                .join("large-repo")
        },
        PathBuf::from,
    );
    let started = Instant::now();
    match generate_large_repo(&output) {
        Ok(summary) => println!(
            "generated {} files in {} directories (max depth {}) at {} in {:.1?}",
            summary.files,
            summary.directories,
            summary.max_depth,
            output.display(),
            started.elapsed()
        ),
        Err(error) => {
            eprintln!("failed to generate {}: {error}", output.display());
            std::process::exit(1);
        }
    }
}

//! Adds the generated members of `fixtures/weird-files` (git-ignored
//! `generated/` directory: a 12 MiB text file and a permission-denied file).
//! Usage:
//!
//! ```text
//! cargo run -p gritty-workspace --example generate_weird_files [-- <fixture dir>]
//! ```

use std::path::PathBuf;

use gritty_workspace::fixtures::{WEIRD_FILES_DIR, augment_weird_files};

fn main() {
    let dir = std::env::args().nth(1).map_or_else(
        || {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("fixtures")
                .join(WEIRD_FILES_DIR)
        },
        PathBuf::from,
    );
    match augment_weird_files(&dir) {
        Ok((bytes, denied)) => println!(
            "wrote generated/large-text.txt ({bytes} bytes) and generated/denied.txt (mode 000: {denied}) under {}",
            dir.display()
        ),
        Err(error) => {
            eprintln!("failed to augment {}: {error}", dir.display());
            std::process::exit(1);
        }
    }
}

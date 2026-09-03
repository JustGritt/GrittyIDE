use std::env;
use std::path::PathBuf;

fn main() {
    tauri_build::build();
    embed_windows_test_manifest();
}

/// Tauri embeds its Windows application manifest (which declares the Common
/// Controls v6 dependency) into the app binary only. Test binaries link the
/// same dialog plugin, whose `rfd` backend imports `TaskDialogIndirect` from
/// ComCtl32 v6; without a manifest they fail to load on Windows with
/// `STATUS_ENTRYPOINT_NOT_FOUND`. Give them the same manifest.
fn embed_windows_test_manifest() {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os != "windows" || target_env != "msvc" {
        return;
    }
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"))
        .join("windows-test-manifest.xml");
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rustc-link-arg-tests=/MANIFEST:EMBED");
    println!(
        "cargo:rustc-link-arg-tests=/MANIFESTINPUT:{}",
        manifest.display()
    );
}

// Compiles the stylesheet (style/style.scss, with style/theme.scss) into the
// binary (main.rs).
use std::{env, path::PathBuf, process::Command};

fn main() {
    let css = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is set")).join("style.css");
    let status = Command::new("sass")
        .arg("--no-source-map")
        .arg("style/style.scss")
        .arg(&css)
        .status()
        .expect("dart-sass's sass is on PATH (nix develop)");
    assert!(status.success(), "sass failed on style/style.scss");
    println!("cargo::rerun-if-changed=style");
}

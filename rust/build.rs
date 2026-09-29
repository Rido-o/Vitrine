// Compiles the TypeScript app's stylesheet (../src/style.scss), shared by both
// apps until the TypeScript one retires, into the binary (main.rs).
use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo::rerun-if-changed=../src/style.scss");
    println!("cargo::rerun-if-changed=../src/theme.scss");
    let css = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is set")).join("style.css");
    let status = Command::new("sass")
        .arg("--no-source-map")
        .arg("../src/style.scss")
        .arg(&css)
        .status()
        .expect("dart-sass's sass is on PATH (nix develop .#spike)");
    assert!(status.success(), "sass failed on ../src/style.scss");
}

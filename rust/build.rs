// Compiles the TypeScript app's stylesheet (../src/style.scss), shared by both
// apps until the TypeScript one retires, and the spike's overrides
// (src/overrides.scss) into the binary (main.rs).
use std::{env, path::PathBuf, process::Command};

fn main() {
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is set"));
    for (source, css) in [
        ("../src/style.scss", "style.css"),
        ("src/overrides.scss", "overrides.css"),
    ] {
        let status = Command::new("sass")
            .arg("--no-source-map")
            .arg("--load-path=../src")
            .arg(source)
            .arg(out.join(css))
            .status()
            .expect("dart-sass's sass is on PATH (nix develop .#spike)");
        assert!(status.success(), "sass failed on {source}");
    }
    for source in [
        "../src/style.scss",
        "../src/theme.scss",
        "src/overrides.scss",
    ] {
        println!("cargo::rerun-if-changed={source}");
    }
}

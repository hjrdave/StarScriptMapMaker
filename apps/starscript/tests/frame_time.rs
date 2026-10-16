//! `--frame-time` on the software adapter: prints the statistics and exits cleanly.

use std::process::Command;

/// Set and non-empty, as CI does for the golden tests.
fn env_flag(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| !value.is_empty())
}

#[test]
fn prints_statistics_and_exits() {
    let output = Command::new(env!("CARGO_BIN_EXE_starscript-map-maker"))
        .args(["--frame-time", "--software-adapter", "--scale", "0.01"])
        .output()
        .unwrap();
    if output.status.code() == Some(2) && !env_flag("SSMAP_REQUIRE_SOFTWARE_GPU") {
        eprintln!(
            "skipping: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        return;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{}\n{}",
        stdout,
        String::from_utf8_lossy(&output.stderr)
    );
    for label in [
        "adapter: ",
        "backend: ",
        "os: ",
        "driver: ",
        "frames: 180",
        "mean: ",
        "p95: ",
        "worst: ",
    ] {
        assert!(stdout.contains(label), "missing {label:?} in:\n{stdout}");
    }
}

#[test]
fn rejects_a_bad_scale() {
    let output = Command::new(env!("CARGO_BIN_EXE_starscript-map-maker"))
        .args(["--frame-time", "--scale", "0"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
}

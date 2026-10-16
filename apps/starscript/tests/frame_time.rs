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
fn runs_an_interaction_script() {
    let output = Command::new(env!("CARGO_BIN_EXE_starscript-map-maker"))
        .args([
            "--frame-time",
            "--software-adapter",
            "--scale",
            "0.01",
            "--script",
            "paint",
            "--frames",
            "2",
        ])
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
    assert!(stdout.contains("script: paint\nframes: 2\n"), "{stdout}");
    // Built with `profiling`, the three coarse stages are six finer ones (see below).
    let stages: [&str; 3] = if cfg!(feature = "profiling") {
        ["  input ", "  update ", "  draw "]
    } else {
        ["  edit ", "  scene ", "  render "]
    };
    for label in ["p50: ", "p99: ", "budget: "].into_iter().chain(stages) {
        assert!(stdout.contains(label), "missing {label:?} in:\n{stdout}");
    }
}

/// With the `profiling` feature the report breaks the frame into six stages that add up to it.
#[cfg(feature = "profiling")]
#[test]
fn reports_the_six_stages_and_they_add_up() {
    let output = Command::new(env!("CARGO_BIN_EXE_starscript-map-maker"))
        .args([
            "--frame-time",
            "--software-adapter",
            "--scale",
            "0.01",
            "--script",
            "paint",
            "--frames",
            "5",
        ])
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
    assert!(output.status.success(), "{stdout}");
    for stage in [
        "input",
        "update",
        "roughness",
        "tessellation",
        "upload",
        "draw",
    ] {
        assert!(
            stdout.contains(&format!("  {stage} ")),
            "missing stage {stage:?} in:\n{stdout}"
        );
    }
    let share: f64 = stdout
        .lines()
        .find_map(|line| line.strip_prefix("stages add up to "))
        .and_then(|rest| rest.split('%').next())
        .and_then(|number| number.parse().ok())
        .unwrap_or_else(|| panic!("no coverage line in:\n{stdout}"));
    // The stages cover the frame, give or take setup between them and timer noise.
    assert!((85.0..=100.5).contains(&share), "{share}% in:\n{stdout}");
}

#[test]
fn rejects_an_unknown_script() {
    let output = Command::new(env!("CARGO_BIN_EXE_starscript-map-maker"))
        .args(["--frame-time", "--script", "nope"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn rejects_a_bad_scale() {
    let output = Command::new(env!("CARGO_BIN_EXE_starscript-map-maker"))
        .args(["--frame-time", "--scale", "0"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
}

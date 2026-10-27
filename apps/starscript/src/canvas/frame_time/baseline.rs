//! The checked-in frame-time baseline (`docs/perf-baseline.json`, issue #338) and the comparison
//! that the nightly regression check runs against it.
//!
//! Timings on a shared CI runner are noisy, so the margin is wide: a number regresses only when
//! it is both `ratio` times the baseline and at least `slack_ms` above it. The check is there to
//! catch a change that makes a script several times slower, not to track small drifts.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

/// Where the baseline lives, relative to the repository root.
pub(super) const DEFAULT_PATH: &str = "docs/perf-baseline.json";

/// A current number regresses when it is above `baseline * RATIO` and `baseline + SLACK_MS`.
pub(super) const RATIO: f64 = 3.0;
pub(super) const SLACK_MS: f64 = 5.0;

/// Percentiles of a series of frames, in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Percentiles {
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
}

/// One script: its whole-frame percentiles and those of each stage (`edit`, `scene`, `render`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct ScriptBaseline {
    pub frame: Percentiles,
    #[serde(default)]
    pub stages: BTreeMap<String, Percentiles>,
}

/// The contents of the baseline file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Baseline {
    /// What the numbers were measured with, for the reader. Not compared.
    pub note: String,
    /// The fraction of the large Map the reduced scripts run on.
    pub scale: f64,
    /// Frames measured per script.
    pub frames: usize,
    pub scripts: BTreeMap<String, ScriptBaseline>,
}

/// One number that is outside the margin.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Regression {
    pub script: String,
    pub metric: String,
    pub baseline: f64,
    pub current: f64,
}

/// Whether `current` is a regression from `baseline` (both in ms).
pub(super) fn exceeds(baseline: f64, current: f64) -> bool {
    current > baseline * RATIO && current > baseline + SLACK_MS
}

/// Compares the whole-frame percentiles of every script in `baseline` with `current`.
/// A script that is in the baseline but was not measured is reported too.
pub(super) fn compare(
    baseline: &Baseline,
    current: &BTreeMap<String, ScriptBaseline>,
) -> Vec<Regression> {
    let mut found = Vec::new();
    for (script, expected) in &baseline.scripts {
        let Some(actual) = current.get(script) else {
            found.push(Regression {
                script: script.clone(),
                metric: "not measured".into(),
                baseline: f64::NAN,
                current: f64::NAN,
            });
            continue;
        };
        for (metric, was, now) in [
            ("p50", expected.frame.p50, actual.frame.p50),
            ("p95", expected.frame.p95, actual.frame.p95),
            ("p99", expected.frame.p99, actual.frame.p99),
        ] {
            if exceeds(was, now) {
                found.push(Regression {
                    script: script.clone(),
                    metric: metric.into(),
                    baseline: was,
                    current: now,
                });
            }
        }
    }
    found
}

/// A readable table of every script: baseline against current, with the regressions marked.
pub(super) fn diff(baseline: &Baseline, current: &BTreeMap<String, ScriptBaseline>) -> String {
    let regressions = compare(baseline, current);
    let mut text = String::new();
    let _ = writeln!(
        text,
        "frame time against {DEFAULT_PATH} (ms; a regression is over {RATIO}x the baseline and \
         over {SLACK_MS} ms more)"
    );
    let _ = writeln!(
        text,
        "{:<16} {:>4} {:>9} {:>9} {:>7}",
        "script", "", "baseline", "current", "change"
    );
    for (script, expected) in &baseline.scripts {
        let Some(actual) = current.get(script) else {
            let _ = writeln!(text, "{script:<16} not measured  <-- REGRESSION");
            continue;
        };
        for (metric, was, now) in [
            ("p50", expected.frame.p50, actual.frame.p50),
            ("p95", expected.frame.p95, actual.frame.p95),
            ("p99", expected.frame.p99, actual.frame.p99),
        ] {
            let change = if was > 0.0 { now / was } else { f64::NAN };
            let flag = if exceeds(was, now) {
                "  <-- REGRESSION"
            } else {
                ""
            };
            let _ = writeln!(
                text,
                "{script:<16} {metric:>4} {was:>9.2} {now:>9.2} {change:>6.2}x{flag}"
            );
        }
        let slow = regressions.iter().any(|r| &r.script == script);
        if slow {
            for (stage, was) in &expected.stages {
                if let Some(now) = actual.stages.get(stage) {
                    let _ = writeln!(
                        text,
                        "  stage {stage:<10} p95 {:>9.2} {:>9.2}",
                        was.p95, now.p95
                    );
                }
            }
        }
    }
    for script in current
        .keys()
        .filter(|s| !baseline.scripts.contains_key(*s))
    {
        let _ = writeln!(text, "{script}: not in the baseline (refresh it to add it)");
    }
    if regressions.is_empty() {
        let _ = writeln!(text, "ok: every script is inside the margin");
    } else {
        let _ = writeln!(
            text,
            "FAILED: {} number(s) regressed. If the slowdown is intended, refresh the baseline \
             (docs/design/performance.md).",
            regressions.len()
        );
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn percentiles(p50: f64, p95: f64, p99: f64) -> Percentiles {
        Percentiles { p50, p95, p99 }
    }

    fn script(p95: f64) -> ScriptBaseline {
        ScriptBaseline {
            frame: percentiles(p95 / 2.0, p95, p95 * 1.2),
            stages: BTreeMap::from([("render".into(), percentiles(1.0, p95 / 2.0, p95 / 2.0))]),
        }
    }

    fn baseline() -> Baseline {
        Baseline {
            note: String::new(),
            scale: 0.05,
            frames: 30,
            scripts: BTreeMap::from([("paint".into(), script(20.0))]),
        }
    }

    fn current(p95: f64) -> BTreeMap<String, ScriptBaseline> {
        BTreeMap::from([("paint".into(), script(p95))])
    }

    #[test]
    fn inside_the_margin_passes() {
        let baseline = baseline();
        // Faster, equal, and noisy-but-under-3x are all fine.
        for p95 in [5.0, 20.0, 59.0] {
            assert!(compare(&baseline, &current(p95)).is_empty(), "{p95}");
        }
        assert!(diff(&baseline, &current(25.0)).contains("ok: every script"));
    }

    #[test]
    fn outside_the_margin_fails_with_a_clear_message() {
        let baseline = baseline();
        let found = compare(&baseline, &current(70.0));
        assert!(found.iter().any(|r| r.metric == "p95" && r.current == 70.0));
        let text = diff(&baseline, &current(70.0));
        assert!(text.contains("paint"), "{text}");
        assert!(text.contains("REGRESSION"), "{text}");
        assert!(text.contains("FAILED"), "{text}");
        assert!(text.contains("stage render"), "{text}");
    }

    #[test]
    fn a_tiny_baseline_needs_the_absolute_slack_too() {
        // 0.5 ms to 2 ms is 4x but only 1.5 ms: noise, not a regression.
        assert!(!exceeds(0.5, 2.0));
        assert!(exceeds(0.5, 6.0));
        // 40 ms to 50 ms is 10 ms more but only 1.25x.
        assert!(!exceeds(40.0, 50.0));
    }

    #[test]
    fn a_script_missing_from_the_run_fails() {
        let found = compare(&baseline(), &BTreeMap::new());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].metric, "not measured");
        assert!(diff(&baseline(), &BTreeMap::new()).contains("not measured"));
    }

    #[test]
    fn the_baseline_round_trips_through_json() {
        let baseline = baseline();
        let json = serde_json::to_string_pretty(&baseline).unwrap();
        assert_eq!(serde_json::from_str::<Baseline>(&json).unwrap(), baseline);
    }
}

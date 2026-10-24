//! Runs every case in `tests/cases.json` through the `spectype` binary and compares the output
//! with what openapi-typescript 7.13.0 produced (`tests/golden/<name>.ts`, or `.error` when the
//! reference tool refused the input).
//!
//! Cases listed in `tests/known-differences.txt` may fail; anything else failing fails the test.
//! An unfiltered run records the pass rate and the failing cases in `tasks/coverage/golden.snap`.
//! `SPECTYPE_GOLDEN=<substring>` limits the run, and `SPECTYPE_GOLDEN_DIFF=1` prints diffs.

#![expect(clippy::print_stderr, reason = "prints the pass rate and diffs")]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[derive(serde::Deserialize)]
struct Case {
    name: String,
    input: String,
    flags: Vec<String>,
}

enum Outcome {
    Pass,
    Fail(String),
}

fn run_case(case: &Case) -> Outcome {
    let root = root();
    let golden = root.join("tests/golden");
    let out = Command::new(env!("CARGO_BIN_EXE_spectype"))
        .current_dir(&root)
        .arg(&case.input)
        .args(&case.flags)
        .output()
        .expect("spawn spectype");
    let expected_error = golden.join(format!("{}.error", case.name));
    if expected_error.exists() {
        return if out.status.success() {
            Outcome::Fail("reference tool errors, spectype succeeded".into())
        } else {
            Outcome::Pass
        };
    }
    let expected = std::fs::read_to_string(golden.join(format!("{}.ts", case.name))).expect("golden file");
    if !out.status.success() {
        return Outcome::Fail(format!("spectype failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let actual = String::from_utf8_lossy(&out.stdout);
    if actual == expected {
        return Outcome::Pass;
    }
    let diff = similar::TextDiff::from_lines(expected.as_str(), &actual)
        .unified_diff()
        .context_radius(2)
        .header("openapi-typescript", "spectype")
        .to_string();
    Outcome::Fail(diff)
}

#[test]
fn golden() {
    let root = root();
    let cases: Vec<Case> =
        serde_json::from_str(&std::fs::read_to_string(root.join("tests/cases.json")).expect("cases.json"))
            .expect("valid cases.json");
    let known: BTreeSet<String> = std::fs::read_to_string(root.join("tests/known-differences.txt"))
        .unwrap_or_default()
        .lines()
        .map(|l| l.split('#').next().unwrap_or_default().trim().to_owned())
        .filter(|l| !l.is_empty())
        .collect();
    let filter = std::env::var("SPECTYPE_GOLDEN").ok();
    let show_diff = std::env::var_os("SPECTYPE_GOLDEN_DIFF").is_some();

    let selected: Vec<&Case> =
        cases.iter().filter(|c| filter.as_ref().is_none_or(|f| c.name.contains(f.as_str()))).collect();
    let results: Vec<(&Case, Outcome)> = std::thread::scope(|s| {
        #[expect(clippy::needless_collect, reason = "spawn every worker before joining any")]
        let handles: Vec<_> = selected
            .chunks(selected.len().div_ceil(8).max(1))
            .map(|chunk| s.spawn(move || chunk.iter().map(|c| (*c, run_case(c))).collect::<Vec<_>>()))
            .collect();
        handles.into_iter().flat_map(|h| h.join().expect("worker")).collect()
    });

    let mut passed = 0u32;
    let mut unexpected = Vec::new();
    let mut fixed = Vec::new();
    let mut failing = BTreeSet::new();
    for (case, outcome) in &results {
        match outcome {
            Outcome::Pass => {
                passed += 1;
                if known.contains(&case.name) {
                    fixed.push(case.name.clone());
                }
            }
            Outcome::Fail(why) => {
                failing.insert(case.name.as_str());
                if show_diff {
                    eprintln!("--- {} ---\n{why}", case.name);
                }
                if !known.contains(&case.name) {
                    unexpected.push(case.name.clone());
                }
            }
        }
    }
    let total = results.len();
    #[expect(clippy::cast_precision_loss, reason = "case counts are far below 2^52")]
    let rate = if total == 0 { 100.0 } else { f64::from(passed) * 100.0 / total as f64 };
    eprintln!("golden: {passed}/{total} byte-identical ({rate:.1}%)");
    if filter.is_none() {
        let mut snap = format!("golden: {passed}/{total} ({rate:.2}%)\n");
        for name in &failing {
            snap.push('\n');
            snap.push_str(name);
        }
        snap.push('\n');
        std::fs::write(root.join("tasks/coverage/golden.snap"), snap).expect("write golden.snap");
    }
    if !fixed.is_empty() {
        eprintln!("now passing, remove from known-differences.txt: {fixed:?}");
    }
    assert!(unexpected.is_empty(), "{} unexpected failures:\n{}", unexpected.len(), unexpected.join("\n"));
}

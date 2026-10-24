//! `redocly.yaml` support: the lint lines, exit status and output openapi-typescript 7.13.0
//! produced for `fixtures/lint/cases.json` (recorded by `tasks/reference/lint.mjs`), and the
//! `apis` output mode.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn run_cli(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_spectype")).current_dir(dir).args(args).output().expect("spawn spectype")
}

fn lint_lines(stderr: &[u8]) -> String {
    String::from_utf8_lossy(stderr)
        .lines()
        .filter(|l| l.starts_with(" ⚠  ") || l.starts_with(" ✘  "))
        .flat_map(|l| [l, "\n"])
        .collect()
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("spectype-lint-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

#[derive(serde::Deserialize)]
struct Case {
    name: String,
    input: String,
    config: String,
    #[serde(default)]
    stdout: bool,
}

#[test]
fn matches_openapi_typescript() {
    let root = root();
    let cases: Vec<Case> =
        serde_json::from_str(&std::fs::read_to_string(root.join("fixtures/lint/cases.json")).expect("cases.json"))
            .expect("valid cases.json");
    let out_dir = temp_dir("cases");
    let mut failures = Vec::new();
    for case in &cases {
        let expected_dir = root.join("fixtures/lint/expected");
        let expected = std::fs::read_to_string(expected_dir.join(format!("{}.txt", case.name))).expect("expected");
        let out_file = out_dir.join(format!("{}.ts", case.name));
        let out_path = out_file.to_string_lossy().into_owned();
        let mut args = vec![case.input.as_str(), "--redocly", case.config.as_str()];
        if !case.stdout {
            args.extend(["-o", out_path.as_str()]);
        }
        let out = run_cli(&root, &args);
        let status = i32::from(!out.status.success());
        let actual = format!("exit {status}\n{}", lint_lines(&out.stderr));
        if actual != expected {
            failures.push(format!("--- {}\nexpected:\n{expected}actual:\n{actual}", case.name));
            continue;
        }
        let Ok(expected_ts) = std::fs::read_to_string(expected_dir.join(format!("{}.ts", case.name))) else { continue };
        let actual_ts = if case.stdout {
            String::from_utf8_lossy(&out.stdout).into_owned()
        } else {
            std::fs::read_to_string(&out_file).unwrap_or_default()
        };
        if actual_ts != expected_ts {
            failures.push(format!("--- {}: generated types differ", case.name));
        }
    }
    let _ = std::fs::remove_dir_all(&out_dir);
    assert!(failures.is_empty(), "{} of {} cases differ:\n{}", failures.len(), cases.len(), failures.join("\n"));
}

const PET: &str = "openapi: 3.0.0\ninfo: { title: t, version: '1' }\npaths:\n  /pets:\n    get:\n      responses:\n        '200': { description: ok }\n";

#[test]
fn apis_mode_writes_every_api_relative_to_the_config() {
    let dir = temp_dir("apis");
    std::fs::create_dir_all(dir.join("conf/specs")).unwrap();
    std::fs::write(dir.join("conf/specs/a.yaml"), PET).unwrap();
    std::fs::write(dir.join("conf/specs/b.yaml"), PET.replace("/pets", "/users/")).unwrap();
    std::fs::write(
        dir.join("conf/redocly.yaml"),
        "rules:\n  no-path-trailing-slash: warn\napis:\n  a@v1:\n    root: ./specs/a.yaml\n    x-openapi-ts:\n      output: ./out/a.ts\n  b@v1:\n    root: specs/b.yaml\n    x-openapi-ts:\n      output: out/nested/b.ts\n",
    )
    .unwrap();

    let out = run_cli(&dir, &["--redocly", "conf", "--enum-values"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("🚀 a@v1 → ./out/a.ts"), "{stderr}");
    assert!(stderr.contains("🚀 b@v1 → out/nested/b.ts"), "{stderr}");
    assert_eq!(lint_lines(&out.stderr), " ⚠  `/users/` should not have a trailing slash. at #/paths/~1users~1\n");
    let single = run_cli(&dir, &["conf/specs/a.yaml", "--enum-values"]);
    assert_eq!(std::fs::read_to_string(dir.join("conf/out/a.ts")).unwrap(), String::from_utf8_lossy(&single.stdout));
    assert!(dir.join("conf/out/nested/b.ts").exists());

    let check = run_cli(&dir, &["-c", "conf/redocly.yaml", "--check", "--enum-values"]);
    assert!(check.status.success(), "{}", String::from_utf8_lossy(&check.stderr));
    std::fs::write(dir.join("conf/specs/a.yaml"), PET.replace("/pets", "/owners")).unwrap();
    let stale = run_cli(&dir, &["-c", "conf/redocly.yaml", "--check", "--enum-values"]);
    assert!(!stale.status.success());
    assert!(String::from_utf8_lossy(&stale.stderr).contains("Generated types are not up-to-date!"));

    let both = run_cli(&dir.join("conf"), &["specs/a.yaml"]);
    assert!(
        String::from_utf8_lossy(&both.stderr)
            .contains(" ⚠  APIs are specified both in Redocly Config and CLI argument. Only using Redocly config.")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn config_errors() {
    let dir = temp_dir("errors");
    let missing = run_cli(&dir, &["--redocly", "nope.yaml"]);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("Redocly config not found at: nope.yaml"));

    std::fs::write(dir.join("redocly.yaml"), "apis:\n  main:\n    root: ./a.yaml\n").unwrap();
    let no_output = run_cli(&dir, &[]);
    assert!(!no_output.status.success());
    assert!(String::from_utf8_lossy(&no_output.stderr).contains(
        "API main is missing an `x-openapi-ts.output` key. See https://openapi-ts.dev/cli/#multiple-schemas."
    ));

    std::fs::write(dir.join("redocly.yaml"), "extends: [nope]\n").unwrap();
    let preset = run_cli(&dir, &["a.yaml"]);
    assert!(String::from_utf8_lossy(&preset.stderr).contains("Invalid config nope: there is no such built-in config."));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn no_config_runs_no_rules() {
    let dir = temp_dir("none");
    std::fs::write(dir.join("a.yaml"), PET.replace("/pets", "/pets/")).unwrap();
    let out = run_cli(&dir, &["a.yaml", "-o", "a.ts"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(lint_lines(&out.stderr), "");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lint_ignore_file_with_fix() {
    let dir = temp_dir("ignore");
    std::fs::write(dir.join("redocly.yaml"), "rules:\n  operation-operationId: error\n").unwrap();
    std::fs::write(
        dir.join("spec.yaml"),
        "openapi: 3.0.0\ninfo: {title: t, version: '1'}\npaths:\n  /x:\n    get:\n      responses:\n        '200': {description: ok}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join(".redocly.lint-ignore.yaml"),
        "spec.yaml:\n  operation-operationId:\n    - '#/paths/~1x/get/operationId'\n",
    )
    .unwrap();
    let plain = run_cli(&dir, &["spec.yaml", "-o", "out.ts"]);
    assert!(!plain.status.success());
    let fixed = run_cli(&dir, &["spec.yaml", "-o", "out.ts", "--fix=lint-ignore"]);
    assert!(fixed.status.success(), "{}", String::from_utf8_lossy(&fixed.stderr));
    assert_eq!(lint_lines(&fixed.stderr), "");
    let _ = std::fs::remove_dir_all(&dir);
}

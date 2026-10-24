//! Snapshots of each opt-in `--fix`, and a check that it changes the output it is meant to.

use std::path::Path;
use std::process::Command;

fn run(spec: &str, flags: &[&str]) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = Command::new(env!("CARGO_BIN_EXE_spectype"))
        .current_dir(&root)
        .arg(format!("fixtures/fixes/{spec}.json"))
        .args(flags)
        .output()
        .expect("spawn spectype");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).expect("utf-8 output")
}

fn check(fix: &str, flags: &[&str]) {
    let plain = run(fix, flags);
    let fix_flag = format!("--fix={fix}");
    let fixed = run(fix, &[flags, &[fix_flag.as_str()]].concat());
    assert_ne!(plain, fixed, "--fix={fix} changed nothing");
    insta::assert_snapshot!(fix, fixed);
}

#[test]
fn array_length() {
    check("array-length", &["--array-length"]);
}

#[test]
fn enum_values_type() {
    check("enum-values-type", &["--enum-values"]);
}

#[test]
fn root_types_names() {
    check("root-types-names", &["--root-types", "--root-types-no-schema-prefix", "--root-types-keep-casing"]);
}

#[test]
fn enum_names() {
    check("enum-names", &["--enum"]);
}

#[test]
fn operation_id_refs() {
    check("operation-id-refs", &[]);
}

#[test]
fn default_non_nullable_scope() {
    check("default-non-nullable-scope", &[]);
}

#[test]
fn const_items() {
    check("const-items", &[]);
}

#[test]
fn allof_empty_objects() {
    check("allof-empty-objects", &[]);
}

#[test]
fn discriminator_oneof_base() {
    check("discriminator-oneof-base", &[]);
}

#[test]
fn allof_required() {
    check("allof-required", &[]);
}

#[test]
fn nested_discriminators() {
    check("nested-discriminators", &[]);
}

#[test]
fn discriminator_declared_values() {
    check("discriminator-declared-values", &[]);
}

#[test]
fn unknown_fix_is_rejected() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = Command::new(env!("CARGO_BIN_EXE_spectype"))
        .current_dir(root)
        .args(["fixtures/fixes/const-items.json", "--fix=nope"])
        .output()
        .expect("spawn spectype");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown fix \"nope\""));
}

#![expect(clippy::print_stdout, reason = "prints the snapshot")]

use rustc_hash::FxHashMap;
use std::alloc::System;
use std::fmt::Write as _;
use std::path::Path;

use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};

use spectype_codegen::{FsHost, LoadedDocument, Options, scan_discriminators, transform_document, ts};
use spectype_document::parse::{Format, parse_document};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

const FIXTURES: [&str; 4] = [
    "fixtures/ots/examples/github-api.yaml",
    "fixtures/ots/examples/stripe-api.yaml",
    "fixtures/ots/examples/octokit-ghes-3.6-diff-to-api.json",
    "fixtures/ots/examples/digital-ocean-api/DigitalOcean-public.v2.yaml",
];

const VIRTUAL_ROOT: &str = "/spectype-allocs";

fn read_tree(dir: &Path, virtual_dir: &str, files: &mut FxHashMap<String, String>) {
    for entry in std::fs::read_dir(dir).expect("fixture dir") {
        let path = entry.expect("dir entry").path();
        let name = format!("{virtual_dir}/{}", path.file_name().unwrap_or_default().to_string_lossy());
        if path.is_dir() {
            read_tree(&path, &name, files);
        } else if let Ok(text) = std::fs::read_to_string(&path) {
            files.insert(name, text);
        }
    }
}

fn measure<T>(out: &mut String, fixture: &str, phase: &str, f: impl FnOnce() -> T) -> T {
    let region = Region::new(GLOBAL);
    let value = f();
    let s = region.change();
    let _ = writeln!(out, "{fixture:<34} | {phase:<14} | {:>10} | {:>10}", s.allocations, s.reallocations);
    value
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = FxHashMap::default();
    for fixture in FIXTURES {
        let dir = Path::new(fixture).parent().unwrap_or_else(|| Path::new(""));
        read_tree(&root.join(dir), &format!("{VIRTUAL_ROOT}/{}", dir.display()), &mut files);
    }
    std::env::set_current_dir("/").expect("chdir /");

    let mut out = format!("{:<34} | {:<14} | {:>10} | {:>10}\n", "fixture", "phase", "allocs", "reallocs");
    let options = Options::default();
    for fixture in FIXTURES {
        let name = Path::new(fixture).file_name().unwrap_or_default().to_string_lossy();
        let location = format!("{VIRTUAL_ROOT}/{fixture}");
        let source = &files[&location];
        let mut doc = measure(&mut out, &name, "parse", || parse_document(source, Format::Yaml).expect("parse"));
        let mut load = |r: &str| Ok(files.get(r).map(|text| LoadedDocument { text: text.clone(), mime_type: None }));
        measure(&mut out, &name, "bundle", || {
            spectype_redocly::bundle(&mut doc, Some(&location), &mut load, &options.lint).expect("bundle")
        });
        let (discriminators, _) =
            measure(&mut out, &name, "discriminators", || scan_discriminators(&mut doc, options.fix));
        let (stmts, _) = measure(&mut out, &name, "transform", || {
            transform_document(&doc, &options, &discriminators, &mut FsHost).expect("transform")
        });
        measure(&mut out, &name, "print", || ts::print(&stmts));
    }
    let snap = Path::new(env!("CARGO_MANIFEST_DIR")).join("allocs.snap");
    std::fs::write(&snap, &out).expect("write snapshot");
    print!("{out}");
}

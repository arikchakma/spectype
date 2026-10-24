//! Prints each listed YAML file as canonical JSON (see `tasks/yamldiff/run.mjs`), one line per
//! file: `OK <json>`, `ERR <message>`, prefixed with `FALLBACK ` when the fast loader defers.

use std::io::{BufRead, Write as _};

use spectype_document::parse::yaml_internals::{canonical, fast, legacy};

fn main() {
    let use_legacy = std::env::args().any(|a| a == "--legacy");
    let list = std::env::args().skip(1).find(|a| a != "--legacy").expect("usage: yaml_canon <list-file> [--legacy]");
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());
    for path in std::io::BufReader::new(std::fs::File::open(list).expect("open list")).lines() {
        let bytes = std::fs::read(path.expect("read list")).expect("read case");
        let Ok(text) = String::from_utf8(bytes) else {
            let _ = writeln!(out, "ERR not utf-8");
            continue;
        };
        let (prefix, result) = match (use_legacy, fast(&text)) {
            (false, Some(r)) => ("", r),
            (false, None) => ("FALLBACK ", legacy(&text)),
            (true, _) => ("", legacy(&text)),
        };
        let line = match result {
            Ok(v) => format!("{prefix}OK {}", canonical(&v)),
            Err(e) => format!("{prefix}ERR {}", e.to_string().replace(['\n', '\r'], " ")),
        };
        let _ = writeln!(out, "{line}");
    }
}

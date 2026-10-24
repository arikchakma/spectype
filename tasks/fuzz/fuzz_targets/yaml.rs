#![no_main]

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Mutex, OnceLock};

use libfuzzer_sys::fuzz_target;
use spectype_core::parse::yaml_internals::{canonical, fast, legacy};

// The js-yaml port against the yaml-rust2 loader. yaml-rust2 follows the YAML spec rather than
// js-yaml, so every disagreement goes to js-yaml itself (tasks/yamldiff/oracle.mjs): the port
// must match js-yaml whenever it returns Ok, and must not reject what js-yaml accepts except
// for its documented limits (aliases into unfinished collections, alias-expansion cap).
// Results holding U+10F000–U+10F7FF are skipped: `Value` stores lone surrogates there.
// `YAML_FUZZ_ORACLE_ALWAYS=1` checks every input against js-yaml (slower).
fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else { return };
    let Some(ours) = fast(text) else { return };
    let theirs = legacy(text);
    let agree = match (&ours, &theirs) {
        (Ok(a), Ok(b)) => canonical(a) == canonical(b),
        (Err(_), Err(_)) => true,
        _ => false,
    };
    if agree && std::env::var_os("YAML_FUZZ_ORACLE_ALWAYS").is_none() {
        return;
    }
    let js = oracle(text);
    match &ours {
        Ok(_) if js == "UNREPRESENTABLE sentinel" => {}
        Ok(v) => assert_eq!(js, format!("OK {}", canonical(v)), "port disagrees with js-yaml on {text:?}"),
        Err(e) => {
            let e = e.to_string();
            let limit = e.contains("is not complete yet") || e.contains("expand to too many nodes");
            assert!(limit || !js.starts_with("OK"), "port rejects what js-yaml accepts ({e}) on {text:?}");
        }
    }
});

struct Oracle {
    _child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

fn oracle(text: &str) -> String {
    static ORACLE: OnceLock<Mutex<Oracle>> = OnceLock::new();
    let oracle = ORACLE.get_or_init(|| {
        let script = concat!(env!("CARGO_MANIFEST_DIR"), "/../yamldiff/oracle.mjs");
        let mut child = Command::new("node")
            .arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("start node oracle");
        let stdin = child.stdin.take().expect("oracle stdin");
        let stdout = BufReader::new(child.stdout.take().expect("oracle stdout"));
        Mutex::new(Oracle { _child: child, stdin, stdout })
    });
    let mut o = oracle.lock().expect("oracle lock");
    let len = u32::try_from(text.len()).expect("input fits u32");
    o.stdin.write_all(&len.to_le_bytes()).expect("write oracle");
    o.stdin.write_all(text.as_bytes()).expect("write oracle");
    o.stdin.flush().expect("flush oracle");
    let mut line = String::new();
    o.stdout.read_line(&mut line).expect("read oracle");
    line.trim_end_matches('\n').to_owned()
}

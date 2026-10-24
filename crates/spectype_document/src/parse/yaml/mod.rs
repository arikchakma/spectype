//! YAML loading with the semantics of Redocly's js-yaml setup (`JSON_SCHEMA` plus merge keys):
//! `yes`/`no`/timestamps stay strings, `0o17` and `0x1A` are numbers, and non-finite floats
//! other than `.inf`/`.nan` stay strings.

use crate::value::Value;
use crate::{Error, Result};

mod legacy;
mod loader;

/// The two loaders and a canonical dump, for the differential tests and fuzzers.
pub mod internals {
    use std::fmt::Write as _;

    use crate::value::{js_number, lone_surrogate};
    use crate::{Error, Result, Value};

    /// The js-yaml port alone; `None` where it defers to yaml-rust2.
    #[must_use]
    pub fn fast(source: &str) -> Option<Result<Value>> {
        match super::loader::load(source) {
            Ok(v) => Some(Ok(v)),
            Err(super::loader::Fail::Error(e)) => Some(Err(Error::Parse(e))),
            Err(super::loader::Fail::Fallback) => None,
        }
    }

    /// The yaml-rust2 event loader.
    pub fn legacy(source: &str) -> Result<Value> {
        super::legacy::parse(source)
    }

    /// JSON with `-0`/`NaN`/`Infinity` spelled out and every non-ASCII UTF-16 unit escaped, so
    /// it compares byte-for-byte with `tasks/yamldiff/jsyaml.mjs`.
    #[must_use]
    pub fn canonical(v: &Value) -> String {
        let mut out = String::new();
        write(v, &mut out);
        out
    }

    fn write(v: &Value, out: &mut String) {
        match v {
            Value::Null => out.push_str("null"),
            Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Value::Number(n) if n.is_nan() => out.push_str("NaN"),
            Value::Number(n) if n.is_infinite() => out.push_str(if *n > 0.0 { "Infinity" } else { "-Infinity" }),
            Value::Number(n) if *n == 0.0 && n.is_sign_negative() => out.push_str("-0"),
            Value::Number(n) => out.push_str(&js_number(*n)),
            Value::String(s) => quote(s, out),
            Value::Array(a) => {
                out.push('[');
                for (i, item) in a.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write(item, out);
                }
                out.push(']');
            }
            Value::Object(m) => {
                out.push('{');
                for (i, (k, item)) in m.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    quote(k, out);
                    out.push(':');
                    write(item, out);
                }
                out.push('}');
            }
        }
    }

    fn quote(s: &str, out: &mut String) {
        out.push('"');
        for c in s.chars() {
            let mut buf = [0; 2];
            let units: &[u16] = match lone_surrogate(c) {
                Some(u) => &[u],
                None => c.encode_utf16(&mut buf),
            };
            for &u in units {
                match u {
                    0x22 => out.push_str("\\\""),
                    0x5c => out.push_str("\\\\"),
                    0x20..=0x7e => out.push(char::from(u8::try_from(u).unwrap_or(b'?'))),
                    _ => {
                        let _ = write!(out, "\\u{u:04x}");
                    }
                }
            }
        }
        out.push('"');
    }
}

/// Loads with the js-yaml port, falling back to yaml-rust2 for the tags it hands back.
pub fn parse(source: &str) -> Result<Value> {
    match loader::load(source) {
        Ok(v) => Ok(v),
        Err(loader::Fail::Error(e)) => Err(Error::Parse(format!("invalid YAML: {e}"))),
        Err(loader::Fail::Fallback) => legacy::parse(source),
    }
}

/// The implicit `JSON_SCHEMA` types (null, bool, int, float) for a non-empty plain scalar.
pub(super) fn resolve_implicit(s: &str) -> Option<Value> {
    match s.as_bytes().first()? {
        b'0'..=b'9' | b'-' | b'+' | b'.' => resolve_int(s).or_else(|| resolve_float(s)).map(Value::Number),
        b'~' if s.len() == 1 => Some(Value::Null),
        b'n' | b'N' if matches!(s, "null" | "Null" | "NULL") => Some(Value::Null),
        b't' | b'T' if matches!(s, "true" | "True" | "TRUE") => Some(Value::Bool(true)),
        b'f' | b'F' if matches!(s, "false" | "False" | "FALSE") => Some(Value::Bool(false)),
        _ => None,
    }
}

pub(super) fn resolve_plain(s: &str) -> Value {
    if s.is_empty() {
        return Value::Null;
    }
    resolve_implicit(s).unwrap_or_else(|| Value::String(s.to_owned()))
}

pub(super) fn resolve_int(s: &str) -> Option<f64> {
    let (sign, body) = match s.as_bytes().first()? {
        b'-' => (-1.0, &s[1..]),
        b'+' => (1.0, &s[1..]),
        _ => (1.0, s),
    };
    if body == "0" {
        return Some(0.0);
    }
    let (radix, digits) = if let Some(d) = body.strip_prefix("0b") {
        (2, d)
    } else if let Some(d) = body.strip_prefix("0x") {
        (16, d)
    } else if let Some(d) = body.strip_prefix("0o") {
        (8, d)
    } else {
        (10, body)
    };
    if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
        return None;
    }
    let n = if radix == 10 { digits.parse::<f64>().ok()? } else { parse_pow2_radix(digits, radix) };
    n.is_finite().then_some(sign * n)
}

/// `parseInt` for radix 2/8/16, which V8 rounds correctly (half to even) past 2^53.
fn parse_pow2_radix(digits: &str, radix: u32) -> f64 {
    let bits = radix.trailing_zeros();
    let mut acc: u128 = 0;
    let mut dropped = 0i32;
    let mut sticky = false;
    for d in digits.chars().filter_map(|c| c.to_digit(radix)) {
        if acc >> 120 == 0 {
            acc = (acc << bits) | u128::from(d);
        } else {
            dropped += i32::try_from(bits).unwrap_or(4);
            sticky |= d != 0;
        }
    }
    #[expect(clippy::cast_precision_loss, reason = "JavaScript numbers are f64, so the rounding matches")]
    let mantissa = (acc | u128::from(sticky)) as f64;
    mantissa * 2f64.powi(dropped)
}

pub(super) fn resolve_float(s: &str) -> Option<f64> {
    match s {
        ".inf" | ".Inf" | ".INF" | "+.inf" | "+.Inf" | "+.INF" => return Some(f64::INFINITY),
        "-.inf" | "-.Inf" | "-.INF" => return Some(f64::NEG_INFINITY),
        ".nan" | ".NaN" | ".NAN" => return Some(f64::NAN),
        _ => {}
    }
    let b = s.as_bytes();
    let mut i = 0;
    if matches!(b.first(), Some(b'-' | b'+')) {
        i += 1;
    }
    let int_start = i;
    while b.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
    }
    let has_int = i > int_start;
    if has_int {
        if b.get(i) == Some(&b'.') {
            i += 1;
            while b.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
        }
    } else {
        if int_start != 0 || b.get(i) != Some(&b'.') {
            return None;
        }
        i += 1;
        let frac = i;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if i == frac {
            return None;
        }
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(b.get(i), Some(b'-' | b'+')) {
            i += 1;
        }
        let exp = i;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if i == exp {
            return None;
        }
    }
    if i != b.len() {
        return None;
    }
    let n = crate::value::js_string_to_number(s.strip_suffix('.').unwrap_or(s));
    n.is_finite().then_some(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(v: &Value) -> Vec<(String, Value)> {
        v.as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    #[test]
    fn scalars_match_js_yaml() {
        let v =
            parse("a: 1e400\nb: 0o17\nc: yes\nd: ~\ne: 1_000\nf: .5\ng: 5.\nh: 2001-01-01\ni: '1'\nj: 010\nk: -.inf\n")
                .unwrap();
        assert_eq!(v.get("a").and_then(Value::as_str), Some("1e400"));
        assert_eq!(v.get("b"), Some(&Value::Number(15.0)));
        assert_eq!(v.get("c").and_then(Value::as_str), Some("yes"));
        assert_eq!(v.get("d"), Some(&Value::Null));
        assert_eq!(v.get("e").and_then(Value::as_str), Some("1_000"));
        assert_eq!(v.get("f"), Some(&Value::Number(0.5)));
        assert_eq!(v.get("g"), Some(&Value::Number(5.0)));
        assert_eq!(v.get("h").and_then(Value::as_str), Some("2001-01-01"));
        assert_eq!(v.get("i").and_then(Value::as_str), Some("1"));
        assert_eq!(v.get("j"), Some(&Value::Number(10.0)));
        assert_eq!(v.get("k"), Some(&Value::Number(f64::NEG_INFINITY)));
        assert_eq!(resolve_int("0x20000000000001"), Some(9_007_199_254_740_992.0));
        assert_eq!(resolve_int("0x20000000000003"), Some(9_007_199_254_740_996.0));
    }

    #[test]
    fn merge_keys_and_duplicates() {
        let v = parse("base: &b {x: 1, y: 2}\nm:\n  <<: *b\n  y: 3\n").unwrap();
        assert_eq!(keys(v.get("m").unwrap()), [("x".into(), Value::Number(1.0)), ("y".into(), Value::Number(3.0))]);
        assert!(parse("a: 1\na: 2\n").is_err());
        assert!(parse("m:\n  <<: {a: 1}\n  a: 2\n  a: 3\n").is_err());
    }

    #[test]
    fn block_scalars_and_quoting() {
        let v =
            parse("a: |-\n  x\n\n  y\n\nb: >\n  p\n  q\n\n   r\nc: \"\\ud83d\\ude00\\ud800\"\nd: 'it''s'\n").unwrap();
        assert_eq!(v.get("a").and_then(Value::as_str), Some("x\n\ny"));
        assert_eq!(v.get("b").and_then(Value::as_str), Some("p q\n\n r\n"));
        assert_eq!(v.get("c").map(|c| c.json_stringify(0)).as_deref(), Some("\"😀\\ud800\""));
        assert_eq!(v.get("d").and_then(Value::as_str), Some("it's"));
    }

    #[test]
    fn rejects_what_js_yaml_rejects() {
        for bad in ["a: 1\n---\nb: 2\n", "a:\n\tb: 1\n", "a: !foo x\n", "a: \"\\q\"\n", "a: &x [*x]\n", "a\0: 1"] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
        assert_eq!(parse("a: 1\n...\n").unwrap().get("a"), Some(&Value::Number(1.0)));
        assert!(matches!(loader::load("a: !!binary aGk=\n"), Err(loader::Fail::Fallback)));
    }

    #[test]
    fn nesting_cap_is_an_error_not_a_stack_overflow() {
        // ~1 KiB of stack per level in release; unoptimized frames need more than a test thread has.
        let nested = |n: usize| (0..n).map(|i| " ".repeat(i) + "a:\n").collect::<String>();
        let run = move || {
            let deep = |n: usize| format!("{}{}", "[".repeat(n), "]".repeat(n));
            assert!(parse(&deep(999)).is_ok());
            assert!(parse(&deep(1001)).is_err());
            assert!(parse(&nested(999)).is_ok());
            assert!(parse(&nested(1001)).is_err());
        };
        std::thread::Builder::new().stack_size(64 << 20).spawn(run).unwrap().join().unwrap();
    }
}

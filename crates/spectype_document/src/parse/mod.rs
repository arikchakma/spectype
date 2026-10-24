//! Reading documents and the validation openapi-typescript gets from Redocly.

use crate::value::{Value, js_string_to_number};
use crate::{Error, Result};

mod json;
mod yaml;

#[doc(hidden)]
pub use yaml::internals as yaml_internals;

/// How a source document is read.
///
/// openapi-typescript reads files with js-yaml (even `.json`
/// files) but strings and stdin starting with `{` with `JSON.parse`, and the two disagree on
/// duplicate keys and out-of-range numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// `JSON.parse` semantics.
    Json,
    /// js-yaml semantics (also used for JSON files).
    Yaml,
    /// `Json` when the trimmed text starts with `{`, `Yaml` otherwise, as for stdin.
    Auto,
}

/// Parses a JSON or YAML document into a [`Value`].
pub fn parse_document(source: &str, format: Format) -> Result<Value> {
    let trimmed = crate::value::js_trim(source);
    let format = match format {
        Format::Auto if trimmed.starts_with('{') => Format::Json,
        Format::Auto => Format::Yaml,
        f => f,
    };
    if format == Format::Json {
        json::parse(trimmed, json::Mode::JsonParse)
    } else {
        let text = source.strip_prefix('\u{feff}').unwrap_or(source);
        if !trimmed.starts_with(['{', '[']) {
            return yaml::parse(text);
        }
        json::parse(trimmed, json::Mode::Yaml).or_else(|json_error| yaml::parse(text).map_err(|_| json_error))
    }
}

fn leading_spaces(bytes: &[u8]) -> usize {
    let wide = bytes.chunks_exact(8).take_while(|chunk| *chunk == b"        ").count() * 8;
    wide + bytes[wide..].iter().take_while(|&&b| b == b' ').count()
}

/// Rejects documents openapi-typescript refuses: Swagger 2 and anything not `openapi: 3.x`.
pub fn check_version(doc: &Value) -> Result<()> {
    if doc.get("swagger").is_some_and(Value::truthy) {
        return Err(Error::Unsupported("Unsupported Swagger version: 2.x. Use OpenAPI 3.x instead.".into()));
    }
    let Some(openapi) = doc.get("openapi").filter(|v| v.truthy()) else {
        return Err(Error::Unsupported("Unsupported schema format, expected `openapi: 3.x`".into()));
    };
    if !matches!(openapi, Value::String(_)) {
        let kind = if matches!(openapi, Value::Number(_)) { "number" } else { "object" };
        return Err(Error::Unsupported(format!("Invalid OpenAPI version: should be a string but got \"{kind}\"")));
    }
    let version = parse_float(&openapi.js_string());
    if version.is_nan() || !(3.0..4.0).contains(&version) {
        return Err(Error::Unsupported(format!("Unsupported OpenAPI version: {}", openapi.js_string())));
    }
    Ok(())
}

/// `Number.parseFloat`.
fn parse_float(s: &str) -> f64 {
    let t = crate::value::js_trim(s);
    let end = t
        .char_indices()
        .scan((false, false, false), |(dot, exp, digit), (i, c)| {
            let ok = match c {
                '0'..='9' => {
                    *digit = true;
                    true
                }
                '.' if !*dot && !*exp => {
                    *dot = true;
                    true
                }
                'e' | 'E' if *digit && !*exp => {
                    *exp = true;
                    true
                }
                '+' | '-' if i == 0 => true,
                _ => false,
            };
            ok.then_some(i + c.len_utf8())
        })
        .last()
        .unwrap_or(0);
    let mut candidate = &t[..end];
    while !candidate.is_empty() {
        let n = js_string_to_number(candidate);
        if !n.is_nan() {
            return n;
        }
        candidate = &candidate[..candidate.len() - 1];
    }
    if t.starts_with("Infinity") || t.starts_with("+Infinity") {
        return f64::INFINITY;
    }
    if t.starts_with("-Infinity") {
        return f64::NEG_INFINITY;
    }
    f64::NAN
}

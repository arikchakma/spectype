//! A JSON value with JavaScript semantics.
//!
//! openapi-typescript runs on plain JS objects, so its output depends on JS rules: integer-like
//! keys enumerate first in ascending order, every number is an `f64`, and `String(x)` /
//! `JSON.stringify(x)` decide how literals print. This module copies those rules.

use std::fmt::{self, Write as _};
use std::hash::BuildHasher as _;

use hashbrown::HashTable;
use rustc_hash::FxBuildHasher;
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};

/// A parsed `OpenAPI` document node.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Self>),
    Object(Map),
}

/// An object whose iteration order matches JavaScript's `Object.keys`.
pub type Map = JsMap<Value>;

/// A string-keyed map that enumerates like a JavaScript object.
#[derive(Clone)]
pub struct JsMap<V> {
    entries: Vec<(String, V)>,
    index: Option<Box<HashTable<usize>>>,
    index_keys: usize,
}

const LINEAR_SCAN_MAX: usize = 8;

impl<V> Default for JsMap<V> {
    fn default() -> Self {
        Self { entries: Vec::new(), index: None, index_keys: 0 }
    }
}

impl<V: fmt::Debug> fmt::Debug for JsMap<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

impl<V: PartialEq> PartialEq for JsMap<V> {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().all(|(k, v)| other.get(k) == Some(v))
    }
}

fn hash(key: &str) -> u64 {
    FxBuildHasher.hash_one(key)
}

/// Whether `key` is a canonical array index, which JS enumerates before other keys.
fn is_array_index(key: &str) -> bool {
    match key.as_bytes() {
        [b'0'] => true,
        [b'1'..=b'9', rest @ ..] if rest.iter().all(u8::is_ascii_digit) && key.len() <= 10 => {
            key.parse::<u64>().is_ok_and(|n| n < 4_294_967_295)
        }
        _ => false,
    }
}

impl<V> JsMap<V> {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn position(&self, key: &str) -> Option<usize> {
        match &self.index {
            Some(index) => index.find(hash(key), |&i| self.entries[i].0 == key).copied(),
            None => self.entries.iter().position(|(k, _)| k == key),
        }
    }

    pub fn insert(&mut self, key: String, value: V) {
        if let Some(i) = self.position(&key) {
            self.entries[i].1 = value;
            return;
        }
        let at = if is_array_index(&key) {
            let n: u64 = key.parse().unwrap_or(0);
            let pos = self.entries[..self.index_keys].iter().position(|(k, _)| k.parse::<u64>().unwrap_or(0) > n);
            self.index_keys += 1;
            pos.unwrap_or(self.index_keys - 1)
        } else {
            self.entries.len()
        };
        self.entries.insert(at, (key, value));
        let entries = &self.entries;
        match &mut self.index {
            Some(index) => {
                if at + 1 < entries.len() {
                    index.iter_mut().filter(|i| **i >= at).for_each(|i| *i += 1);
                }
                index.insert_unique(hash(&entries[at].0), at, |&i| hash(&entries[i].0));
            }
            None if entries.len() > LINEAR_SCAN_MAX => {
                let mut index = HashTable::with_capacity(entries.len() * 2);
                for (i, (k, _)) in entries.iter().enumerate() {
                    index.insert_unique(hash(k), i, |&j| hash(&entries[j].0));
                }
                self.index = Some(Box::new(index));
            }
            None => {}
        }
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<&V> {
        self.position(key).map(|i| &self.entries[i].1)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut V> {
        self.position(key).map(|i| &mut self.entries[i].1)
    }

    #[must_use]
    pub fn contains_key(&self, key: &str) -> bool {
        self.position(key).is_some()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &V)> {
        self.entries.iter().map(|(k, v)| (k, v))
    }

    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.entries.iter().map(|(k, _)| k)
    }

    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.entries.iter().map(|(_, v)| v)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn into_values(self) -> impl Iterator<Item = V> {
        self.entries.into_iter().map(|(_, v)| v)
    }

    #[must_use]
    pub fn map_values<U>(self, mut f: impl FnMut(V) -> U) -> JsMap<U> {
        let entries = self.entries.into_iter().map(|(k, v)| (k, f(v))).collect();
        JsMap { entries, index: self.index, index_keys: self.index_keys }
    }

    pub fn try_map_values<U, E>(self, mut f: impl FnMut(V) -> Result<U, E>) -> Result<JsMap<U>, E> {
        let entries = self.entries.into_iter().map(|(k, v)| Ok((k, f(v)?))).collect::<Result<_, E>>()?;
        Ok(JsMap { entries, index: self.index, index_keys: self.index_keys })
    }
}

impl<V> FromIterator<(String, V)> for JsMap<V> {
    fn from_iter<T: IntoIterator<Item = (String, V)>>(iter: T) -> Self {
        let mut map = Self::new();
        for (k, v) in iter {
            map.insert(k, v);
        }
        map
    }
}

impl Value {
    #[must_use]
    pub fn as_object(&self) -> Option<&Map> {
        match self {
            Self::Object(m) => Some(m),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_array(&self) -> Option<&Vec<Self>> {
        match self {
            Self::Array(a) => Some(a),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }

    #[must_use]
    pub fn is_object(&self) -> bool {
        matches!(self, Self::Object(_))
    }

    /// `obj[key]` on a plain object; arrays and primitives yield nothing.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Self> {
        self.as_object().and_then(|m| m.get(key))
    }

    /// `key in obj` for an object node.
    #[must_use]
    pub fn has(&self, key: &str) -> bool {
        self.as_object().is_some_and(|m| m.contains_key(key))
    }

    /// `obj[key]` for objects and arrays (numeric string indexes).
    #[must_use]
    pub fn index(&self, key: &str) -> Option<&Self> {
        match self {
            Self::Object(m) => m.get(key),
            Self::Array(a) => key.parse::<usize>().ok().filter(|_| is_array_index(key)).and_then(|i| a.get(i)),
            _ => None,
        }
    }

    pub fn index_mut(&mut self, key: &str) -> Option<&mut Self> {
        match self {
            Self::Object(m) => m.get_mut(key),
            Self::Array(a) => {
                if is_array_index(key) {
                    key.parse::<usize>().ok().and_then(|i| a.get_mut(i))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// JavaScript truthiness.
    #[must_use]
    pub fn truthy(&self) -> bool {
        match self {
            Self::Null => false,
            Self::Bool(b) => *b,
            Self::Number(n) => *n != 0.0 && !n.is_nan(),
            Self::String(s) => !s.is_empty(),
            Self::Array(_) | Self::Object(_) => true,
        }
    }

    /// `String(value)`.
    #[must_use]
    pub fn js_string(&self) -> String {
        match self {
            Self::Null => "null".into(),
            Self::Bool(b) => b.to_string(),
            Self::Number(n) => js_number(*n),
            Self::String(s) => s.clone(),
            Self::Array(a) => a
                .iter()
                .map(|v| if matches!(v, Self::Null) { String::new() } else { v.js_string() })
                .collect::<Vec<_>>()
                .join(","),
            Self::Object(_) => "[object Object]".into(),
        }
    }

    /// `typeof value === "object"` (true for `null`, arrays and objects).
    #[must_use]
    pub fn is_js_object(&self) -> bool {
        matches!(self, Self::Null | Self::Array(_) | Self::Object(_))
    }

    /// `JSON.stringify(value, null, indent)`.
    #[must_use]
    pub fn json_stringify(&self, indent: usize) -> String {
        let mut out = String::new();
        write_json(&mut out, self, indent, 0);
        out
    }
}

/// `String(n)` for a JS number.
#[must_use]
pub fn js_number(n: f64) -> String {
    if n == 0.0 {
        return "0".into();
    }
    ryu_js::Buffer::new().format(n).to_owned()
}

/// `Number(s)` for a string.
#[must_use]
pub fn js_string_to_number(s: &str) -> f64 {
    let t = js_trim(s);
    if t.is_empty() {
        return 0.0;
    }
    let radix = |prefix: &[&str], r| {
        prefix
            .iter()
            .find_map(|p| t.strip_prefix(p))
            .map(|digits| if digits.is_empty() { None } else { u128::from_str_radix(digits, r).ok() })
    };
    for (prefixes, r) in [(&["0x", "0X"][..], 16), (&["0o", "0O"][..], 8), (&["0b", "0B"][..], 2)] {
        if let Some(parsed) = radix(prefixes, r) {
            #[expect(clippy::cast_precision_loss, reason = "JavaScript numbers are f64, so the rounding matches")]
            return parsed.map_or(f64::NAN, |n| n as f64);
        }
    }
    let (sign, body) = match t.as_bytes()[0] {
        b'+' => (1.0, &t[1..]),
        b'-' => (-1.0, &t[1..]),
        _ => (1.0, t),
    };
    if body == "Infinity" {
        return sign * f64::INFINITY;
    }
    let valid = {
        let b = body.as_bytes();
        let mut i = 0;
        let int_start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        let mut digits = i - int_start;
        if i < b.len() && b[i] == b'.' {
            i += 1;
            let frac = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            digits += i - frac;
        }
        if digits > 0 && i < b.len() && (b[i] == b'e' || b[i] == b'E') {
            i += 1;
            if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
                i += 1;
            }
            let exp = i;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            if exp == i {
                return f64::NAN;
            }
        }
        digits > 0 && i == b.len()
    };
    if !valid {
        return f64::NAN;
    }
    body.parse::<f64>().map_or(f64::NAN, |n| sign * n)
}

/// Characters `String.prototype.trim` removes.
#[must_use]
pub fn is_js_whitespace(c: char) -> bool {
    matches!(
        c,
        '\u{9}' | '\u{a}' | '\u{b}' | '\u{c}' | '\u{d}' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
    )
}

/// `String.prototype.trim`.
#[must_use]
pub fn js_trim(s: &str) -> &str {
    s.trim_matches(is_js_whitespace)
}

const SURROGATE_SENTINEL_BASE: u32 = 0x10_F000;

/// Rust strings cannot hold the lone UTF-16 surrogates JS strings can, so the parser stores each
/// one as a private-use character; printers turn it back into the escape JS would print.
#[must_use]
pub fn lone_surrogate(c: char) -> Option<u16> {
    let offset = u32::from(c).checked_sub(SURROGATE_SENTINEL_BASE)?;
    u16::try_from(offset).ok().filter(|o| *o < 0x800).map(|o| 0xd800 + o)
}

#[must_use]
pub fn lone_surrogate_char(unit: u32) -> char {
    char::from_u32(SURROGATE_SENTINEL_BASE + unit - 0xd800).unwrap_or('\u{fffd}')
}

#[expect(clippy::needless_bitwise_bool, reason = "branchless fold over the bytes")]
fn write_json_string(out: &mut String, s: &str) {
    out.push('"');
    let special = s.bytes().fold(false, |acc, b| acc | (b < 0x20) | (b == b'"') | (b == b'\\') | (b == 0xf4));
    if !special {
        out.push_str(s);
        out.push('"');
        return;
    }
    let mut plain = 0;
    for (i, c) in s.char_indices() {
        let escape = match c {
            '"' => "\\\"",
            '\\' => "\\\\",
            '\u{8}' => "\\b",
            '\u{c}' => "\\f",
            '\n' => "\\n",
            '\r' => "\\r",
            '\t' => "\\t",
            c if u32::from(c) < 0x20 || lone_surrogate(c).is_some() => "",
            _ => continue,
        };
        out.push_str(&s[plain..i]);
        plain = i + c.len_utf8();
        if escape.is_empty() {
            let unit = lone_surrogate(c).map_or(c as u32, u32::from);
            let _ = write!(out, "\\u{unit:04x}");
        } else {
            out.push_str(escape);
        }
    }
    out.push_str(&s[plain..]);
    out.push('"');
}

/// `JSON.stringify(s)` for a string.
#[must_use]
pub fn json_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    write_json_string(&mut out, s);
    out
}

fn write_json(out: &mut String, v: &Value, indent: usize, depth: usize) {
    let newline = |out: &mut String, depth: usize| {
        if indent > 0 {
            out.push('\n');
            out.extend(std::iter::repeat_n(' ', indent * depth));
        }
    };
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&if n.is_finite() { js_number(*n) } else { "null".into() }),
        Value::String(s) => write_json_string(out, s),
        Value::Array(a) if a.is_empty() => out.push_str("[]"),
        Value::Array(a) => {
            out.push('[');
            for (i, item) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(out, depth + 1);
                write_json(out, item, indent, depth + 1);
            }
            newline(out, depth);
            out.push(']');
        }
        Value::Object(m) if m.is_empty() => out.push_str("{}"),
        Value::Object(m) => {
            out.push('{');
            for (i, (k, item)) in m.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline(out, depth + 1);
                write_json_string(out, k);
                out.push(':');
                if indent > 0 {
                    out.push(' ');
                }
                write_json(out, item, indent, depth + 1);
            }
            newline(out, depth);
            out.push('}');
        }
    }
}

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(ValueVisitor)
    }
}

struct ValueVisitor;

impl<'de> Visitor<'de> for ValueVisitor {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("any JSON value")
    }

    fn visit_bool<E>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }

    #[expect(clippy::cast_precision_loss, reason = "JavaScript numbers are f64, so the rounding matches")]
    fn visit_i64<E>(self, v: i64) -> Result<Value, E> {
        Ok(Value::Number(v as f64))
    }

    #[expect(clippy::cast_precision_loss, reason = "JavaScript numbers are f64, so the rounding matches")]
    fn visit_u64<E>(self, v: u64) -> Result<Value, E> {
        Ok(Value::Number(v as f64))
    }

    fn visit_f64<E>(self, v: f64) -> Result<Value, E> {
        Ok(Value::Number(v))
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Value, E> {
        Ok(Value::String(v.to_owned()))
    }

    fn visit_string<E>(self, v: String) -> Result<Value, E> {
        Ok(Value::String(v))
    }

    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_none<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut out = Vec::with_capacity(seq.size_hint().unwrap_or(0));
        while let Some(v) = seq.next_element()? {
            out.push(v);
        }
        Ok(Value::Array(out))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Value, A::Error> {
        let mut map = Map::new();
        while let Some((k, v)) = access.next_entry::<String, Value>()? {
            map.insert(k, v);
        }
        Ok(Value::Object(map))
    }
}

#[cfg(test)]
#[expect(clippy::float_cmp, reason = "tests assert exact JavaScript numbers")]
mod tests {
    use super::*;

    #[test]
    fn key_order_matches_js() {
        let v: Value =
            serde_json::from_str(r#"{"default":1,"404":2,"b":3,"200":4,"01":5,"4294967295":6,"2XX":7}"#).unwrap();
        let keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, ["200", "404", "default", "b", "01", "4294967295", "2XX"]);
    }

    #[test]
    fn indexed_map_keeps_order_and_lookups() {
        let mut m = JsMap::new();
        for k in ["a", "b", "c", "d", "e", "f", "g", "h", "i", "7", "j", "3", "10", "b"] {
            m.insert(k.to_owned(), k.len());
        }
        let keys: Vec<_> = m.keys().map(String::as_str).collect();
        assert_eq!(keys, ["3", "7", "10", "a", "b", "c", "d", "e", "f", "g", "h", "i", "j"]);
        assert!(keys.iter().enumerate().all(|(i, k)| m.position(k) == Some(i)));
        assert_eq!(m.get("x"), None);
    }

    #[test]
    fn value_stays_small() {
        assert_eq!(std::mem::size_of::<Value>(), 40);
    }

    #[test]
    fn numbers_print_like_js() {
        assert_eq!(js_number(1e21), "1e+21");
        assert_eq!(js_number(0.1), "0.1");
        assert_eq!(js_number(-0.0), "0");
        assert_eq!(js_number(123_456_789_012_345_680_000.0), "123456789012345680000");
        assert_eq!(js_number(1.5e-7), "1.5e-7");
        assert_eq!(js_string_to_number("1e3"), 1000.0);
        assert!(js_string_to_number("1_0").is_nan());
        assert_eq!(js_string_to_number(" 12 "), 12.0);
        assert_eq!(js_string_to_number("0x1A"), 26.0);
        assert_eq!(js_string_to_number("-Infinity"), f64::NEG_INFINITY);
        assert_eq!(js_string_to_number(".5"), 0.5);
        assert_eq!(js_string_to_number("5."), 5.0);
        assert!(js_string_to_number(".").is_nan());
    }

    #[test]
    fn stringify_matches_js() {
        let v: Value = serde_json::from_str(r#"{"a":[1,{"b":null}],"c":"\u0001\"","d":{},"e":[]}"#).unwrap();
        assert_eq!(
            v.json_stringify(2),
            "{\n  \"a\": [\n    1,\n    {\n      \"b\": null\n    }\n  ],\n  \"c\": \"\\u0001\\\"\",\n  \"d\": {},\n  \"e\": []\n}"
        );
    }
}

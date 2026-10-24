//! A JSON reader that can follow either `JSON.parse` or js-yaml (which Redocly uses for files).

use crate::value::{Map, Value};
use crate::{Error, Result};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// `JSON.parse`: later duplicate keys win, `1e400` is `Infinity`.
    JsonParse,
    /// js-yaml's JSON schema: duplicate keys are an error, non-finite numbers stay strings.
    Yaml,
}

struct Reader<'a> {
    src: &'a [u8],
    text: &'a str,
    pos: usize,
    mode: Mode,
}

pub fn parse(text: &str, mode: Mode) -> Result<Value> {
    let mut r = Reader { src: text.as_bytes(), text, pos: 0, mode };
    r.ws();
    let v = r.value(0)?;
    r.ws();
    if r.pos != r.src.len() {
        return Err(r.error("unexpected trailing characters"));
    }
    Ok(v)
}

fn plain_run(bytes: &[u8]) -> usize {
    const ONES: u64 = u64::from_ne_bytes([0x01; 8]);
    const HIGH: u64 = u64::from_ne_bytes([0x80; 8]);
    let below = |x: u64, n: u8| x.wrapping_sub(ONES * u64::from(n)) & !x & HIGH;
    bytes
        .chunks_exact(8)
        .take_while(|chunk| {
            let x = chunk.iter().fold(0u64, |acc, &b| (acc << 8) | u64::from(b));
            below(x ^ (ONES * u64::from(b'"')), 1) | below(x ^ (ONES * u64::from(b'\\')), 1) | below(x, 0x20) == 0
        })
        .count()
        * 8
}

pub const MAX_DEPTH: usize = 1000;

impl Reader<'_> {
    fn error(&self, msg: &str) -> Error {
        let before = &self.text[..self.pos.min(self.text.len())];
        let line = before.matches('\n').count() + 1;
        let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
        Error::Parse(format!("invalid JSON at line {line} column {col}: {msg}"))
    }

    fn ws(&mut self) {
        while let Some(&b) = self.src.get(self.pos) {
            match b {
                b' ' => self.pos += super::leading_spaces(&self.src[self.pos..]),
                b'\t' | b'\n' | b'\r' => self.pos += 1,
                _ => break,
            }
        }
    }

    fn eat(&mut self, b: u8) -> bool {
        if self.src.get(self.pos) == Some(&b) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value> {
        if depth > MAX_DEPTH {
            return Err(self.error(&format!("nesting deeper than {MAX_DEPTH} levels")));
        }
        match self.src.get(self.pos) {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => Ok(Value::String(self.string()?)),
            Some(b't') => self.keyword("true", Value::Bool(true)),
            Some(b'f') => self.keyword("false", Value::Bool(false)),
            Some(b'n') => self.keyword("null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.error("unexpected character")),
            None => Err(self.error("unexpected end of input")),
        }
    }

    fn keyword(&mut self, word: &str, v: Value) -> Result<Value> {
        if self.src[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(v)
        } else {
            Err(self.error("unexpected character"))
        }
    }

    fn number(&mut self) -> Result<Value> {
        let start = self.pos;
        self.eat(b'-');
        let digits = |r: &mut Self| {
            let s = r.pos;
            while r.src.get(r.pos).is_some_and(u8::is_ascii_digit) {
                r.pos += 1;
            }
            r.pos - s
        };
        if self.eat(b'0') {
        } else if digits(self) == 0 {
            return Err(self.error("invalid number"));
        }
        if self.eat(b'.') && digits(self) == 0 {
            return Err(self.error("invalid number"));
        }
        if matches!(self.src.get(self.pos), Some(b'e' | b'E')) {
            self.pos += 1;
            if !self.eat(b'+') {
                self.eat(b'-');
            }
            if digits(self) == 0 {
                return Err(self.error("invalid number"));
            }
        }
        let text = &self.text[start..self.pos];
        let n: f64 = text.parse().map_err(|_| self.error("invalid number"))?;
        if self.mode == Mode::Yaml && !n.is_finite() {
            return Ok(Value::String(text.to_owned()));
        }
        Ok(Value::Number(n))
    }

    fn hex4(&mut self) -> Result<u32> {
        let h = self.text.get(self.pos..self.pos + 4).ok_or_else(|| self.error("invalid unicode escape"))?;
        let v = u32::from_str_radix(h, 16).map_err(|_| self.error("invalid unicode escape"))?;
        self.pos += 4;
        Ok(v)
    }

    fn string(&mut self) -> Result<String> {
        self.pos += 1;
        let mut out = String::new();
        loop {
            let start = self.pos;
            self.pos += plain_run(&self.src[self.pos..]);
            while let Some(&b) = self.src.get(self.pos) {
                if b == b'"' || b == b'\\' || b < 0x20 {
                    break;
                }
                self.pos += 1;
            }
            let run = &self.text[start..self.pos];
            match self.src.get(self.pos) {
                Some(b'"') if out.is_empty() => {
                    self.pos += 1;
                    return Ok(run.to_owned());
                }
                Some(b'"') => {
                    self.pos += 1;
                    out.push_str(run);
                    return Ok(out);
                }
                Some(b'\\') => {
                    out.push_str(run);
                    self.pos += 1;
                    let esc = *self.src.get(self.pos).ok_or_else(|| self.error("unterminated string"))?;
                    self.pos += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let c = if (0xd800..0xdc00).contains(&hi) && self.src[self.pos..].starts_with(b"\\u") {
                                let save = self.pos;
                                self.pos += 2;
                                let lo = self.hex4()?;
                                if (0xdc00..0xe000).contains(&lo) {
                                    char::from_u32(0x10000 + ((hi - 0xd800) << 10) + (lo - 0xdc00))
                                } else {
                                    self.pos = save;
                                    None
                                }
                            } else {
                                char::from_u32(hi)
                            };
                            out.push(c.unwrap_or_else(|| crate::value::lone_surrogate_char(hi)));
                        }
                        _ => return Err(self.error("invalid escape")),
                    }
                }
                Some(_) => return Err(self.error("control character in string")),
                None => return Err(self.error("unterminated string")),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Value> {
        self.pos += 1;
        let mut out = Vec::new();
        self.ws();
        if self.eat(b']') {
            return Ok(Value::Array(out));
        }
        loop {
            self.ws();
            out.push(self.value(depth + 1)?);
            self.ws();
            if self.eat(b',') {
                continue;
            }
            if self.eat(b']') {
                return Ok(Value::Array(out));
            }
            return Err(self.error("expected `,` or `]`"));
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value> {
        self.pos += 1;
        let mut map = Map::new();
        self.ws();
        if self.eat(b'}') {
            return Ok(Value::Object(map));
        }
        loop {
            self.ws();
            if self.src.get(self.pos) != Some(&b'"') {
                return Err(self.error("expected a string key"));
            }
            let key_pos = self.pos;
            let key = self.string()?;
            self.ws();
            if !self.eat(b':') {
                return Err(self.error("expected `:`"));
            }
            self.ws();
            let v = self.value(depth + 1)?;
            let len = map.len();
            map.insert(key, v);
            if self.mode == Mode::Yaml && map.len() == len {
                self.pos = key_pos;
                let key = self.string()?;
                self.pos = key_pos;
                return Err(self.error(&format!("duplicated mapping key \"{key}\"")));
            }
            self.ws();
            if self.eat(b',') {
                continue;
            }
            if self.eat(b'}') {
                return Ok(Value::Object(map));
            }
            return Err(self.error("expected `,` or `}`"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_differ_like_js() {
        let src = r#"{"a":1e400,"b":"😀\ud800"}"#;
        let v = parse(src, Mode::JsonParse).unwrap();
        assert_eq!(v.get("a"), Some(&Value::Number(f64::INFINITY)));
        assert_eq!(v.get("b").map(|b| b.json_stringify(0)).as_deref(), Some("\"😀\\ud800\""));
        let v = parse(src, Mode::Yaml).unwrap();
        assert_eq!(v.get("a").and_then(Value::as_str), Some("1e400"));
        assert!(parse(r#"{"a":1,"a":2}"#, Mode::Yaml).is_err());
        assert_eq!(parse(r#"{"a":1,"a":2}"#, Mode::JsonParse).unwrap().get("a"), Some(&Value::Number(2.0)));
        let long = parse(r#"{"abcdefghijklmnop":"0123456789é01234\"x\\y\n"}"#, Mode::Yaml).unwrap();
        assert_eq!(long.get("abcdefghijklmnop").and_then(Value::as_str), Some("0123456789é01234\"x\\y\n"));
        assert!(parse("\"0123456789abcdef\u{1}\"", Mode::Yaml).is_err());
    }
}

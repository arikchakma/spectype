//! A port of js-yaml 4's `loader.js` over bytes, building [`Value`]s directly.
//!
//! It follows js-yaml's state machine function for function (including its quirks, such as how
//! a failed block-mapping probe leaves `tag`/`anchor` behind) instead of the YAML spec, because
//! openapi-typescript sees whatever js-yaml produces. Positions are byte offsets; the one place
//! js-yaml measures UTF-16 columns (block indentation after a same-line prefix) converts.

use std::borrow::Cow;

use rustc_hash::FxHashMap;

use crate::parse::json::MAX_DEPTH;
use crate::value::{Map, Value, lone_surrogate, lone_surrogate_char};

pub enum Fail {
    Error(String),
    /// A construct this loader leaves to yaml-rust2 (`!!binary`).
    Fallback,
}

type R<T> = Result<T, Fail>;

const YAML_TAG: &str = "tag:yaml.org,2002:";
const MERGE_TAG: &str = "tag:yaml.org,2002:merge";
const MAX_MERGE_WORK: usize = 10_000;
/// js-yaml shares aliased objects; we copy them, so cap the copies against alias bombs.
const MAX_ALIAS_NODES: usize = 10_000_000;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    None,
    Scalar,
    Sequence,
    Mapping,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Ctx {
    FlowIn,
    FlowOut,
    BlockIn,
    BlockOut,
}

#[derive(Clone)]
enum Anchor {
    /// A collection still being read; js-yaml would hand out the half-built object.
    Pending(u32),
    Done(Value, usize),
}

#[derive(Clone)]
struct Snapshot<'a> {
    position: usize,
    line: usize,
    line_start: usize,
    line_indent: i64,
    first_tab: Option<usize>,
    tag: Option<Cow<'a, str>>,
    anchor: Option<&'a str>,
    kind: Kind,
}

#[derive(Default)]
struct Overridable(Option<rustc_hash::FxHashSet<String>>);

struct Loader<'a> {
    text: &'a str,
    src: &'a [u8],
    /// The `\n` js-yaml appends when the input does not end with a line break.
    tail: u8,
    length: usize,
    position: usize,
    line: usize,
    line_start: usize,
    line_indent: i64,
    first_tab: Option<usize>,
    depth: usize,
    merge_work: usize,
    alias_nodes: usize,
    tag: Option<Cow<'a, str>>,
    anchor: Option<&'a str>,
    kind: Kind,
    result: Value,
    anchors: FxHashMap<&'a str, Anchor>,
    transactions: Vec<FxHashMap<&'a str, Option<Anchor>>>,
    tag_map: FxHashMap<&'a str, String>,
    version: bool,
    next_pending: u32,
}

#[inline]
fn is_eol(c: u8) -> bool {
    c == b'\n' || c == b'\r'
}

#[inline]
fn is_ws(c: u8) -> bool {
    c == b' ' || c == b'\t'
}

#[inline]
fn is_ws_or_eol(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r')
}

#[inline]
fn is_flow(c: u8) -> bool {
    matches!(c, b',' | b'[' | b']' | b'{' | b'}')
}

const ORDINARY: u8 = 0;
const STOP: u8 = 1;
const SUSPICIOUS: u8 = 2;

/// Byte classes for the scanning fast paths: `STOP` bytes need the slow path, `SUSPICIOUS`
/// bytes may start a character js-yaml rejects, so the segment gets checked.
const fn byte_classes(stops: &[u8], printable_check: bool) -> [u8; 256] {
    let mut t = [ORDINARY; 256];
    let mut i = 0;
    while i < 0x20 {
        t[i] = SUSPICIOUS;
        i += 1;
    }
    if printable_check {
        t[0x7f] = SUSPICIOUS;
        t[0xc2] = SUSPICIOUS;
        t[0xef] = SUSPICIOUS;
    }
    let mut j = 0;
    while j < stops.len() {
        t[stops[j] as usize] = STOP;
        j += 1;
    }
    t
}

static PLAIN_BLOCK: [u8; 256] = byte_classes(b"\0\t\n\r :#", true);
static PLAIN_FLOW: [u8; 256] = byte_classes(b"\0\t\n\r :#,[]{}", true);
static SINGLE_QUOTED: [u8; 256] = byte_classes(b"\0\t\n\r '", false);
static DOUBLE_QUOTED: [u8; 256] = byte_classes(b"\0\t\n\r \"\\", false);
static LINE: [u8; 256] = byte_classes(b"\0\n\r", true);
static EOL: [u8; 256] = {
    let mut t = [ORDINARY; 256];
    t[0] = STOP;
    t[b'\n' as usize] = STOP;
    t[b'\r' as usize] = STOP;
    t
};

/// js-yaml's `PATTERN_NON_PRINTABLE` (minus lone surrogates, which UTF-8 cannot hold).
fn has_non_printable(b: &[u8]) -> bool {
    b.iter().enumerate().any(|(i, &c)| match c {
        0x09 | 0x0a | 0x0d => false,
        0x00..=0x1f | 0x7f => true,
        0xc2 => matches!(b.get(i + 1), Some(0x80..=0x84 | 0x86..=0x9f)),
        0xef => b.get(i + 1) == Some(&0xbf) && matches!(b.get(i + 2), Some(0xbe | 0xbf)),
        _ => false,
    })
}

fn simple_escape(c: u8) -> Option<char> {
    Some(match c {
        b'0' => '\0',
        b'a' => '\x07',
        b'b' => '\x08',
        b't' | b'\t' => '\t',
        b'n' => '\n',
        b'v' => '\x0b',
        b'f' => '\x0c',
        b'r' => '\r',
        b'e' => '\x1b',
        b' ' => ' ',
        b'"' => '"',
        b'/' => '/',
        b'\\' => '\\',
        b'N' => '\u{85}',
        b'_' => '\u{a0}',
        b'L' => '\u{2028}',
        b'P' => '\u{2029}',
        _ => return None,
    })
}

fn hex_value(c: u8) -> Option<u32> {
    char::from(c).to_digit(16)
}

/// A scalar under construction: one input slice when possible, an owned buffer otherwise.
struct Text<'a> {
    src: &'a str,
    single: Option<(usize, usize)>,
    buf: String,
    /// `buf.len()` right after a high-surrogate escape, so a following low one can pair up.
    high_at: usize,
}

impl<'a> Text<'a> {
    fn new(src: &'a str) -> Self {
        Text { src, single: None, buf: String::new(), high_at: usize::MAX }
    }

    fn slice(&self, s: usize, e: usize) -> R<&'a str> {
        self.src.get(s..e).ok_or_else(|| Fail::Error("scalar split inside a character".into()))
    }

    fn spill(&mut self) -> R<()> {
        if let Some((s, e)) = self.single.take() {
            let t = self.slice(s, e)?;
            self.buf.push_str(t);
        }
        Ok(())
    }

    fn segment(&mut self, s: usize, e: usize) -> R<()> {
        if s >= e {
            return Ok(());
        }
        if self.single.is_none() && self.buf.is_empty() {
            self.slice(s, e)?;
            self.single = Some((s, e));
            return Ok(());
        }
        self.spill()?;
        let t = self.slice(s, e)?;
        self.buf.push_str(t);
        Ok(())
    }

    fn push(&mut self, c: char) -> R<()> {
        self.spill()?;
        self.buf.push(c);
        Ok(())
    }

    fn push_newlines(&mut self, n: usize) -> R<()> {
        if n > 0 {
            self.spill()?;
            self.buf.extend(std::iter::repeat_n('\n', n));
        }
        Ok(())
    }

    fn fold(&mut self, count: usize) -> R<()> {
        if count == 1 { self.push(' ') } else { self.push_newlines(count.saturating_sub(1)) }
    }

    /// Appends one UTF-16 code unit the way a JS string concatenation would.
    fn push_unit(&mut self, unit: u32) -> R<()> {
        self.spill()?;
        if (0xdc00..0xe000).contains(&unit)
            && self.high_at == self.buf.len()
            && let Some(hi) = self.buf.pop().and_then(lone_surrogate)
        {
            let c = 0x10000 + ((u32::from(hi) - 0xd800) << 10) + (unit - 0xdc00);
            self.buf.push(char::from_u32(c).unwrap_or('\u{fffd}'));
            self.high_at = usize::MAX;
            return Ok(());
        }
        if let Some(c) = char::from_u32(unit) {
            self.buf.push(c);
        } else {
            self.buf.push(lone_surrogate_char(unit));
            if unit < 0xdc00 {
                self.high_at = self.buf.len();
            }
        }
        Ok(())
    }

    fn is_empty(&self) -> bool {
        self.single.is_none() && self.buf.is_empty()
    }

    fn finish(self) -> R<String> {
        match self.single {
            Some((s, e)) => Ok(self.slice(s, e)?.to_owned()),
            None => Ok(self.buf),
        }
    }
}

/// `decodeURIComponent`.
fn decode_uri(s: &str) -> Option<Cow<'_, str>> {
    if !s.contains('%') {
        return Some(Cow::Borrowed(s));
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hi = hex_value(*b.get(i + 1)?)?;
            let lo = hex_value(*b.get(i + 2)?)?;
            out.push(u8::try_from(hi * 16 + lo).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok().map(Cow::Owned)
}

/// js-yaml's `PATTERN_TAG_URI`, matched per UTF-16 unit.
fn is_tag_uri(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if u32::from(c) > 0xffff || matches!(c, ',' | '[' | ']' | '{' | '}') => return false,
        None => return false,
        Some(_) => {}
    }
    let rest = chars.as_str().as_bytes();
    let mut i = 0;
    while i < rest.len() {
        let c = rest[i];
        if c == b'%' {
            if !(rest.get(i + 1).is_some_and(u8::is_ascii_hexdigit)
                && rest.get(i + 2).is_some_and(u8::is_ascii_hexdigit))
            {
                return false;
            }
            i += 3;
        } else if c.is_ascii_alphanumeric() || b"-#;/?:@&=+$,_.!~*'()[]".contains(&c) {
            i += 1;
        } else {
            return false;
        }
    }
    true
}

/// js-yaml's `PATTERN_TAG_HANDLE`.
fn is_tag_handle(s: &str) -> bool {
    match s.as_bytes() {
        b"!" | b"!!" => true,
        [b'!', mid @ .., b'!'] => !mid.is_empty() && mid.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'-'),
        _ => false,
    }
}

fn node_count(v: &Value) -> usize {
    1 + match v {
        Value::Array(a) => a.iter().map(node_count).sum(),
        Value::Object(m) => m.values().map(node_count).sum(),
        _ => 0,
    }
}

fn js_keys(v: &Value) -> Vec<(String, Value)> {
    match v {
        Value::Object(m) => m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        Value::Array(a) => a.iter().enumerate().map(|(i, v)| (i.to_string(), v.clone())).collect(),
        _ => Vec::new(),
    }
}

impl<'a> Loader<'a> {
    #[inline]
    fn at(&self, i: usize) -> u8 {
        match self.src.get(i) {
            Some(&b) => b,
            None => self.past_end(i),
        }
    }

    #[cold]
    fn past_end(&self, i: usize) -> u8 {
        if i == self.src.len() { self.tail } else { 0 }
    }

    #[inline]
    fn ch(&self) -> u8 {
        self.at(self.position)
    }

    fn error(&self, msg: &str) -> Fail {
        let col = self.position.saturating_sub(self.line_start) + 1;
        Fail::Error(format!("{msg} (line {}, column {col})", self.line + 1))
    }

    fn slice(&self, s: usize, e: usize) -> R<&'a str> {
        self.text.get(s..e.min(self.src.len())).ok_or_else(|| self.error("token split inside a character"))
    }

    /// `position - lineStart` in UTF-16 units, as js-yaml measures it.
    fn column(&self, position: usize, line_start: usize) -> i64 {
        let bytes = &self.src[line_start.min(self.src.len())..position.min(self.src.len())];
        let n = if bytes.is_ascii() {
            position - line_start
        } else {
            bytes.iter().map(|&b| usize::from(b & 0xc0 != 0x80) + usize::from(b >= 0xf0)).sum::<usize>()
                + (position - line_start - bytes.len())
        };
        i64::try_from(n).unwrap_or(i64::MAX)
    }

    fn snapshot(&self) -> Snapshot<'a> {
        Snapshot {
            position: self.position,
            line: self.line,
            line_start: self.line_start,
            line_indent: self.line_indent,
            first_tab: self.first_tab,
            tag: self.tag.clone(),
            anchor: self.anchor,
            kind: self.kind,
        }
    }

    fn restore(&mut self, s: Snapshot<'a>) {
        self.position = s.position;
        self.line = s.line;
        self.line_start = s.line_start;
        self.line_indent = s.line_indent;
        self.first_tab = s.first_tab;
        self.tag = s.tag;
        self.anchor = s.anchor;
        self.kind = s.kind;
    }

    fn record(&mut self, name: &'a str) {
        if let Some(tx) = self.transactions.last()
            && !tx.contains_key(name)
        {
            let prev = self.anchors.get(name).cloned();
            if let Some(tx) = self.transactions.last_mut() {
                tx.insert(name, prev);
            }
        }
    }

    fn store_anchor(&mut self, name: &'a str, value: Value) {
        self.record(name);
        let size = node_count(&value);
        self.anchors.insert(name, Anchor::Done(value, size));
    }

    fn store_pending(&mut self, name: &'a str) -> (&'a str, u32) {
        self.record(name);
        self.next_pending += 1;
        self.anchors.insert(name, Anchor::Pending(self.next_pending));
        (name, self.next_pending)
    }

    /// The collection js-yaml stored at its start is complete (or abandoned empty).
    fn settle(&mut self, pending: Option<(&'a str, u32)>, value: impl FnOnce(&Self) -> Value) {
        if let Some((name, id)) = pending
            && matches!(self.anchors.get(name), Some(Anchor::Pending(p)) if *p == id)
        {
            let value = value(self);
            let size = node_count(&value);
            self.anchors.insert(name, Anchor::Done(value, size));
        }
    }

    fn commit_transaction(&mut self) {
        let Some(tx) = self.transactions.pop() else { return };
        if let Some(parent) = self.transactions.last_mut() {
            for (name, prev) in tx {
                parent.entry(name).or_insert(prev);
            }
        }
    }

    fn rollback_transaction(&mut self) {
        let Some(tx) = self.transactions.pop() else { return };
        for (name, prev) in tx {
            match prev {
                Some(a) => self.anchors.insert(name, a),
                None => self.anchors.remove(name),
            };
        }
    }

    fn line_break(&mut self) {
        if self.ch() == b'\r' {
            self.position += 1;
            if self.ch() == b'\n' {
                self.position += 1;
            }
        } else {
            self.position += 1;
        }
        self.line += 1;
        self.line_start = self.position;
        self.first_tab = None;
    }

    fn read_line_break(&mut self) -> R<()> {
        if !is_eol(self.ch()) {
            return Err(self.error("a line break is expected"));
        }
        self.line_break();
        Ok(())
    }

    fn skip_separation_space(&mut self, allow_comments: bool) -> usize {
        let mut breaks = 0;
        let mut ch = self.ch();
        while ch != 0 {
            while is_ws(ch) {
                if ch == b'\t' && self.first_tab.is_none() {
                    self.first_tab = Some(self.position);
                }
                self.position += 1;
                ch = self.ch();
            }
            if allow_comments && ch == b'#' {
                self.position = self.scan(self.position + 1, &EOL);
                ch = self.ch();
            }
            if !is_eol(ch) {
                break;
            }
            self.line_break();
            breaks += 1;
            let spaces = self.src.get(self.position..).map_or(0, crate::parse::leading_spaces);
            self.line_indent = i64::try_from(spaces).unwrap_or(i64::MAX);
            self.position += spaces;
            ch = self.ch();
        }
        breaks
    }

    fn test_document_separator(&self) -> bool {
        let p = self.position;
        let ch = self.at(p);
        (ch == b'-' || ch == b'.')
            && self.at(p + 1) == ch
            && self.at(p + 2) == ch
            && (self.at(p + 3) == 0 || is_ws_or_eol(self.at(p + 3)))
    }

    /// The first position at or after `from` whose byte is not `ORDINARY`.
    #[inline]
    fn scan(&self, from: usize, table: &[u8; 256]) -> usize {
        self.src.get(from..).map_or(from, |rest| {
            rest.iter().position(|&b| table[usize::from(b)] != ORDINARY).map_or(self.src.len(), |n| from + n)
        })
    }

    fn capture(&self, text: &mut Text<'a>, s: usize, e: usize, json: bool, check: bool) -> R<()> {
        if check && s < e {
            let bytes = &self.src[s.min(self.src.len())..e.min(self.src.len())];
            let bad = if json { bytes.iter().any(|&c| c < 0x20 && c != 0x09) } else { has_non_printable(bytes) };
            if bad {
                return Err(self.error(if json {
                    "expected valid JSON character"
                } else {
                    "the stream contains non-printable characters"
                }));
            }
        }
        text.segment(s, e)
    }

    #[inline(never)]
    fn read_plain_scalar(&mut self, node_indent: i64, within_flow: bool) -> R<bool> {
        let mut ch = self.ch();
        if is_ws_or_eol(ch)
            || is_flow(ch)
            || matches!(ch, b'#' | b'&' | b'*' | b'!' | b'|' | b'>' | b'\'' | b'"' | b'%' | b'@' | b'`')
        {
            return Ok(false);
        }
        if ch == b'?' || ch == b'-' {
            let following = self.at(self.position + 1);
            if is_ws_or_eol(following) || (within_flow && is_flow(following)) {
                return Ok(false);
            }
        }
        let mut text = Text::new(self.text);
        let mut cap_start = self.position;
        let mut cap_end = self.position;
        let mut pending = false;
        let mut saved_line = self.line;
        let table = if within_flow { &PLAIN_FLOW } else { &PLAIN_BLOCK };
        let mut suspicious = false;
        while ch != 0 {
            if ch == b':' {
                let following = self.at(self.position + 1);
                if is_ws_or_eol(following) || (within_flow && is_flow(following)) {
                    break;
                }
            } else if ch == b'#' {
                if self.position > 0 && is_ws_or_eol(self.at(self.position - 1)) {
                    break;
                }
            } else if (self.position == self.line_start && self.test_document_separator())
                || (within_flow && is_flow(ch))
            {
                break;
            } else if is_eol(ch) {
                saved_line = self.line;
                let (line_start, line_indent) = (self.line_start, self.line_indent);
                self.skip_separation_space(false);
                if self.line_indent >= node_indent {
                    pending = true;
                    ch = self.ch();
                    continue;
                }
                self.position = cap_end;
                self.line = saved_line;
                self.line_start = line_start;
                self.line_indent = line_indent;
                break;
            }
            if pending {
                self.capture(&mut text, cap_start, cap_end, false, suspicious)?;
                text.fold(self.line - saved_line)?;
                cap_start = self.position;
                cap_end = self.position;
                pending = false;
            }
            if !is_ws(ch) {
                cap_end = self.position + 1;
                suspicious |= table[usize::from(ch)] == SUSPICIOUS;
            }
            self.position += 1;
            let next = self.scan(self.position, table);
            if next > self.position {
                self.position = next;
                cap_end = next;
            }
            ch = self.ch();
        }
        self.capture(&mut text, cap_start, cap_end, false, suspicious)?;
        if text.is_empty() {
            return Ok(false);
        }
        self.kind = Kind::Scalar;
        self.result = Value::String(text.finish()?);
        Ok(true)
    }

    #[inline(never)]
    fn read_single_quoted_scalar(&mut self) -> R<bool> {
        if self.ch() != b'\'' {
            return Ok(false);
        }
        self.kind = Kind::Scalar;
        self.result = Value::Null;
        self.position += 1;
        let mut text = Text::new(self.text);
        let mut cap_start = self.position;
        let mut cap_end = self.position;
        let mut suspicious = false;
        loop {
            let ch = self.ch();
            if ch == 0 {
                break;
            }
            if ch == b'\'' {
                self.capture(&mut text, cap_start, self.position, true, suspicious)?;
                self.position += 1;
                if self.ch() == b'\'' {
                    cap_start = self.position;
                    self.position += 1;
                    cap_end = self.position;
                } else {
                    self.result = Value::String(text.finish()?);
                    return Ok(true);
                }
            } else if is_eol(ch) {
                self.capture(&mut text, cap_start, cap_end, true, suspicious)?;
                let n = self.skip_separation_space(false);
                text.fold(n)?;
                cap_start = self.position;
                cap_end = self.position;
            } else if self.position == self.line_start && self.test_document_separator() {
                return Err(self.error("unexpected end of the document within a single quoted scalar"));
            } else {
                self.position += 1;
                if !is_ws(ch) {
                    cap_end = self.position;
                    suspicious |= ch < 0x20;
                }
                let next = self.scan(self.position, &SINGLE_QUOTED);
                if next > self.position {
                    self.position = next;
                    cap_end = next;
                }
            }
        }
        Err(self.error("unexpected end of the stream within a single quoted scalar"))
    }

    #[inline(never)]
    fn read_double_quoted_scalar(&mut self) -> R<bool> {
        if self.ch() != b'"' {
            return Ok(false);
        }
        self.kind = Kind::Scalar;
        self.result = Value::Null;
        self.position += 1;
        let mut text = Text::new(self.text);
        let mut cap_start = self.position;
        let mut cap_end = self.position;
        let mut suspicious = false;
        loop {
            let ch = self.ch();
            if ch == 0 {
                break;
            }
            if ch == b'"' {
                self.capture(&mut text, cap_start, self.position, true, suspicious)?;
                self.position += 1;
                self.result = Value::String(text.finish()?);
                return Ok(true);
            } else if ch == b'\\' {
                self.capture(&mut text, cap_start, self.position, true, suspicious)?;
                self.position += 1;
                let ch = self.ch();
                if is_eol(ch) {
                    self.skip_separation_space(false);
                } else if let Some(c) = simple_escape(ch) {
                    text.push(c)?;
                    self.position += 1;
                } else if let Some(len) = match ch {
                    b'x' => Some(2),
                    b'u' => Some(4),
                    b'U' => Some(8),
                    _ => None,
                } {
                    let mut code: i32 = 0;
                    for _ in 0..len {
                        self.position += 1;
                        let Some(d) = hex_value(self.ch()) else {
                            return Err(self.error("expected hexadecimal character"));
                        };
                        code = code.wrapping_shl(4).wrapping_add(i32::try_from(d).unwrap_or(0));
                    }
                    push_code_point(&mut text, code)?;
                    self.position += 1;
                } else {
                    return Err(self.error("unknown escape sequence"));
                }
                cap_start = self.position;
                cap_end = self.position;
            } else if is_eol(ch) {
                self.capture(&mut text, cap_start, cap_end, true, suspicious)?;
                let n = self.skip_separation_space(false);
                text.fold(n)?;
                cap_start = self.position;
                cap_end = self.position;
            } else if self.position == self.line_start && self.test_document_separator() {
                return Err(self.error("unexpected end of the document within a double quoted scalar"));
            } else {
                self.position += 1;
                if !is_ws(ch) {
                    cap_end = self.position;
                    suspicious |= ch < 0x20;
                }
                let next = self.scan(self.position, &DOUBLE_QUOTED);
                if next > self.position {
                    self.position = next;
                    cap_end = next;
                }
            }
        }
        Err(self.error("unexpected end of the stream within a double quoted scalar"))
    }

    #[inline(never)]
    fn read_flow_collection(&mut self, node_indent: i64) -> R<bool> {
        let (terminator, is_mapping) = match self.ch() {
            b'[' => (b']', false),
            b'{' => (b'}', true),
            _ => return Ok(false),
        };
        let tag = self.tag.clone();
        let anchor = self.anchor;
        let pending = anchor.map(|a| self.store_pending(a));
        let mut seq = Vec::new();
        let mut map = Map::new();
        let mut overridable = Overridable::default();
        let mut read_next = true;
        self.position += 1;
        let mut ch = self.ch();
        while ch != 0 {
            self.skip_separation_space(true);
            ch = self.ch();
            if ch == terminator {
                self.position += 1;
                self.tag = tag;
                self.anchor = anchor;
                if is_mapping {
                    self.kind = Kind::Mapping;
                    self.result = Value::Object(map);
                } else {
                    self.kind = Kind::Sequence;
                    self.result = Value::Array(seq);
                }
                self.settle(pending, |l| l.result.clone());
                return Ok(true);
            } else if !read_next {
                return Err(self.error("missed comma between flow collection entries"));
            } else if ch == b',' {
                return Err(self.error("expected the node content, but found ','"));
            }
            let mut is_pair = false;
            let mut is_explicit = false;
            if ch == b'?' && is_ws_or_eol(self.at(self.position + 1)) {
                is_pair = true;
                is_explicit = true;
                self.position += 1;
                self.skip_separation_space(true);
            }
            let line = self.line;
            self.compose_node(node_indent, Ctx::FlowIn, false, true)?;
            let mut key_tag = self.tag.take();
            let mut key_node = std::mem::replace(&mut self.result, Value::Null);
            self.skip_separation_space(true);
            ch = self.ch();
            let mut value_node = Value::Null;
            if (is_explicit || self.line == line) && ch == b':' {
                is_pair = true;
                self.position += 1;
                self.skip_separation_space(true);
                self.compose_node(node_indent, Ctx::FlowIn, false, true)?;
                std::mem::swap(&mut value_node, &mut self.result);
            }
            if is_mapping {
                self.store_mapping_pair(
                    &mut map,
                    &mut overridable,
                    &mut key_tag,
                    &mut key_node,
                    Some(&mut value_node),
                )?;
            } else if is_pair {
                let mut pair = Map::new();
                self.store_mapping_pair(
                    &mut pair,
                    &mut overridable,
                    &mut key_tag,
                    &mut key_node,
                    Some(&mut value_node),
                )?;
                seq.push(Value::Object(pair));
            } else {
                seq.push(key_node);
            }
            self.skip_separation_space(true);
            ch = self.ch();
            if ch == b',' {
                read_next = true;
                self.position += 1;
                ch = self.ch();
            } else {
                read_next = false;
            }
        }
        Err(self.error("unexpected end of the stream within a flow collection"))
    }

    #[inline(never)]
    fn read_block_scalar(&mut self, node_indent: i64) -> R<bool> {
        let mut ch = self.ch();
        let folding = match ch {
            b'|' => false,
            b'>' => true,
            _ => return Ok(false),
        };
        self.kind = Kind::Scalar;
        self.result = Value::Null;
        let mut text = Text::new(self.text);
        let (mut keep, mut strip) = (false, false);
        let mut did_read = false;
        let mut detected = false;
        let mut text_indent = node_indent;
        let mut empty_lines = 0usize;
        let mut at_more_indented = false;
        while ch != 0 {
            self.position += 1;
            ch = self.ch();
            if ch == b'+' || ch == b'-' {
                if keep || strip {
                    return Err(self.error("repeat of a chomping mode identifier"));
                }
                keep = ch == b'+';
                strip = ch == b'-';
            } else if ch.is_ascii_digit() {
                if ch == b'0' {
                    return Err(
                        self.error("bad explicit indentation width of a block scalar; it cannot be less than one")
                    );
                } else if detected {
                    return Err(self.error("repeat of an indentation width identifier"));
                }
                text_indent = node_indent + i64::from(ch - b'0') - 1;
                detected = true;
            } else {
                break;
            }
        }
        if is_ws(ch) {
            while is_ws(ch) {
                self.position += 1;
                ch = self.ch();
            }
            if ch == b'#' {
                while !is_eol(ch) && ch != 0 {
                    self.position += 1;
                    ch = self.ch();
                }
            }
        }
        while ch != 0 {
            self.read_line_break()?;
            self.line_indent = 0;
            ch = self.ch();
            while (!detected || self.line_indent < text_indent) && ch == b' ' {
                self.line_indent += 1;
                self.position += 1;
                ch = self.ch();
            }
            if !detected && self.line_indent > text_indent {
                text_indent = self.line_indent;
            }
            if is_eol(ch) {
                empty_lines += 1;
                continue;
            }
            if !detected && text_indent == 0 {
                return Err(self.error("missing indentation for block scalar"));
            }
            if self.line_indent < text_indent {
                if keep {
                    text.push_newlines(if did_read { 1 + empty_lines } else { empty_lines })?;
                } else if !strip && did_read {
                    text.push_newlines(1)?;
                }
                break;
            }
            if folding {
                if is_ws(ch) {
                    at_more_indented = true;
                    text.push_newlines(if did_read { 1 + empty_lines } else { empty_lines })?;
                } else if at_more_indented {
                    at_more_indented = false;
                    text.push_newlines(empty_lines + 1)?;
                } else if empty_lines == 0 {
                    if did_read {
                        text.push(' ')?;
                    }
                } else {
                    text.push_newlines(empty_lines)?;
                }
            } else {
                text.push_newlines(if did_read { 1 + empty_lines } else { empty_lines })?;
            }
            did_read = true;
            detected = true;
            empty_lines = 0;
            let cap_start = self.position;
            let mut suspicious = false;
            loop {
                self.position = self.scan(self.position, &LINE);
                ch = self.ch();
                if is_eol(ch) || ch == 0 {
                    break;
                }
                suspicious = true;
                self.position += 1;
            }
            self.capture(&mut text, cap_start, self.position, false, suspicious)?;
        }
        self.result = Value::String(text.finish()?);
        Ok(true)
    }

    #[inline(never)]
    fn read_block_sequence(&mut self, node_indent: i64) -> R<bool> {
        if self.first_tab.is_some() {
            return Ok(false);
        }
        let tag = self.tag.clone();
        let anchor = self.anchor;
        let pending = anchor.map(|a| self.store_pending(a));
        let mut items = Vec::new();
        let mut detected = false;
        let mut ch = self.ch();
        while ch != 0 {
            if let Some(tab) = self.first_tab {
                self.position = tab;
                return Err(self.error("tab characters must not be used in indentation"));
            }
            if ch != b'-' || !is_ws_or_eol(self.at(self.position + 1)) {
                break;
            }
            detected = true;
            self.position += 1;
            if self.skip_separation_space(true) > 0 && self.line_indent <= node_indent {
                items.push(Value::Null);
                ch = self.ch();
                continue;
            }
            let line = self.line;
            self.compose_node(node_indent, Ctx::BlockIn, false, true)?;
            items.push(std::mem::replace(&mut self.result, Value::Null));
            self.skip_separation_space(true);
            ch = self.ch();
            if (self.line == line || self.line_indent > node_indent) && ch != 0 {
                return Err(self.error("bad indentation of a sequence entry"));
            } else if self.line_indent < node_indent {
                break;
            }
        }
        if !detected {
            self.settle(pending, |_| Value::Array(Vec::new()));
            return Ok(false);
        }
        self.tag = tag;
        self.anchor = anchor;
        self.kind = Kind::Sequence;
        self.result = Value::Array(items);
        self.settle(pending, |l| l.result.clone());
        Ok(true)
    }

    #[inline(never)]
    fn read_block_mapping(&mut self, node_indent: i64, flow_indent: i64) -> R<bool> {
        if self.first_tab.is_some() {
            return Ok(false);
        }
        let tag = self.tag.clone();
        let anchor = self.anchor;
        let pending = anchor.map(|a| self.store_pending(a));
        let mut map = Map::new();
        let mut overridable = Overridable::default();
        let mut key_tag: Option<Cow<'a, str>> = None;
        let mut key_node = Value::Null;
        let mut value_node = Value::Null;
        let mut at_explicit_key = false;
        let mut detected = false;
        let mut allow_compact;
        let mut ch = self.ch();
        while ch != 0 {
            if !at_explicit_key && let Some(tab) = self.first_tab {
                self.position = tab;
                return Err(self.error("tab characters must not be used in indentation"));
            }
            let following = self.at(self.position + 1);
            let line = self.line;
            if (ch == b'?' || ch == b':') && is_ws_or_eol(following) {
                if ch == b'?' {
                    if at_explicit_key {
                        self.store_mapping_pair(&mut map, &mut overridable, &mut key_tag, &mut key_node, None)?;
                        value_node = Value::Null;
                    }
                    detected = true;
                    at_explicit_key = true;
                    allow_compact = true;
                } else if at_explicit_key {
                    at_explicit_key = false;
                    allow_compact = true;
                } else {
                    return Err(self.error(
                        "incomplete explicit mapping pair; a key node is missed; or followed by a non-tabulated empty line",
                    ));
                }
                self.position += 1;
                ch = following;
            } else {
                if !self.compose_node(flow_indent, Ctx::FlowOut, false, true)? {
                    break;
                }
                if self.line == line {
                    ch = self.ch();
                    while is_ws(ch) {
                        self.position += 1;
                        ch = self.ch();
                    }
                    if ch == b':' {
                        self.position += 1;
                        ch = self.ch();
                        if !is_ws_or_eol(ch) {
                            return Err(self.error(
                                "a whitespace character is expected after the key-value separator within a block mapping",
                            ));
                        }
                        if at_explicit_key {
                            self.store_mapping_pair(&mut map, &mut overridable, &mut key_tag, &mut key_node, None)?;
                            value_node = Value::Null;
                        }
                        detected = true;
                        at_explicit_key = false;
                        allow_compact = false;
                        key_tag = self.tag.take();
                        std::mem::swap(&mut key_node, &mut self.result);
                    } else if detected {
                        return Err(self.error("can not read an implicit mapping pair; a colon is missed"));
                    } else {
                        self.tag = tag;
                        self.anchor = anchor;
                        self.settle(pending, |_| Value::Object(Map::new()));
                        return Ok(true);
                    }
                } else if detected {
                    return Err(
                        self.error("can not read a block mapping entry; a multiline key may not be an implicit key")
                    );
                } else {
                    self.tag = tag;
                    self.anchor = anchor;
                    self.settle(pending, |_| Value::Object(Map::new()));
                    return Ok(true);
                }
            }
            if self.line == line || self.line_indent > node_indent {
                if self.compose_node(node_indent, Ctx::BlockOut, true, allow_compact)? {
                    let slot = if at_explicit_key { &mut key_node } else { &mut value_node };
                    std::mem::swap(slot, &mut self.result);
                }
                if !at_explicit_key {
                    self.store_mapping_pair(
                        &mut map,
                        &mut overridable,
                        &mut key_tag,
                        &mut key_node,
                        Some(&mut value_node),
                    )?;
                }
                self.skip_separation_space(true);
                ch = self.ch();
            }
            if (self.line == line || self.line_indent > node_indent) && ch != 0 {
                return Err(self.error("bad indentation of a mapping entry"));
            } else if self.line_indent < node_indent {
                break;
            }
        }
        if at_explicit_key {
            self.store_mapping_pair(&mut map, &mut overridable, &mut key_tag, &mut key_node, None)?;
        }
        if !detected {
            self.settle(pending, |_| Value::Object(Map::new()));
            return Ok(false);
        }
        self.tag = tag;
        self.anchor = anchor;
        self.kind = Kind::Mapping;
        self.result = Value::Object(map);
        self.settle(pending, |l| l.result.clone());
        Ok(true)
    }

    fn charge_merge_work(&mut self) -> R<()> {
        self.merge_work += 1;
        if self.merge_work > MAX_MERGE_WORK {
            return Err(self.error("merge keys exceeded maxTotalMergeKeys (10000)"));
        }
        Ok(())
    }

    #[inline(never)]
    fn merge_mappings(&mut self, dest: &mut Map, source: &Value, overridable: &mut Overridable) -> R<()> {
        if !matches!(source, Value::Object(_) | Value::Array(_)) {
            return Err(self.error("cannot merge mappings; the provided source object is unacceptable"));
        }
        self.charge_merge_work()?;
        for (key, value) in js_keys(source) {
            self.charge_merge_work()?;
            if !dest.contains_key(&key) {
                overridable.0.get_or_insert_default().insert(key.clone());
                dest.insert(key, value);
            }
        }
        Ok(())
    }

    #[inline(never)]
    fn store_mapping_pair(
        &mut self,
        dest: &mut Map,
        overridable: &mut Overridable,
        key_tag: &mut Option<Cow<'a, str>>,
        key: &mut Value,
        value: Option<&mut Value>,
    ) -> R<()> {
        let key_tag = key_tag.take();
        let key = std::mem::replace(key, Value::Null);
        let value = value.map_or(Value::Null, |v| std::mem::replace(v, Value::Null));
        let key = match key {
            Value::String(s) => s,
            Value::Array(a) => {
                if a.iter().any(|v| matches!(v, Value::Array(_))) {
                    return Err(self.error("nested arrays are not supported inside keys"));
                }
                Value::Array(a).js_string()
            }
            other => other.js_string(),
        };
        if key_tag.as_deref() == Some(MERGE_TAG) {
            if let Value::Array(sources) = &value {
                if sources.len() > 100 {
                    return Err(self.error("abnormal merge sequence size"));
                }
                for source in sources {
                    self.merge_mappings(dest, source, overridable)?;
                }
            } else {
                self.merge_mappings(dest, &value, overridable)?;
            }
            return Ok(());
        }
        let was_overridable = overridable.0.as_mut().is_some_and(|o| o.remove(&key));
        let len = dest.len();
        dest.insert(key, value);
        if !was_overridable && dest.len() == len {
            return Err(self.error("duplicated mapping key"));
        }
        Ok(())
    }

    #[inline(never)]
    fn read_tag_property(&mut self) -> R<bool> {
        if self.ch() != b'!' {
            return Ok(false);
        }
        if self.tag.is_some() {
            return Err(self.error("duplication of a tag property"));
        }
        self.position += 1;
        let mut ch = self.ch();
        let mut verbatim = false;
        let mut named = false;
        let mut handle: &'a str = "!";
        if ch == b'<' {
            verbatim = true;
            self.position += 1;
        } else if ch == b'!' {
            named = true;
            handle = "!!";
            self.position += 1;
        }
        let mut start = self.position;
        let name: &'a str;
        if verbatim {
            loop {
                self.position += 1;
                ch = self.ch();
                if ch == 0 || ch == b'>' {
                    break;
                }
            }
            if self.position >= self.length {
                return Err(self.error("unexpected end of the stream within a verbatim tag"));
            }
            name = self.slice(start, self.position)?;
            self.position += 1;
        } else {
            ch = self.ch();
            while ch != 0 && !is_ws_or_eol(ch) {
                if ch == b'!' {
                    if named {
                        return Err(self.error("tag suffix cannot contain exclamation marks"));
                    }
                    handle = self.slice(start - 1, self.position + 1)?;
                    if !is_tag_handle(handle) {
                        return Err(self.error("named tag handle cannot contain such characters"));
                    }
                    named = true;
                    start = self.position + 1;
                }
                self.position += 1;
                ch = self.ch();
            }
            name = self.slice(start, self.position)?;
            if name.bytes().any(is_flow) {
                return Err(self.error("tag suffix cannot contain flow indicator characters"));
            }
        }
        if !name.is_empty() && !is_tag_uri(name) {
            return Err(self.error(&format!("tag name cannot contain such characters: {name}")));
        }
        let Some(name) = decode_uri(name) else {
            return Err(self.error(&format!("tag name is malformed: {name}")));
        };
        self.tag = Some(if verbatim {
            name
        } else if let Some(prefix) = self.tag_map.get(handle) {
            Cow::Owned(format!("{prefix}{name}"))
        } else if handle == "!" {
            Cow::Owned(format!("!{name}"))
        } else if handle == "!!" {
            Cow::Owned(format!("{YAML_TAG}{name}"))
        } else {
            return Err(self.error(&format!("undeclared tag handle \"{handle}\"")));
        });
        Ok(true)
    }

    fn read_name(&mut self) -> R<&'a str> {
        self.position += 1;
        let start = self.position;
        let mut ch = self.ch();
        while ch != 0 && !is_ws_or_eol(ch) && !is_flow(ch) {
            self.position += 1;
            ch = self.ch();
        }
        self.slice(start, self.position)
    }

    fn read_anchor_property(&mut self) -> R<bool> {
        if self.ch() != b'&' {
            return Ok(false);
        }
        if self.anchor.is_some() {
            return Err(self.error("duplication of an anchor property"));
        }
        let name = self.read_name()?;
        if name.is_empty() {
            return Err(self.error("name of an anchor node must contain at least one character"));
        }
        self.anchor = Some(name);
        Ok(true)
    }

    #[inline(never)]
    fn read_alias(&mut self) -> R<bool> {
        if self.ch() != b'*' {
            return Ok(false);
        }
        let name = self.read_name()?;
        if name.is_empty() {
            return Err(self.error("name of an alias node must contain at least one character"));
        }
        match self.anchors.get(name) {
            None => return Err(self.error(&format!("unidentified alias \"{name}\""))),
            Some(Anchor::Pending(_)) => {
                return Err(self.error(&format!("alias \"{name}\" refers to a collection that is not complete yet")));
            }
            Some(Anchor::Done(v, size)) => {
                self.alias_nodes += size;
                if self.alias_nodes > MAX_ALIAS_NODES {
                    return Err(self.error("aliases expand to too many nodes"));
                }
                self.result = v.clone();
            }
        }
        self.skip_separation_space(true);
        Ok(true)
    }

    #[inline(never)]
    fn try_read_block_mapping_from_property(
        &mut self,
        property_start: Snapshot<'a>,
        node_indent: i64,
        flow_indent: i64,
    ) -> R<bool> {
        let fallback = self.snapshot();
        self.transactions.push(FxHashMap::default());
        self.restore(property_start);
        self.tag = None;
        self.anchor = None;
        self.kind = Kind::None;
        self.result = Value::Null;
        if self.read_block_mapping(node_indent, flow_indent)? && self.kind == Kind::Mapping {
            self.commit_transaction();
            return Ok(true);
        }
        self.rollback_transaction();
        self.restore(fallback);
        self.result = Value::Null;
        Ok(false)
    }

    /// Reads the tag/anchor properties before a node; returns where they started.
    #[inline(never)]
    fn read_properties(
        &mut self,
        parent_indent: i64,
        allow_block_styles: bool,
        at_new_line: &mut bool,
        indent_status: &mut i8,
        allow_block_collections: &mut bool,
    ) -> R<Option<Box<Snapshot<'a>>>> {
        let mut property_start = None;
        loop {
            let ch = self.ch();
            if ch != b'!' && ch != b'&' {
                break;
            }
            if *at_new_line && ((ch == b'!' && self.tag.is_some()) || (ch == b'&' && self.anchor.is_some())) {
                break;
            }
            let snap = self.snapshot();
            if !(self.read_tag_property()? || self.read_anchor_property()?) {
                break;
            }
            if property_start.is_none() {
                property_start = Some(Box::new(snap));
            }
            if self.skip_separation_space(true) > 0 {
                *at_new_line = true;
                *allow_block_collections = allow_block_styles;
                *indent_status = indent_cmp(self.line_indent, parent_indent);
            } else {
                *allow_block_collections = false;
            }
        }
        Ok(property_start)
    }

    /// The non-collection branch of `composeNode`.
    #[inline(never)]
    fn read_scalar_node(
        &mut self,
        property_start: Option<&Snapshot<'a>>,
        allow_block_styles: bool,
        allow_block_collections: bool,
        flow_indent: i64,
        in_flow: bool,
    ) -> R<bool> {
        let ch = self.ch();
        let from_property = if let Some(ps) = property_start
            && allow_block_styles
            && !allow_block_collections
            && ch != b'|'
            && ch != b'>'
        {
            let indent = self.column(ps.position, ps.line_start);
            self.try_read_block_mapping_from_property(ps.clone(), indent, flow_indent)?
        } else {
            false
        };
        let mut has_content = false;
        if from_property
            || (allow_block_styles && self.read_block_scalar(flow_indent)?)
            || self.read_single_quoted_scalar()?
            || self.read_double_quoted_scalar()?
        {
            has_content = true;
        } else if self.read_alias()? {
            has_content = true;
            if self.tag.is_some() || self.anchor.is_some() {
                return Err(self.error("alias node should not have any properties"));
            }
        } else if self.read_plain_scalar(flow_indent, in_flow)? {
            has_content = true;
            if self.tag.is_none() {
                self.tag = Some(Cow::Borrowed("?"));
            }
        }
        if let Some(a) = self.anchor {
            self.store_anchor(a, self.result.clone());
        }
        Ok(has_content)
    }

    #[inline(never)]
    fn compose_node(&mut self, parent_indent: i64, ctx: Ctx, allow_to_seek: bool, allow_compact: bool) -> R<bool> {
        if self.depth >= MAX_DEPTH {
            return Err(self.error(&format!("nesting deeper than {MAX_DEPTH} levels")));
        }
        self.depth += 1;
        self.tag = None;
        self.anchor = None;
        self.kind = Kind::None;
        self.result = Value::Null;
        let allow_block_styles = matches!(ctx, Ctx::BlockOut | Ctx::BlockIn);
        let mut allow_block_collections = allow_block_styles;
        let mut indent_status = 1;
        let mut at_new_line = false;
        let mut has_content = false;
        if allow_to_seek && self.skip_separation_space(true) > 0 {
            at_new_line = true;
            indent_status = indent_cmp(self.line_indent, parent_indent);
        }
        let property_start = if indent_status == 1 && matches!(self.ch(), b'!' | b'&') {
            self.read_properties(
                parent_indent,
                allow_block_styles,
                &mut at_new_line,
                &mut indent_status,
                &mut allow_block_collections,
            )?
        } else {
            None
        };
        if allow_block_collections {
            allow_block_collections = at_new_line || allow_compact;
        }
        if indent_status == 1 || ctx == Ctx::BlockOut {
            let flow_indent = if matches!(ctx, Ctx::FlowIn | Ctx::FlowOut) { parent_indent } else { parent_indent + 1 };
            let block_indent = self.column(self.position, self.line_start);
            if indent_status == 1 {
                has_content = (allow_block_collections
                    && (self.read_block_sequence(block_indent)?
                        || self.read_block_mapping(block_indent, flow_indent)?))
                    || self.read_flow_collection(flow_indent)?
                    || self.read_scalar_node(
                        property_start.as_deref(),
                        allow_block_styles,
                        allow_block_collections,
                        flow_indent,
                        ctx == Ctx::FlowIn,
                    )?;
            } else if indent_status == 0 {
                has_content = allow_block_collections && self.read_block_sequence(block_indent)?;
            }
        }
        if self.tag.is_some() || self.anchor.is_some() {
            self.resolve_tag()?;
        }
        self.depth -= 1;
        Ok(self.tag.is_some() || self.anchor.is_some() || has_content)
    }

    /// The tag handling at the end of `composeNode`.
    #[inline(never)]
    fn resolve_tag(&mut self) -> R<()> {
        match self.tag.as_deref() {
            None => {
                if let Some(a) = self.anchor {
                    self.store_anchor(a, self.result.clone());
                }
            }
            Some("?") => {
                if !matches!(self.result, Value::Null) && self.kind != Kind::Scalar {
                    return Err(self.error(&format!(
                        "unacceptable node kind for !<?> tag; it should be \"scalar\", not \"{}\"",
                        kind_name(self.kind)
                    )));
                }
                let resolved = match &self.result {
                    Value::Null => true,
                    Value::String(s) => {
                        if let Some(v) = super::resolve_implicit(s) {
                            self.result = v;
                            true
                        } else if s == "<<" {
                            self.tag = Some(Cow::Borrowed(MERGE_TAG));
                            true
                        } else {
                            false
                        }
                    }
                    // Only reachable when a probe hands back a resolved key: js-yaml's float type
                    // matches `String(n)`, then its constructor calls `toLowerCase` on a number.
                    Value::Number(n) if n.is_finite() => {
                        return Err(self.error("data.toLowerCase is not a function"));
                    }
                    _ => false,
                };
                if resolved && let Some(a) = self.anchor {
                    self.store_anchor(a, self.result.clone());
                }
            }
            Some("!") => {}
            Some(tag) => {
                let tag = tag.to_owned();
                let value = std::mem::replace(&mut self.result, Value::Null);
                self.result = self.construct_explicit(&tag, value)?;
                if let Some(a) = self.anchor {
                    self.store_anchor(a, self.result.clone());
                }
            }
        }
        Ok(())
    }

    #[inline(never)]
    fn construct_explicit(&self, tag: &str, data: Value) -> R<Value> {
        let name = tag.strip_prefix(YAML_TAG);
        let type_kind = match name {
            Some("null" | "bool" | "int" | "float" | "merge" | "str" | "binary") => Kind::Scalar,
            Some("seq" | "omap" | "pairs") => Kind::Sequence,
            Some("map" | "set") => Kind::Mapping,
            _ => Kind::None,
        };
        if type_kind == Kind::None || (self.kind != Kind::None && self.kind != type_kind) {
            return Err(self.error(&format!("unknown tag !<{tag}>")));
        }
        if !matches!(data, Value::Null) && self.kind != type_kind {
            return Err(self.error(&format!(
                "unacceptable node kind for !<{tag}> tag; it should be \"{}\", not \"{}\"",
                kind_name(type_kind),
                kind_name(self.kind)
            )));
        }
        let unresolved = || self.error(&format!("cannot resolve a node with !<{tag}> explicit tag"));
        let text = match &data {
            Value::String(s) => Some(s.as_str()),
            _ => None,
        };
        Ok(match name.unwrap_or_default() {
            "binary" => return Err(Fail::Fallback),
            "null" => match (&data, text) {
                (Value::Null, _) | (_, Some("~" | "null" | "Null" | "NULL")) => Value::Null,
                _ => return Err(unresolved()),
            },
            "bool" => match text {
                Some("true" | "True" | "TRUE") => Value::Bool(true),
                Some("false" | "False" | "FALSE") => Value::Bool(false),
                _ => return Err(unresolved()),
            },
            "int" => Value::Number(text.and_then(super::resolve_int).ok_or_else(unresolved)?),
            "float" => Value::Number(text.and_then(super::resolve_float).ok_or_else(unresolved)?),
            "merge" => match (&data, text) {
                (Value::Null, _) | (_, Some("<<")) => data,
                _ => return Err(unresolved()),
            },
            "str" if matches!(data, Value::Null) => Value::String(String::new()),
            "seq" | "omap" | "pairs" if matches!(data, Value::Null) => Value::Array(Vec::new()),
            "map" | "set" if matches!(data, Value::Null) => Value::Object(Map::new()),
            "omap" => {
                let Value::Array(items) = &data else { return Err(unresolved()) };
                let mut seen = rustc_hash::FxHashSet::default();
                for item in items {
                    let Value::Object(pair) = item else { return Err(unresolved()) };
                    let mut keys = pair.keys();
                    match (keys.next(), keys.next()) {
                        (Some(k), None) if seen.insert(k.clone()) => {}
                        _ => return Err(unresolved()),
                    }
                }
                data
            }
            "pairs" => {
                let Value::Array(items) = data else { return Err(unresolved()) };
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    let Value::Object(pair) = item else { return Err(unresolved()) };
                    if pair.len() != 1 {
                        return Err(unresolved());
                    }
                    let (k, v) = pair.iter().next().map(|(k, v)| (k.clone(), v.clone())).ok_or_else(unresolved)?;
                    out.push(Value::Array(vec![Value::String(k), v]));
                }
                Value::Array(out)
            }
            "set" => {
                let Value::Object(m) = &data else { return Err(unresolved()) };
                if m.values().any(|v| !matches!(v, Value::Null)) {
                    return Err(unresolved());
                }
                data
            }
            _ => data,
        })
    }

    fn read_directive_args(&mut self) -> R<(&'a str, Vec<&'a str>)> {
        self.position += 1;
        let mut ch = self.ch();
        let mut start = self.position;
        while ch != 0 && !is_ws_or_eol(ch) {
            self.position += 1;
            ch = self.ch();
        }
        let name = self.slice(start, self.position)?;
        let mut args = Vec::new();
        if name.is_empty() {
            return Err(self.error("directive name must not be less than one character in length"));
        }
        while ch != 0 {
            while is_ws(ch) {
                self.position += 1;
                ch = self.ch();
            }
            if ch == b'#' {
                while ch != 0 && !is_eol(ch) {
                    self.position += 1;
                    ch = self.ch();
                }
                break;
            }
            if is_eol(ch) {
                break;
            }
            start = self.position;
            while ch != 0 && !is_ws_or_eol(ch) {
                self.position += 1;
                ch = self.ch();
            }
            args.push(self.slice(start, self.position)?);
        }
        if ch != 0 {
            self.read_line_break()?;
        }
        Ok((name, args))
    }

    fn directive(&mut self, name: &str, args: &[&'a str]) -> R<()> {
        match name {
            "YAML" => {
                if self.version {
                    return Err(self.error("duplication of %YAML directive"));
                }
                let [arg] = args else {
                    return Err(self.error("YAML directive accepts exactly one argument"));
                };
                let Some((major, minor)) = arg.split_once('.').filter(|(a, b)| {
                    !a.is_empty() && !b.is_empty() && a.bytes().chain(b.bytes()).all(|c| c.is_ascii_digit())
                }) else {
                    return Err(self.error("ill-formed argument of the YAML directive"));
                };
                let _ = minor;
                if major.trim_start_matches('0') != "1" {
                    return Err(self.error("unacceptable YAML version of the document"));
                }
                self.version = true;
            }
            "TAG" => {
                let [handle, prefix] = args else {
                    return Err(self.error("TAG directive accepts exactly two arguments"));
                };
                if !is_tag_handle(handle) {
                    return Err(self.error("ill-formed tag handle (first argument) of the TAG directive"));
                }
                if self.tag_map.contains_key(handle) {
                    return Err(
                        self.error(&format!("there is a previously declared suffix for \"{handle}\" tag handle"))
                    );
                }
                if !is_tag_uri(prefix) {
                    return Err(self.error("ill-formed tag prefix (second argument) of the TAG directive"));
                }
                let Some(decoded) = decode_uri(prefix) else {
                    return Err(self.error(&format!("tag prefix is malformed: {prefix}")));
                };
                self.tag_map.insert(handle, decoded.into_owned());
            }
            _ => {}
        }
        Ok(())
    }

    fn read_document(&mut self) -> R<Value> {
        self.version = false;
        self.tag_map.clear();
        self.anchors.clear();
        let mut has_directives = false;
        while self.ch() != 0 {
            self.skip_separation_space(true);
            if self.line_indent > 0 || self.ch() != b'%' {
                break;
            }
            has_directives = true;
            let (name, args) = self.read_directive_args()?;
            self.directive(name, &args)?;
        }
        self.skip_separation_space(true);
        if self.line_indent == 0
            && self.ch() == b'-'
            && self.at(self.position + 1) == b'-'
            && self.at(self.position + 2) == b'-'
        {
            self.position += 3;
            self.skip_separation_space(true);
        } else if has_directives {
            return Err(self.error("directives end mark is expected"));
        }
        self.compose_node(self.line_indent - 1, Ctx::BlockOut, false, true)?;
        self.skip_separation_space(true);
        let doc = std::mem::replace(&mut self.result, Value::Null);
        if self.position == self.line_start && self.test_document_separator() {
            if self.ch() == b'.' {
                self.position += 3;
                self.skip_separation_space(true);
            }
            return Ok(doc);
        }
        if self.position + 1 < self.length {
            return Err(self.error("end of the stream or a document separator is expected"));
        }
        Ok(doc)
    }
}

fn indent_cmp(line_indent: i64, parent_indent: i64) -> i8 {
    match line_indent.cmp(&parent_indent) {
        std::cmp::Ordering::Greater => 1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Less => -1,
    }
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::None => "null",
        Kind::Scalar => "scalar",
        Kind::Sequence => "sequence",
        Kind::Mapping => "mapping",
    }
}

/// js-yaml's `charFromCodepoint` on an int32 escape value, including its truncation.
fn push_code_point(text: &mut Text<'_>, code: i32) -> R<()> {
    let unit = |v: i64| u32::try_from(v.rem_euclid(0x1_0000)).unwrap_or(0);
    if code <= 0xffff {
        return text.push_unit(unit(i64::from(code)));
    }
    let rel = code - 0x1_0000;
    text.push_unit(unit(i64::from(rel >> 10) + 0xd800))?;
    text.push_unit(unit(i64::from(rel & 0x3ff) + 0xdc00))
}

/// `js-yaml.load`: one document, or `null` for an empty stream.
pub fn load(source: &str) -> R<Value> {
    let text = source.strip_prefix('\u{feff}').unwrap_or(source);
    if text.as_bytes().contains(&0) {
        let p = text.bytes().position(|b| b == 0).unwrap_or(0);
        let line = text[..p].matches('\n').count() + 1;
        return Err(Fail::Error(format!("null byte is not allowed in input (line {line})")));
    }
    let src = text.as_bytes();
    let tail = match src.last() {
        None | Some(b'\n' | b'\r') => 0,
        Some(_) => b'\n',
    };
    let mut loader = Loader {
        text,
        src,
        tail,
        length: src.len() + usize::from(tail != 0),
        position: 0,
        line: 0,
        line_start: 0,
        line_indent: 0,
        first_tab: None,
        depth: 0,
        merge_work: 0,
        alias_nodes: 0,
        tag: None,
        anchor: None,
        kind: Kind::None,
        result: Value::Null,
        anchors: FxHashMap::default(),
        transactions: Vec::new(),
        tag_map: FxHashMap::default(),
        version: false,
        next_pending: 0,
    };
    while loader.ch() == b' ' {
        loader.line_indent += 1;
        loader.position += 1;
    }
    let mut doc = None;
    while loader.position + 1 < loader.length {
        if doc.is_some() {
            return Err(Fail::Error("expected a single document in the stream, but found more".into()));
        }
        doc = Some(loader.read_document()?);
    }
    Ok(doc.unwrap_or(Value::Null))
}

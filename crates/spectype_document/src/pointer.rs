//! JSON Pointer helpers that copy `@redocly/openapi-core`'s `ref-utils`, quirks included.

use std::borrow::Cow;
use std::convert::Infallible;
use std::fmt;
use std::ops::Deref;
use std::str::FromStr;

use crate::value::Value;

/// An escaped JSON Pointer fragment such as `#/paths/~1users/get`, as Redocly and
/// openapi-typescript pass them around.
///
/// It is not validated: upstream never rejects a pointer, so neither does this. It derefs to the
/// escaped text, which is what every message and hook receives.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JsonPointer(String);

impl JsonPointer {
    /// The escaped pointer text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The escaped pointer text, without copying it.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }

    /// Appends `segment`, escaped, the way Redocly's `Location.child` joins: an empty pointer
    /// becomes `#/` first, and no `/` is added after a pointer that already ends in one.
    pub fn push(&mut self, segment: &str) {
        if self.0.is_empty() {
            self.0.push_str("#/");
        } else if !self.0.ends_with('/') {
            self.0.push('/');
        }
        push_escaped(&mut self.0, segment);
    }

    /// The unescaped segments, as `parseRef(pointer).pointer` reads them: only the part after
    /// the first `#/`, empty segments dropped, and `%xx` decoded like `decodeURIComponent`.
    pub fn segments(&self) -> impl Iterator<Item = Cow<'_, str>> + '_ {
        segments(&self.0)
    }
}

impl Deref for JsonPointer {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for JsonPointer {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for JsonPointer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for JsonPointer {
    /// Takes already escaped pointer text as is.
    fn from(pointer: String) -> Self {
        Self(pointer)
    }
}

impl From<&str> for JsonPointer {
    /// Takes already escaped pointer text as is.
    fn from(pointer: &str) -> Self {
        Self(pointer.to_owned())
    }
}

impl From<&Self> for JsonPointer {
    fn from(pointer: &Self) -> Self {
        pointer.clone()
    }
}

impl From<JsonPointer> for String {
    fn from(pointer: JsonPointer) -> Self {
        pointer.0
    }
}

impl FromStr for JsonPointer {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(s.into())
    }
}

impl PartialEq<str> for JsonPointer {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for JsonPointer {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

/// `escapePointer`: `~` to `~0`, then `/` to `~1`.
#[must_use]
pub fn escape_pointer(fragment: &str) -> String {
    if !fragment.contains(['~', '/']) {
        return fragment.to_owned();
    }
    fragment.replace('~', "~0").replace('/', "~1")
}

/// `decodeURIComponent`, returning `None` where JS would throw a `URIError`.
#[must_use]
pub fn decode_uri_component(s: &str) -> Option<String> {
    if !s.contains('%') {
        return Some(s.to_owned());
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    let hex = |b: u8| char::from(b).to_digit(16);
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let byte = bytes.get(i + 1).zip(bytes.get(i + 2)).and_then(|(&h, &l)| Some(hex(h)? * 16 + hex(l)?))?;
            out.push(u8::try_from(byte).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// `unescapePointer`; a fragment JS could not decode is returned as is instead of throwing.
#[must_use]
pub fn unescape_pointer(fragment: &str) -> Cow<'_, str> {
    if !fragment.bytes().any(|b| b == b'~' || b == b'%') {
        return Cow::Borrowed(fragment);
    }
    let mut s = String::with_capacity(fragment.len());
    let mut rest = fragment;
    while let Some(i) = rest.find('~') {
        s.push_str(&rest[..i]);
        match rest.as_bytes().get(i + 1) {
            Some(b'1') => s.push('/'),
            Some(b'0') => s.push('~'),
            _ => {
                s.push('~');
                rest = &rest[i + 1..];
                continue;
            }
        }
        rest = &rest[i + 2..];
    }
    s.push_str(rest);
    Cow::Owned(decode_uri_component(&s).unwrap_or(s))
}

/// `parseRef(ref).pointer`.
#[must_use]
pub fn parse_ref(r: &str) -> Vec<Cow<'_, str>> {
    segments(r).collect()
}

fn segments(r: &str) -> impl Iterator<Item = Cow<'_, str>> + '_ {
    let pointer = find_hash_slash(r).map_or("", |i| {
        let rest = &r[i + 2..];
        find_hash_slash(rest).map_or(rest, |j| &rest[..j])
    });
    pointer.split('/').map(unescape_pointer).filter(|s| !s.is_empty())
}

fn find_hash_slash(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    s.match_indices('#').map(|(i, _)| i).find(|&i| b.get(i + 1) == Some(&b'/'))
}

/// `createRef(parts)`: joins parts into a `#/...` pointer, re-parsing parts that are pointers
/// (any part containing `#/`), so a part may itself be a [`JsonPointer`].
#[must_use]
pub fn create_ref<S: AsRef<str>>(parts: &[S]) -> JsonPointer {
    let mut pointer = String::with_capacity(1 + parts.iter().map(|p| p.as_ref().len() + 1).sum::<usize>());
    pointer.push('#');
    for part in parts {
        let part = part.as_ref();
        if part.is_empty() {
            continue;
        }
        if !(part.as_bytes().contains(&b'#') && part.contains("#/")) {
            pointer.push('/');
            push_escaped(&mut pointer, part);
        } else if let Some(rest) = part.strip_prefix("#/").filter(|rest| reparses_to_itself(rest)) {
            pointer.push('/');
            pointer.push_str(rest);
        } else {
            let nested = parse_ref(part);
            if nested.is_empty() {
                pointer.push('/');
                push_escaped(&mut pointer, part);
            }
            for p in nested {
                pointer.push('/');
                push_escaped(&mut pointer, &p);
            }
        }
    }
    JsonPointer(pointer)
}

fn push_escaped(out: &mut String, s: &str) {
    let mut plain = 0;
    for (i, b) in s.bytes().enumerate() {
        let escaped = match b {
            b'~' => "~0",
            b'/' => "~1",
            _ => continue,
        };
        out.push_str(&s[plain..i]);
        out.push_str(escaped);
        plain = i + 1;
    }
    out.push_str(&s[plain..]);
}

#[expect(clippy::needless_bitwise_bool, reason = "branchless fold over the bytes")]
fn reparses_to_itself(rest: &str) -> bool {
    let b = rest.as_bytes();
    let (Some(&first), Some(&last)) = (b.first(), b.last()) else { return false };
    let bad = b.iter().zip(&b[1..]).fold(false, |bad, (&c, &n)| {
        bad | (c == b'%') | ((n == b'/') & ((c == b'/') | (c == b'#'))) | ((c == b'~') & (n != b'0') & (n != b'1'))
    });
    !bad && first != b'/' && !matches!(last, b'/' | b'~' | b'%')
}

/// The result of following a `$ref`, mirroring openapi-typescript's `resolveRef`.
pub enum Resolved<'a> {
    /// The node the `$ref` chain ends at.
    Found(&'a Value),
    /// A step of the pointer is missing or falsy.
    Missing,
    /// The chain comes back to a `$ref` it already followed.
    Circular,
}

/// `resolveRef(schema, $ref)`: walks the pointer, requiring every step to be truthy, and follows
/// chained `$ref`s.
#[must_use]
pub fn resolve_ref<'a>(root: &'a Value, r: &str) -> Resolved<'a> {
    let mut visited: Vec<String> = Vec::new();
    let mut current = Cow::Borrowed(r);
    loop {
        let pointer = parse_ref(&current);
        if pointer.is_empty() {
            return Resolved::Missing;
        }
        let mut node = root;
        for key in &pointer {
            match node.index(key) {
                Some(next) if next.truthy() => node = next,
                _ => return Resolved::Missing,
            }
        }
        match node.get("$ref") {
            Some(next) if next.truthy() => {
                let next = next.js_string();
                if visited.contains(&next) {
                    return Resolved::Circular;
                }
                visited.push(next.clone());
                current = Cow::Owned(next);
            }
            _ => return Resolved::Found(node),
        }
    }
}

/// The pointer segments of the node `resolve_ref` lands on, for callers that need to mutate it.
#[must_use]
pub fn resolve_ref_path(root: &Value, r: &str) -> Option<Vec<String>> {
    let mut visited: Vec<String> = Vec::new();
    let mut current = Cow::Borrowed(r);
    loop {
        let pointer = parse_ref(&current);
        if pointer.is_empty() {
            return None;
        }
        let mut node = root;
        for key in &pointer {
            node = node.index(key).filter(|n| n.truthy())?;
        }
        match node.get("$ref") {
            Some(next) if next.truthy() => {
                let next = next.js_string();
                if visited.contains(&next) {
                    return None;
                }
                visited.push(next.clone());
                current = Cow::Owned(next);
            }
            _ => return Some(pointer.into_iter().map(Cow::into_owned).collect()),
        }
    }
}

/// The node at `path`, a list of unescaped segments, for mutation.
pub fn get_path_mut<'a>(root: &'a mut Value, path: &[String]) -> Option<&'a mut Value> {
    let mut node = root;
    for key in path {
        node = node.index_mut(key)?;
    }
    Some(node)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_ref_reparses_pointers() {
        assert_eq!(create_ref(&["paths", "/users/{id}"]), "#/paths/~1users~1{id}");
        assert_eq!(create_ref(&["#/paths/~1a%20b", "get"]), "#/paths/~1a b/get");
        assert_eq!(create_ref(&["#/components/schemas/Foo", "", "bar"]), "#/components/schemas/Foo/bar");
        assert_eq!(parse_ref("#/a#/b"), ["a"]);
        assert_eq!(parse_ref("other.yaml#/x/y~1z"), ["x", "y/z"]);
        assert_eq!(unescape_pointer("100%"), "100%");
        assert_eq!(create_ref(&["#/a~0~1b/~01", "c/d"]), "#/a~0~1b/~01/c~1d");
        assert_eq!(create_ref(&["#/a~b", "c"]), "#/a~0b/c");
        assert_eq!(create_ref(&["#/a#/b", "c"]), "#/a/c");
        assert_eq!(create_ref(&["#/a//b/", "c"]), "#/a/b/c");
        assert_eq!(create_ref(&["#/", "c"]), "#/#~1/c");
        assert_eq!(create_ref(&["#/a~", "c"]), "#/a~0/c");
        assert_eq!(create_ref(&["#/a%25", "c"]), "#/a%/c");
    }

    #[test]
    fn push_joins_like_redocly_locations() {
        let mut p = JsonPointer::default();
        p.push("a/b~");
        assert_eq!(p, "#/a~1b~0");
        let mut p = JsonPointer::from("#/");
        p.push("x");
        p.push("");
        p.push("y");
        assert_eq!(p, "#/x/y");
        assert_eq!(p.segments().collect::<Vec<_>>(), ["x", "y"]);
    }
}

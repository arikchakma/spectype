//! `resolve.http.headers`: the headers Redocly's `readFileFromUrl` sends with each request.

use spectype_document::value::Value;

/// One `resolve.http.headers` entry.
#[derive(Debug, Clone, Default)]
pub struct HttpHeader {
    /// A minimatch glob over the URL; without `http(s)://` it is matched against the URL with
    /// its protocol removed.
    pub matches: String,
    /// The header name.
    pub name: String,
    /// The header value, read from `envVariable` when the entry has one.
    pub value: String,
}

/// Reads `resolve.http.headers` from a Redocly `resolve` object. An entry with `envVariable`
/// takes that variable's value, or `""` when it is unset.
#[must_use]
pub fn headers_of(resolve: Option<&Value>) -> Vec<HttpHeader> {
    let Some(Value::Array(list)) = resolve.and_then(|r| r.get("http")).and_then(|h| h.get("headers")) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|item| {
            let matches = item.get("matches")?.as_str()?.to_owned();
            let name = item.get("name").map_or_else(|| "undefined".into(), Value::js_string);
            let value = match item.get("envVariable") {
                Some(var) => std::env::var(var.js_string()).unwrap_or_default(),
                None => item.get("value").map_or_else(|| "undefined".into(), Value::js_string),
            };
            Some(HttpHeader { matches, name, value })
        })
        .collect()
}

/// The headers to send with a request for `url`. A later entry with the same name replaces the
/// value of an earlier one.
#[must_use]
pub fn request_headers(url: &str, headers: &[HttpHeader]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for header in headers.iter().filter(|h| url_matches(url, &h.matches)) {
        match out.iter_mut().find(|(name, _)| *name == header.name) {
            Some(existing) => existing.1.clone_from(&header.value),
            None => out.push((header.name.clone(), header.value.clone())),
        }
    }
    out
}

fn url_matches(url: &str, pattern: &str) -> bool {
    let has_protocol = |s: &str| s.starts_with("http://") || s.starts_with("https://");
    let url = if has_protocol(pattern) {
        url
    } else {
        url.split_once("://").filter(|_| has_protocol(url)).map_or(url, |(_, rest)| rest)
    };
    minimatch(url, pattern)
}

/// `minimatch(text, pattern)` with default options: brace lists, `**` path segments, and `*`,
/// `?` and `[…]` that do not match a leading `.`.
fn minimatch(text: &str, pattern: &str) -> bool {
    if pattern.starts_with('#') {
        return false;
    }
    let bangs = pattern.bytes().take_while(|&b| b == b'!').count();
    let pattern = &pattern[bangs..];
    let segments: Vec<&str> = text.split('/').collect();
    let found = expand_braces(pattern).iter().any(|p| match_segments(&p.split('/').collect::<Vec<_>>(), &segments));
    found != (bangs % 2 == 1)
}

fn expand_braces(pattern: &str) -> Vec<String> {
    let bytes = pattern.as_bytes();
    let Some(open) = bytes.iter().position(|&b| b == b'{') else { return vec![pattern.to_owned()] };
    let (mut depth, mut commas, mut close) = (0, Vec::new(), None);
    for (i, &b) in bytes.iter().enumerate().skip(open) {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(i);
                    break;
                }
            }
            b',' if depth == 1 => commas.push(i),
            _ => {}
        }
    }
    let Some(close) = close.filter(|_| !commas.is_empty()) else {
        let rest = expand_braces(&pattern[open + 1..]);
        return rest.into_iter().map(|r| format!("{}{{{r}", &pattern[..open])).collect();
    };
    let bounds = std::iter::once(open).chain(commas).chain(std::iter::once(close)).collect::<Vec<_>>();
    let tails = expand_braces(&pattern[close + 1..]);
    let mut out = Vec::new();
    for pair in bounds.windows(2) {
        for option in expand_braces(&pattern[pair[0] + 1..pair[1]]) {
            for tail in &tails {
                out.push(format!("{}{option}{tail}", &pattern[..open]));
            }
        }
    }
    out
}

fn match_segments(pattern: &[&str], text: &[&str]) -> bool {
    match pattern.split_first() {
        None => text.is_empty() || text == [""],
        Some((&"**", [])) => !text.is_empty() && text.iter().all(|s| !s.starts_with('.')),
        Some((&"**", rest)) => (0..=text.len())
            .take_while(|&i| i == 0 || !text[i - 1].starts_with('.'))
            .any(|i| match_segments(rest, &text[i..])),
        Some((segment, rest)) => text
            .split_first()
            .is_some_and(|(first, others)| match_segment(segment, first) && match_segments(rest, others)),
    }
}

fn match_segment(pattern: &str, text: &str) -> bool {
    if text.starts_with('.') && !pattern.starts_with('.') {
        return false;
    }
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    wildcard(&p, &t)
}

fn wildcard(p: &[char], t: &[char]) -> bool {
    match p.split_first() {
        None => t.is_empty(),
        Some(('*', rest)) => (0..=t.len()).any(|i| wildcard(rest, &t[i..])),
        Some(('?', rest)) => !t.is_empty() && wildcard(rest, &t[1..]),
        Some(('[', rest)) => match class(rest, t.first().copied()) {
            Some((true, after)) => wildcard(after, &t[1..]),
            Some((false, _)) => false,
            None => t.first() == Some(&'[') && wildcard(rest, &t[1..]),
        },
        Some(('\\', rest)) if !rest.is_empty() => t.first() == Some(&rest[0]) && wildcard(&rest[1..], &t[1..]),
        Some((c, rest)) => t.first() == Some(c) && wildcard(rest, &t[1..]),
    }
}

/// A `[…]` class after its `[`: whether `c` is in it and the pattern after `]`, or `None` when
/// the class is not closed.
fn class(p: &[char], c: Option<char>) -> Option<(bool, &[char])> {
    let negated = matches!(p.first(), Some('!' | '^'));
    let body_start = usize::from(negated);
    let close = p.iter().skip(body_start + 1).position(|&x| x == ']')? + body_start + 1;
    let body = &p[body_start..close];
    let Some(c) = c else { return Some((false, &p[close + 1..])) };
    let mut found = false;
    let mut i = 0;
    while i < body.len() {
        if i + 2 < body.len() && body[i + 1] == '-' {
            found |= body[i] <= c && c <= body[i + 2];
            i += 3;
        } else {
            found |= body[i] == c;
            i += 1;
        }
    }
    Some((found != negated, &p[close + 1..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(matches: &str, name: &str, value: &str) -> HttpHeader {
        HttpHeader { matches: matches.into(), name: name.into(), value: value.into() }
    }

    #[test]
    fn matches_like_minimatch() {
        let cases = [
            ("api.example.com/registry/a/b.yaml", "api.example.com/**", true),
            ("api.example.com", "api.example.com/**", false),
            ("api.example.com/", "api.example.com/**", true),
            ("api.example.com/a/", "api.example.com/*", true),
            ("api.example.com/b", "api.example.com/**/b", true),
            ("api.example.com/a", "api.example.com/[a", false),
            ("api.example.com/a.yaml", "*.example.com/*.yaml", true),
            ("api.example.com/a/b.yaml", "*.example.com/*.yaml", false),
            ("api.example.com/.hidden", "api.example.com/*", false),
            ("api.example.com/.hidden/x", "api.example.com/**", false),
            ("a.example.com/x", "{a,b}.example.com/x", true),
            ("c.example.com/x", "{a,b}.example.com/x", false),
            ("v1.example.com/x", "v[0-9].example.com/x", true),
            ("va.example.com/x", "v[!0-9].example.com/x", true),
            ("v1.example.com/x", "!v1.example.com/x", false),
            ("v2.example.com/x", "!v1.example.com/x", true),
            ("x.yaml", "#x.yaml", false),
        ];
        for (text, pattern, expected) in cases {
            assert_eq!(minimatch(text, pattern), expected, "{pattern} vs {text}");
        }
    }

    #[test]
    fn strips_the_protocol_unless_the_pattern_has_one() {
        let headers = [
            header("api.example.com/**", "Authorization", "Bearer a"),
            header("https://api.example.com/private/**", "X-Private", "1"),
            header("http://api.example.com/**", "X-Plain", "1"),
            header("**", "Authorization", "Bearer b"),
        ];
        let got = request_headers("https://api.example.com/private/openapi.yaml", &headers);
        assert_eq!(got, [("Authorization".into(), "Bearer b".into()), ("X-Private".into(), "1".into())]);
    }
}

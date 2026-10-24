//! Identifier rules copied from openapi-typescript and `change-case`.

use std::sync::LazyLock;

use regex::Regex;

use crate::ts::PropName;
use spectype_document::value::{js_number, js_string_to_number, js_trim};

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == '$'
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
}

/// `JS_PROPERTY_INDEX_RE`: `/^[A-Za-z_$][A-Za-z_$0-9]*$/`.
#[must_use]
#[expect(clippy::needless_bitwise_bool, reason = "branchless fold over the bytes")]
pub fn is_js_identifier(s: &str) -> bool {
    let b = s.as_bytes();
    b.first().is_some_and(|&c| is_ident_start(c.into()))
        && b.iter().fold(true, |acc, &c| acc & (c.is_ascii_alphanumeric() | (c == b'_') | (c == b'$')))
}

/// `tsPropertyIndex`.
#[must_use]
pub fn property_index(index: &str) -> PropName {
    let maybe_number = index.starts_with(|c: char| c.is_ascii_digit() || c == 'I' || c == 'N');
    if maybe_number && js_number(js_string_to_number(index)) == index {
        PropName::Num(index.to_owned())
    } else if is_js_identifier(index) {
        PropName::Ident(index.to_owned())
    } else {
        PropName::Str(index.to_owned())
    }
}

/// `Number(s[0]) >= 0`, where `s[0]` is the first UTF-16 code unit.
fn starts_like_number(s: &str) -> bool {
    match s.chars().next() {
        Some(c) if c.len_utf16() == 1 => js_string_to_number(&c.to_string()) >= 0.0,
        _ => false,
    }
}

fn upper_first_unit(s: &str, upper: bool) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.len_utf16() == 1 => {
            let head: String = if upper { c.to_uppercase().collect() } else { c.to_lowercase().collect() };
            head + chars.as_str()
        }
        _ => s.to_owned(),
    }
}

/// Tracks `lastIndex` of openapi-typescript's global `JS_PROPERTY_INDEX_INVALID_CHARS_RE`, which
/// leaks between `.test()` calls and changes how trailing invalid characters are sanitized.
#[derive(Debug, Default)]
pub struct InvalidCharsRegexState {
    last_index: usize,
}

impl InvalidCharsRegexState {
    /// `JS_PROPERTY_INDEX_INVALID_CHARS_RE.test(unit)` on a one-code-unit string.
    fn test(&mut self, invalid: bool) -> bool {
        if self.last_index == 0 && invalid {
            self.last_index = 1;
            true
        } else {
            self.last_index = 0;
            false
        }
    }

    fn reset(&mut self) {
        self.last_index = 0;
    }

    /// `sanitizeMemberName`.
    pub fn sanitize_member_name(&mut self, name: &str) -> String {
        if starts_like_number(name) {
            self.replace_invalid_runs(name);
            return format!("Value{name}");
        }
        self.replace_invalid_runs(name)
    }

    fn replace_invalid_runs(&mut self, name: &str) -> String {
        let mut out = String::with_capacity(name.len());
        let mut chars = name.chars().peekable();
        while let Some(c) = chars.next() {
            if is_ident_char(c) {
                out.push(c);
                continue;
            }
            let mut last = c;
            while let Some(&n) = chars.peek() {
                if is_ident_char(n) {
                    break;
                }
                last = n;
                chars.next();
            }
            match chars.next() {
                Some(captured) => {
                    self.test(false);
                    out.extend(captured.to_uppercase());
                }
                None => {
                    if !self.test(true) {
                        if last.len_utf16() == 1 {
                            out.extend(last.to_uppercase());
                        } else {
                            // Only the trailing low surrogate would survive in JS; it cannot be
                            // represented in UTF-8, so the whole character is dropped.
                        }
                    }
                }
            }
        }
        out
    }

    /// The enum declaration name `tsEnum` derives.
    pub fn enum_name(&mut self, name: &str) -> String {
        upper_first_unit(&self.sanitize_member_name(name), true)
    }

    /// The variable name `tsArrayLiteralExpression` derives.
    pub fn array_name(&mut self, name: &str) -> String {
        upper_first_unit(&self.sanitize_member_name(name), false)
    }

    /// The member name `tsEnumMember` derives.
    pub fn enum_member_name(&mut self, raw: &str) -> String {
        if is_js_identifier(raw) {
            return raw.to_owned();
        }
        let mut name = raw.to_owned();
        if starts_like_number(&name) {
            name = format!("Value{name}").replacen('.', "_", 1);
        } else if let Some(rest) = name.strip_prefix('-') {
            name = format!("ValueMinus{rest}");
        }
        self.reset();
        let first_run: Option<String> = {
            let mut run = String::new();
            let mut started = false;
            for c in name.chars() {
                if !is_ident_char(c) {
                    started = true;
                    run.push(c);
                } else if started {
                    break;
                }
            }
            started.then_some(run)
        };
        match first_run {
            Some(run) if run == name => format!("\"{name}\""),
            Some(_) => {
                let mut out = String::with_capacity(name.len());
                let mut run = String::new();
                for c in name.chars() {
                    if is_ident_char(c) {
                        if !run.is_empty() {
                            out.push_str(if run == "+" { "Plus" } else { "_" });
                            run.clear();
                        }
                        out.push(c);
                    } else {
                        run.push(c);
                    }
                }
                if !run.is_empty() {
                    out.push_str(if run == "+" { "Plus" } else { "_" });
                }
                out
            }
            None => name,
        }
    }
}

static SPLIT_LOWER_UPPER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([\p{Ll}0-9])(\p{Lu})").expect("valid regex"));
static SPLIT_UPPER_UPPER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\p{Lu})([\p{Lu}][\p{Ll}])").expect("valid regex"));
static STRIP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)[^\p{L}0-9]+").expect("valid regex"));

/// `split` from change-case 5.
fn split_words(value: &str) -> Vec<String> {
    let result = js_trim(value);
    if result.is_ascii() { split_ascii_words(result.as_bytes()) } else { split_words_regex(result) }
}

fn split_words_regex(result: &str) -> Vec<String> {
    let result = SPLIT_LOWER_UPPER.replace_all(result, "$1\0$2");
    let result = SPLIT_UPPER_UPPER.replace_all(&result, "$1\0$2");
    let result = STRIP.replace_all(&result, "\0");
    let trimmed = result.trim_matches('\0');
    if trimmed.is_empty() {
        return Vec::new();
    }
    trimmed.split('\0').map(str::to_owned).collect()
}

/// [`split_words`] for ASCII input, where the regexes reduce to byte classes.
fn split_ascii_words(b: &[u8]) -> Vec<String> {
    let mut marked: Vec<u8> = Vec::with_capacity(b.len() + 8);
    for (i, &c) in b.iter().enumerate() {
        if i > 0 && c.is_ascii_uppercase() && (b[i - 1].is_ascii_lowercase() || b[i - 1].is_ascii_digit()) {
            marked.push(0);
        }
        marked.push(c);
    }
    let mut words = Vec::new();
    let mut word = String::new();
    for (i, &c) in marked.iter().enumerate() {
        if c.is_ascii_alphanumeric() {
            if c.is_ascii_uppercase()
                && i > 0
                && marked[i - 1].is_ascii_uppercase()
                && marked.get(i + 1).is_some_and(u8::is_ascii_lowercase)
            {
                words.push(std::mem::take(&mut word));
            }
            word.push(char::from(c));
        } else if !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

/// `pascalCase` from change-case 5.
#[must_use]
pub fn pascal_case(input: &str) -> String {
    let mut out = String::new();
    for (i, word) in split_words(input).iter().enumerate() {
        let mut chars = word.chars();
        let Some(first) = chars.next() else { continue };
        if first.len_utf16() == 2 {
            out.push(first);
        } else if i > 0 && first.is_ascii_digit() {
            out.push('_');
            out.push(first);
        } else {
            out.extend(first.to_uppercase());
        }
        out.push_str(&chars.as_str().to_lowercase());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pascal_case_matches_change_case() {
        assert_eq!(pascal_case("user_profile"), "UserProfile");
        assert_eq!(pascal_case("XMLHttpRequest"), "XmlHttpRequest");
        assert_eq!(pascal_case("foo.bar-baz"), "FooBarBaz");
        assert_eq!(pascal_case("v1.2"), "V1_2");
        assert_eq!(pascal_case("Foo_Bar"), "FooBar");
        assert_eq!(pascal_case("APIKey"), "ApiKey");
        assert_eq!(pascal_case("requestBody"), "RequestBody");
        assert_eq!(pascal_case("pathItem"), "PathItem");
    }

    #[test]
    fn ascii_split_matches_regex_split() {
        for s in [
            "",
            "_",
            "a",
            "A",
            "ABC",
            "ABCd",
            "ABCDef",
            "AAaAa",
            "aBcD",
            "1A2B",
            "a1B",
            "XMLHttpRequest",
            "v1.2",
            "foo--bar__Baz",
            "-x-",
            "getHTTP2Response",
            "Foo$Bar",
            "a\0B",
            "UPPER_snake",
            "x9Y9z",
            "AbC1dEF2gh",
        ] {
            assert_eq!(split_ascii_words(s.as_bytes()), split_words_regex(s), "{s:?}");
        }
        let alphabet = b"aZbY09_-. \0";
        let mut seed = 1u32;
        for _ in 0..5000 {
            let mut s = String::new();
            for _ in 0..8 {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                s.push(char::from(alphabet[(seed >> 16) as usize % alphabet.len()]));
            }
            assert_eq!(split_ascii_words(s.as_bytes()), split_words_regex(&s), "{s:?}");
        }
    }

    #[test]
    fn sanitizes_like_openapi_typescript() {
        let mut s = InvalidCharsRegexState::default();
        assert_eq!(
            s.array_name("paths//v1/workspaces/{workspace_slug}/files/get/parameters/query/typeValues"),
            "pathsV1WorkspacesWorkspace_slugFilesGetParametersQueryTypeValues"
        );
        assert_eq!(s.enum_name("/Status"), "Status");
        assert_eq!(s.enum_name("/foo-"), "Foo");
        assert_eq!(s.enum_name("bar-"), "Bar-");
        assert_eq!(s.enum_name("1abc"), "Value1abc");
        assert_eq!(s.enum_member_name("a-b"), "a_b");
        assert_eq!(s.enum_member_name("1.5"), "Value1_5");
        assert_eq!(s.enum_member_name("-1"), "ValueMinus1");
        assert_eq!(s.enum_member_name("a+"), "aPlus");
        assert_eq!(s.enum_member_name("+"), "\"+\"");
        assert_eq!(s.enum_member_name(""), "");
    }
}

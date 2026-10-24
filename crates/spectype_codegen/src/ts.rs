//! A small TypeScript type IR and a printer that reproduces the TypeScript compiler's printer
//! output for the node shapes openapi-typescript builds.

use std::fmt::Write as _;

use spectype_document::value::js_trim;

/// A TypeScript type node.
#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    /// A keyword type: `string`, `number`, `boolean`, `unknown`, `never`, `undefined`.
    Keyword(&'static str),
    /// The `null` literal type.
    Null,
    Bool(bool),
    /// Text printed verbatim in type position.
    Raw(String),
    /// A string literal type, escaped the way the TS printer escapes `createStringLiteral`.
    Str(String),
    /// A numeric literal type; a leading `-` stands for a prefix-minus expression.
    Num(String),
    Ref(String, Vec<Self>),
    Indexed(Box<Self>, Box<Self>),
    Array(Box<Self>),
    Tuple(Vec<Self>),
    Rest(Box<Self>),
    Union(Vec<Self>),
    Intersection(Vec<Self>),
    Object(Vec<Member>),
    Readonly(Box<Self>),
    /// TypeScript source from a [`Host`](crate::Host) hook, printed by itself at indent 0. It is
    /// spliced in at the current indentation, the way the TS printer prints the node in place.
    /// Build it with [`Ty::source`].
    Source {
        text: String,
        kind: TypeKind,
    },
}

/// The syntax kind of a [`Ty::Source`] type, as far as parenthesization, deduplication and
/// array wrapping care. Mirrors the TypeScript node kind the source was printed from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeKind {
    /// A keyword (`string`, `any`, …) or the `null` literal.
    Keyword,
    Union,
    Intersection,
    /// A function, constructor or conditional type.
    Function,
    /// A `keyof`, `unique` or `readonly` type operator.
    TypeOperator,
    /// A `typeof` query or an `infer` type.
    Query,
    Array,
    Tuple,
    /// Anything else: references, literals, type literals, indexed access, …
    Other,
}

impl Ty {
    /// A type given as TypeScript source printed at indent 0. The keywords openapi-typescript
    /// treats as primitives (`string`, `number`, `boolean`, `never`, `undefined`, `null`) become
    /// the same types the core builds, so they dedupe the same way.
    #[must_use]
    pub fn source(text: impl Into<String>, kind: TypeKind) -> Self {
        let text = text.into();
        if kind == TypeKind::Keyword {
            match text.as_str() {
                "string" => return STRING,
                "number" => return NUMBER,
                "boolean" => return BOOLEAN,
                "never" => return NEVER,
                "undefined" => return UNDEFINED,
                "null" => return Self::Null,
                _ => {}
            }
        }
        Self::Source { text, kind }
    }
}

pub const STRING: Ty = Ty::Keyword("string");
pub const NUMBER: Ty = Ty::Keyword("number");
pub const BOOLEAN: Ty = Ty::Keyword("boolean");
pub const UNKNOWN: Ty = Ty::Keyword("unknown");
pub const NEVER: Ty = Ty::Keyword("never");
pub const UNDEFINED: Ty = Ty::Keyword("undefined");

/// A property name as `tsPropertyIndex` builds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropName {
    /// `createIdentifier(text)`.
    Ident(String),
    /// `createStringLiteral(text)`: the unescaped value, printed with [`ts_string_literal`].
    Str(String),
    /// `createNumericLiteral(text)`.
    Num(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum MemberKind {
    Property {
        readonly: bool,
        name: PropName,
        optional: bool,
        ty: Ty,
    },
    Index {
        readonly: bool,
        param: String,
        param_ty: Ty,
        ty: Ty,
    },
    /// A whole member printed by a [`Host`](crate::Host) at indent 0, leading comments and
    /// trailing `;` included.
    Source(String),
}

/// A type literal member with an optional leading `/* */` comment body.
#[derive(Debug, Clone, PartialEq)]
pub struct Member {
    pub comment: Option<String>,
    pub kind: MemberKind,
}

impl Member {
    #[must_use]
    pub fn prop(name: PropName, optional: bool, readonly: bool, ty: Ty) -> Self {
        Self { comment: None, kind: MemberKind::Property { readonly, name, optional, ty } }
    }

    #[must_use]
    pub fn index(param: &str, readonly: bool, param_ty: Ty, ty: Ty) -> Self {
        Self { comment: None, kind: MemberKind::Index { readonly, param: param.into(), param_ty, ty } }
    }

    #[must_use]
    pub fn optional(&self) -> bool {
        matches!(self.kind, MemberKind::Property { optional: true, .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnumValue {
    Str(String),
    Num(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumMember {
    pub name: String,
    pub value: EnumValue,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Interface {
        name: String,
        members: Vec<Member>,
    },
    TypeAlias {
        name: String,
        ty: Ty,
    },
    Enum {
        name: String,
        members: Vec<EnumMember>,
    },
    ConstArray {
        name: String,
        ty: Ty,
        values: Vec<EnumValue>,
    },
    /// A declaration openapi-typescript parses from a string; stored as the text TS prints.
    Raw {
        name: String,
        text: String,
    },
}

impl Stmt {
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Interface { name, .. }
            | Self::TypeAlias { name, .. }
            | Self::Enum { name, .. }
            | Self::ConstArray { name, .. }
            | Self::Raw { name, .. } => name,
        }
    }
}

#[derive(Clone, Copy)]
enum Paren {
    None,
    UnionMember,
    IntersectionMember,
    PostfixOperand,
    ReadonlyOperand,
}

fn needs_parens(ty: &Ty, rule: Paren) -> bool {
    if let Ty::Source { kind, .. } = ty {
        use TypeKind::{Function, Intersection, Query, TypeOperator, Union};
        return match rule {
            Paren::None => false,
            Paren::UnionMember | Paren::IntersectionMember => matches!(kind, Union | Intersection | Function),
            Paren::ReadonlyOperand => matches!(kind, Union | Intersection | Function | TypeOperator),
            Paren::PostfixOperand => matches!(kind, Union | Intersection | Function | TypeOperator | Query),
        };
    }
    let composite = matches!(ty, Ty::Union(_) | Ty::Intersection(_));
    match rule {
        Paren::None => false,
        Paren::UnionMember | Paren::IntersectionMember => composite,
        Paren::PostfixOperand | Paren::ReadonlyOperand => composite || matches!(ty, Ty::Readonly(_)),
    }
}

/// Escapes a string the way the TS printer prints a synthesized string literal (double quotes,
/// non-ASCII as `\uXXXX`).
#[must_use]
pub fn ts_string_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    push_ts_string_literal(&mut out, s);
    out
}

#[expect(clippy::needless_bitwise_bool, reason = "branchless fold over the bytes")]
fn push_ts_string_literal(out: &mut String, s: &str) {
    out.push('"');
    let plain = s.bytes().fold(true, |acc, b| acc & (0x20..=0x7f).contains(&b) & (b != b'"') & (b != b'\\'));
    if plain {
        out.push_str(s);
        out.push('"');
        return;
    }
    let mut units: Vec<u16> = Vec::with_capacity(s.len());
    for c in s.chars() {
        match spectype_document::value::lone_surrogate(c) {
            Some(u) => units.push(u),
            None => units.extend(c.encode_utf16(&mut [0; 2]).iter()),
        }
    }
    for (i, &u) in units.iter().enumerate() {
        match u {
            0x5c => out.push_str("\\\\"),
            0x22 => out.push_str("\\\""),
            0x09 => out.push_str("\\t"),
            0x0b => out.push_str("\\v"),
            0x0c => out.push_str("\\f"),
            0x08 => out.push_str("\\b"),
            0x0d => out.push_str("\\r"),
            0x0a => out.push_str("\\n"),
            0x00 => {
                let next_digit = units.get(i + 1).is_some_and(|&n| (0x30..=0x39).contains(&n));
                out.push_str(if next_digit { "\\x00" } else { "\\0" });
            }
            u if !(0x20..=0x7f).contains(&u) => {
                let _ = write!(out, "\\u{u:04X}");
            }
            u => out.push(char::from(u8::try_from(u).unwrap_or(b'?'))),
        }
    }
    out.push('"');
}

/// Mirrors the TypeScript emitter's text writer, including its "at start of line" tracking.
struct Writer {
    out: String,
    scratch: String,
    indent: usize,
    line_start: bool,
}

fn ends_with_line_break(s: &str) -> bool {
    s.ends_with(['\n', '\r', '\u{2028}', '\u{2029}'])
}

fn has_line_break(s: &str) -> bool {
    s.bytes().any(|b| matches!(b, b'\n' | b'\r' | 0xe2)) && s.contains(['\n', '\r', '\u{2028}', '\u{2029}'])
}

impl Writer {
    fn write(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        self.indent_if_line_start();
        self.out.push_str(s);
        if has_line_break(s) {
            self.line_start = ends_with_line_break(s);
        }
    }

    /// [`Writer::write`] for fixed text without line breaks.
    fn token(&mut self, s: &'static str) {
        self.indent_if_line_start();
        self.out.push_str(s);
    }

    fn write_string_literal(&mut self, s: &str) {
        self.indent_if_line_start();
        push_ts_string_literal(&mut self.out, s);
    }

    fn indent_if_line_start(&mut self) {
        if self.line_start {
            for _ in 0..self.indent {
                self.out.push_str("    ");
            }
            self.line_start = false;
        }
    }

    fn raw_write(&mut self, s: &str) {
        self.out.push_str(s);
        self.line_start = has_line_break(s) && ends_with_line_break(s);
    }

    /// Prints host-printed source in place: continuation lines move by the current indent.
    fn write_source(&mut self, text: &str) {
        let mut lines = text.split('\n');
        self.write(lines.next().unwrap_or_default());
        for line in lines {
            self.out.push('\n');
            for _ in 0..self.indent {
                self.out.push_str("    ");
            }
            self.out.push_str(line);
            self.line_start = false;
        }
    }

    fn write_line(&mut self) {
        if !self.line_start {
            self.out.push('\n');
            self.line_start = true;
        }
    }

    /// `writeCommentRange` for a synthesized `/* */` comment: continuation lines are re-indented
    /// relative to the writer's indent and trimmed.
    fn write_block_comment(&mut self, text: &str) {
        let mut lines = split_ts_lines(text).enumerate().peekable();
        while let Some((i, line)) = lines.next() {
            if i > 0 {
                let spaces = self.indent * 4 + calculate_indent(line);
                for _ in 0..spaces / 4 {
                    self.out.push_str("    ");
                }
                for _ in 0..spaces % 4 {
                    self.out.push(' ');
                }
                self.line_start = false;
            }
            let trimmed = js_trim(line);
            if trimmed.is_empty() {
                self.raw_write("\n");
            } else {
                self.indent_if_line_start();
                self.out.push_str(trimmed);
                if lines.peek().is_some() {
                    self.write_line();
                }
            }
        }
    }
}

/// Splits on the line terminators `computeLineStarts` recognises, keeping terminators off.
fn split_ts_lines(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = Some(text);
    std::iter::from_fn(move || {
        let current = rest?;
        let b = current.as_bytes();
        let found = (0..b.len()).find(|&i| match b[i] {
            b'\n' | b'\r' => true,
            0xe2 => b.get(i + 1) == Some(&0x80) && matches!(b.get(i + 2), Some(0xa8 | 0xa9)),
            _ => false,
        });
        let Some(i) = found else {
            rest = None;
            return Some(current);
        };
        let skip = match b[i] {
            b'\r' if b.get(i + 1) == Some(&b'\n') => 2,
            b'\r' | b'\n' => 1,
            _ => 3,
        };
        rest = Some(&current[i + skip..]);
        Some(&current[..i])
    })
}

fn calculate_indent(line: &str) -> usize {
    let mut indent = 0;
    for c in line.chars() {
        match c {
            '\t' => indent += 4 - indent % 4,
            ' '
            | '\u{b}'
            | '\u{c}'
            | '\u{a0}'
            | '\u{85}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200b}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
            | '\u{feff}' => {
                indent += c.len_utf16();
            }
            _ => break,
        }
    }
    indent
}

impl Writer {
    fn ty(&mut self, ty: &Ty, rule: Paren) {
        if needs_parens(ty, rule) {
            self.token("(");
            self.ty(ty, Paren::None);
            self.token(")");
            return;
        }
        match ty {
            Ty::Keyword(k) => self.token(k),
            Ty::Null => self.token("null"),
            Ty::Bool(b) => self.token(if *b { "true" } else { "false" }),
            Ty::Raw(s) | Ty::Num(s) => self.write(s),
            Ty::Str(s) => self.write_string_literal(s),
            Ty::Source { text, .. } => self.write_source(text),
            Ty::Ref(name, args) => {
                self.write(name);
                if !args.is_empty() {
                    self.token("<");
                    for (i, a) in args.iter().enumerate() {
                        if i > 0 {
                            self.token(", ");
                        }
                        self.ty(a, Paren::None);
                    }
                    self.token(">");
                }
            }
            Ty::Indexed(obj, idx) => {
                self.ty(obj, Paren::PostfixOperand);
                self.token("[");
                self.ty(idx, Paren::None);
                self.token("]");
            }
            Ty::Array(el) => {
                self.ty(el, Paren::PostfixOperand);
                self.token("[]");
            }
            Ty::Rest(el) => {
                self.token("...");
                self.ty(el, Paren::None);
            }
            Ty::Readonly(el) => {
                self.token("readonly ");
                self.ty(el, Paren::ReadonlyOperand);
            }
            Ty::Union(types) | Ty::Intersection(types) => {
                let (sep, rule) = if matches!(ty, Ty::Union(_)) {
                    (" | ", Paren::UnionMember)
                } else {
                    (" & ", Paren::IntersectionMember)
                };
                for (i, t) in types.iter().enumerate() {
                    if i > 0 {
                        self.token(sep);
                    }
                    self.ty(t, rule);
                }
            }
            Ty::Tuple(elements) => {
                self.token("[");
                self.write_line();
                self.indent += 1;
                for (i, el) in elements.iter().enumerate() {
                    if i > 0 {
                        self.token(",");
                        self.write_line();
                    }
                    self.ty(el, Paren::None);
                }
                self.indent -= 1;
                self.write_line();
                self.token("]");
            }
            Ty::Object(members) => {
                self.token("{");
                if !members.is_empty() {
                    self.members(members);
                }
                self.token("}");
            }
        }
    }

    fn members(&mut self, members: &[Member]) {
        self.write_line();
        self.indent += 1;
        for (i, m) in members.iter().enumerate() {
            if i > 0 {
                self.write_line();
            }
            self.member(m);
        }
        self.indent -= 1;
        self.write_line();
    }

    fn member(&mut self, m: &Member) {
        if let Some(c) = &m.comment {
            let mut text = std::mem::take(&mut self.scratch);
            text.clear();
            text.push_str("/*");
            text.push_str(c);
            text.push_str("*/");
            self.write_block_comment(&text);
            self.scratch = text;
            self.write_line();
        }
        match &m.kind {
            MemberKind::Property { readonly, name, optional, ty } => {
                if *readonly {
                    self.token("readonly ");
                }
                match name {
                    PropName::Ident(s) | PropName::Num(s) => self.write(s),
                    PropName::Str(s) => self.write_string_literal(s),
                }
                if *optional {
                    self.token("?");
                }
                self.token(": ");
                self.ty(ty, Paren::None);
            }
            MemberKind::Index { readonly, param, param_ty, ty } => {
                if *readonly {
                    self.token("readonly ");
                }
                self.token("[");
                self.write(param);
                self.token(": ");
                self.ty(param_ty, Paren::None);
                self.token("]: ");
                self.ty(ty, Paren::None);
            }
            MemberKind::Source(text) => return self.write_source(text),
        }
        self.token(";");
    }

    fn enum_value(&mut self, v: &EnumValue) {
        match v {
            EnumValue::Str(s) => self.write_string_literal(s),
            EnumValue::Num(n) => self.write(n),
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Interface { name, members } => {
                self.token("export interface ");
                self.write(name);
                self.token(" {");
                if members.is_empty() {
                    self.write_line();
                } else {
                    self.members(members);
                }
                self.token("}");
            }
            Stmt::TypeAlias { name, ty } => {
                self.token("export type ");
                self.write(name);
                self.token(" = ");
                self.ty(ty, Paren::None);
                self.token(";");
            }
            Stmt::Enum { name, members } => {
                self.token("export enum ");
                self.write(name);
                self.token(" {");
                self.write_line();
                self.indent += 1;
                for (i, m) in members.iter().enumerate() {
                    if i > 0 {
                        self.token(",");
                        self.write_line();
                    }
                    if let Some(c) = &m.comment {
                        self.write_line();
                        self.write(&format!("//{c}"));
                        self.write_line();
                    }
                    self.write(&m.name);
                    self.token(" = ");
                    self.enum_value(&m.value);
                }
                self.indent -= 1;
                self.write_line();
                self.token("}");
            }
            Stmt::ConstArray { name, ty, values } => {
                self.token("export const ");
                self.write(name);
                self.token(": ");
                self.ty(ty, Paren::None);
                self.token(" = [");
                for (i, v) in values.iter().enumerate() {
                    if i > 0 {
                        self.token(", ");
                    }
                    self.enum_value(v);
                }
                self.token("];");
            }
            Stmt::Raw { text, .. } => self.write(text),
        }
        self.write_line();
    }
}

/// Prints a type by itself at indent 0, as `printer.printNode` would.
#[must_use]
pub fn print_type(ty: &Ty) -> String {
    let mut w = Writer { out: String::new(), scratch: String::new(), indent: 0, line_start: true };
    w.ty(ty, Paren::None);
    w.out
}

/// Prints a type literal member by itself at indent 0, leading comment and `;` included.
#[must_use]
pub fn print_member(member: &Member) -> String {
    let mut w = Writer { out: String::new(), scratch: String::new(), indent: 0, line_start: true };
    w.member(member);
    w.out
}

/// Prints statements the way `ts.createPrinter().printFile` does for synthesized nodes.
#[must_use]
pub fn print(stmts: &[Stmt]) -> String {
    print_into(String::new(), stmts)
}

#[must_use]
pub fn print_into(out: String, stmts: &[Stmt]) -> String {
    let mut w = Writer { out, scratch: String::new(), indent: 0, line_start: true };
    for s in stmts {
        w.stmt(s);
    }
    w.out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_like_ts() {
        assert_eq!(ts_string_literal("a\"b\\c\u{0}1é😀\u{2028}"), "\"a\\\"b\\\\c\\x001\\u00E9\\uD83D\\uDE00\\u2028\"");
        assert_eq!(ts_string_literal("\u{7f}\u{1f}"), "\"\u{7f}\\u001F\"");
    }

    #[test]
    fn reindents_comments() {
        let ty = Ty::Object(vec![Member {
            comment: Some("*\n * @description a\n *     b  \n ".into()),
            kind: MemberKind::Property {
                readonly: false,
                name: PropName::Ident("x".into()),
                optional: true,
                ty: Ty::Union(vec![Ty::Intersection(vec![STRING, NUMBER]), Ty::Null]),
            },
        }]);
        let out = print(&[Stmt::TypeAlias { name: "T".into(), ty }]);
        assert_eq!(
            out,
            "export type T = {\n    /**\n     * @description a\n     *     b\n     */\n    x?: (string & number) | null;\n};\n"
        );
    }
}

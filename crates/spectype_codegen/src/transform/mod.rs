//! A port of openapi-typescript's `src/transform/*`.

mod schema;

use rustc_hash::{FxHashMap, FxHashSet};
use std::borrow::Cow;
use std::cmp::Ordering;
use std::sync::LazyLock;

use icu_collator::options::CollatorOptions;
use icu_collator::preferences::CollationNumericOrdering;
use icu_collator::{Collator, CollatorBorrowed, CollatorPreferences};

use crate::discriminators::Discriminators;
use crate::host::{HookContext, Host, PropertyHookInput, TransformResult};
use crate::names::{InvalidCharsRegexState, pascal_case, property_index};
use crate::ts::{EnumMember, EnumValue, Member, MemberKind, NEVER, PropName, STRING, Stmt, Ty, UNKNOWN};
use crate::{Fix, Options};
use spectype_document::oas::{ComponentKind, HttpMethod, ParamLocation};
use spectype_document::pointer::{JsonPointer, Resolved, create_ref, escape_pointer, parse_ref, resolve_ref};
use spectype_document::value::{JsMap, Value, js_trim};
use spectype_document::{Error, Result};

use schema::record;
pub use schema::transform_schema;

static COLLATOR: LazyLock<CollatorBorrowed<'static>> = LazyLock::new(|| {
    Collator::try_new(CollatorPreferences::default(), CollatorOptions::default()).expect("root collation data")
});
static NUMERIC_COLLATOR: LazyLock<CollatorBorrowed<'static>> = LazyLock::new(|| {
    let mut prefs = CollatorPreferences::default();
    prefs.numeric_ordering = Some(CollationNumericOrdering::True);
    Collator::try_new(prefs, CollatorOptions::default()).expect("root collation data")
});

/// `a.localeCompare(b, "en-us", { numeric: true })`.
fn locale_compare_numeric(a: &str, b: &str) -> Ordering {
    NUMERIC_COLLATOR.compare(a, b)
}

/// `a.localeCompare(b)` in Node's default locale.
fn locale_compare(a: &str, b: &str) -> Ordering {
    COLLATOR.compare(a, b)
}

pub struct Ctx<'a, 'h> {
    opts: &'a Options,
    root: &'a Value,
    host: &'h mut dyn Host,
    /// Upstream's `options.schema`: the component (or root `$defs`) being transformed.
    hook_schema: Option<&'a Value>,
    discriminators: &'a Discriminators,
    footer: Vec<Stmt>,
    footer_names: FxHashSet<String>,
    regex_state: InvalidCharsRegexState,
    enum_cache: FxHashMap<String, String>,
    warnings: Vec<String>,
    /// The `$ref`s reachable from parameters and request bodies, for [`Fix::RequestDefaults`].
    request_refs: FxHashSet<String>,
}

impl<'a> Ctx<'a, '_> {
    fn hook_ctx<'p>(&self, path: &'p JsonPointer) -> HookContext<'p>
    where
        'a: 'p,
    {
        HookContext { path, schema: self.hook_schema, document: self.root, discriminators: self.discriminators }
    }

    /// `ctx.transform(schema, options)`.
    fn transform_hook(&mut self, schema: &Value, path: &JsonPointer) -> Result<Option<TransformResult>> {
        let hc = self.hook_ctx(path);
        self.host.transform(schema, &hc).map_err(Error::Host)
    }

    /// `ctx.postTransform(type, options)`, keeping `ty` when the hook returns nothing.
    fn post_transform_hook(&mut self, ty: Ty, path: &JsonPointer) -> Result<Ty> {
        let hc = self.hook_ctx(path);
        Ok(self.host.post_transform(&ty, &hc).map_err(Error::Host)?.unwrap_or(ty))
    }

    /// `ctx.transformProperty(property, schema, options)` then `addJSDocComment(schema, property)`.
    fn property_hook(&mut self, member: Member, schema: &Value, path: &JsonPointer) -> Result<Member> {
        let comment = jsdoc(schema);
        let hc = self.hook_ctx(path);
        let replaced = match &member.kind {
            MemberKind::Property { readonly, name, optional, ty } => {
                let input =
                    PropertyHookInput { name, readonly: *readonly, optional: *optional, ty, jsdoc: comment.as_deref() };
                self.host.transform_property(&input, schema, &hc).map_err(Error::Host)?
            }
            _ => None,
        };
        Ok(match replaced {
            Some(text) => Member { comment: None, kind: MemberKind::Source(text) },
            None => Member { comment, ..member },
        })
    }

    /// `ctx.resolve($ref)`, warning like openapi-typescript when the target is missing.
    fn resolve(&mut self, r: &str) -> Option<&'a Value> {
        match resolve_ref(self.root, r) {
            Resolved::Found(v) => Some(v),
            Resolved::Missing => {
                self.warnings.push(format!("Could not resolve $ref \"{r}\""));
                None
            }
            Resolved::Circular => {
                self.warnings.push(format!("Could not resolve circular $ref \"{r}\""));
                None
            }
        }
    }

    fn push_footer(&mut self, stmt: Stmt) {
        self.footer_names.insert(stmt.name().to_owned());
        self.footer.push(stmt);
    }

    fn inject_helper(&mut self, name: &str, text: &str) {
        let present = self.footer_names.contains(name)
            && self.footer.iter().any(|s| matches!(s, Stmt::Raw { name: n, .. } if n == name));
        if !present {
            self.push_footer(Stmt::Raw { name: name.into(), text: text.into() });
        }
    }
}

fn oapi_ref(path: &str, at: &str) -> Result<Ty> {
    oapi_ref_with(path, None, false, &[], at)
}

/// `oapiRef`: turns a JSON Pointer into an indexed access type into the generated types.
fn oapi_ref_with(path: &str, parameter: Option<(&str, &str)>, deep: bool, extract: &[&str], at: &str) -> Result<Ty> {
    let pointer = parse_ref(path);
    if pointer.is_empty() {
        return Err(Error::at(at, format!("Error parsing $ref: {path}. Is this a valid $ref?")));
    }
    let head = if deep { format!("FlattenedDeepRequired<{}>", pointer[0]) } else { pointer[0].to_string() };
    let mut acc = Ty::Ref(head, Vec::new());
    for seg in pointer.iter().skip(1).take(2) {
        acc = index(acc, seg);
    }
    let rest = pointer.get(3..).unwrap_or_default();
    for (i, seg) in rest.iter().enumerate() {
        if seg == "properties" {
            continue;
        }
        if let Some((loc, name)) = parameter.filter(|_| i == rest.len() - 1) {
            acc = index(index(acc, loc), name);
            continue;
        }
        if extract.contains(&seg.as_ref()) {
            let narrowed = Ty::Ref(
                "Extract".into(),
                vec![acc, Ty::Object(vec![Member::prop(PropName::Ident(seg.to_string()), false, false, UNKNOWN)])],
            );
            acc = index(narrowed, seg);
            continue;
        }
        acc = index(acc, seg);
    }
    Ok(acc)
}

fn index(ty: Ty, seg: &str) -> Ty {
    Ty::Indexed(Box::new(ty), Box::new(Ty::Str(seg.to_owned())))
}

/// `Object.entries(obj)` without sorting or filtering.
fn raw_entries(v: &Value) -> Vec<(Cow<'_, str>, &Value)> {
    match v {
        Value::Object(m) => m.iter().map(|(k, v)| (Cow::Borrowed(k.as_str()), v)).collect(),
        Value::Array(a) => a.iter().enumerate().map(|(i, v)| (Cow::Owned(i.to_string()), v)).collect(),
        _ => Vec::new(),
    }
}

/// `getEntries(obj, ctx)`: optionally alphabetized and with deprecated entries dropped.
fn entries<'v>(ctx: &Ctx, v: &'v Value) -> Vec<(Cow<'v, str>, &'v Value)> {
    entries_with(v, ctx.opts.alphabetize, ctx.opts.exclude_deprecated)
}

fn entries_with(v: &Value, alphabetize: bool, exclude_deprecated: bool) -> Vec<(Cow<'_, str>, &Value)> {
    let mut out = raw_entries(v);
    if alphabetize {
        out.sort_by(|(a, _), (b, _)| locale_compare_numeric(a, b));
    }
    if exclude_deprecated {
        out.retain(|(_, v)| !(v.is_object() && v.get("deprecated").is_some_and(Value::truthy)));
    }
    out
}

const LB: &str = "\n *     ";

/// `.trim().replace(/\r?\n/g, "\n *     ")`.
fn push_comment_text(out: &mut String, s: &str) {
    let mut lines = js_trim(s).split('\n').peekable();
    while let Some(line) = lines.next() {
        if lines.peek().is_some() {
            out.push_str(line.strip_suffix('\r').unwrap_or(line));
            out.push_str(LB);
        } else {
            out.push_str(line);
        }
    }
}

fn js_text(v: &Value) -> Cow<'_, str> {
    match v {
        Value::String(s) => Cow::Borrowed(s),
        v => Cow::Owned(v.js_string()),
    }
}

fn serialized(v: &Value) -> Cow<'_, str> {
    if v.is_js_object() { Cow::Owned(v.json_stringify(2)) } else { js_text(v) }
}

const MULTI_LINE_START: &str = "*\n * ";

struct DocLines {
    body: String,
    count: usize,
    capacity: usize,
}

impl DocLines {
    fn line(&mut self) -> &mut String {
        if self.count == 0 {
            self.body.reserve(self.capacity);
            self.body.push_str(MULTI_LINE_START);
        } else {
            self.body.push_str("\n * ");
        }
        self.count += 1;
        &mut self.body
    }
}

#[derive(Default)]
struct DocFields<'v> {
    title: Option<&'v Value>,
    summary: Option<&'v Value>,
    format: Option<&'v Value>,
    deprecated: Option<&'v Value>,
    description: Option<&'v Value>,
    default: Option<&'v Value>,
    example: Option<&'v Value>,
    examples: Option<&'v Value>,
    constant: bool,
    enum_: Option<&'v Value>,
    ty: Option<&'v Value>,
    nullable: Option<&'v Value>,
}

/// `addJSDocComment`: the text between `/*` and `*/`, or `None` when there is nothing to say.
fn jsdoc(v: &Value) -> Option<String> {
    let Value::Object(o) = v else { return None };
    let mut f = DocFields::default();
    for (k, v) in o.iter() {
        match k.as_str() {
            "title" => f.title = Some(v),
            "summary" => f.summary = Some(v),
            "format" => f.format = Some(v),
            "deprecated" => f.deprecated = Some(v),
            "description" => f.description = Some(v),
            "default" => f.default = Some(v),
            "example" => f.example = Some(v),
            "examples" => f.examples = Some(v),
            "const" => f.constant = true,
            "enum" => f.enum_ = Some(v),
            "type" => f.ty = Some(v),
            "nullable" => f.nullable = Some(v),
            _ => {}
        }
    }
    let truthy = |v: Option<&Value>| v.is_some_and(Value::truthy);
    let text_len = [f.title, f.summary, f.description].into_iter().flatten().filter_map(Value::as_str).map(str::len);
    let mut doc = DocLines { body: String::new(), count: 0, capacity: 64 + text_len.sum::<usize>() };
    for s in [f.title, f.summary].into_iter().flatten().filter(|s| s.truthy()) {
        push_comment_text(doc.line(), &js_text(s));
    }
    if let Some(format) = f.format.filter(|s| s.truthy()) {
        let line = doc.line();
        line.push_str("Format: ");
        line.push_str(&js_text(format));
    }
    if truthy(f.deprecated) {
        doc.line().push_str("@deprecated");
    }
    for (field, value) in [("description", f.description), ("default", f.default), ("example", f.example)] {
        let Some(value) = value else { continue };
        if field == "description" && value.as_str() == Some("") {
            continue;
        }
        let line = doc.line();
        line.push('@');
        line.push_str(field);
        line.push(' ');
        push_comment_text(line, &serialized(value));
    }
    if let Some(Value::Array(examples)) = f.examples {
        for example in examples {
            let line = doc.line();
            line.push_str("@example ");
            push_comment_text(line, &serialized(example));
        }
    }
    if f.constant {
        doc.line().push_str("@constant");
    }
    if truthy(f.enum_) {
        let ty = match f.ty {
            Some(Value::Array(a)) => a
                .iter()
                .map(|v| if matches!(v, Value::Null) { String::new() } else { v.js_string() })
                .collect::<Vec<_>>()
                .join("|"),
            Some(Value::String(s)) => s.clone(),
            _ => "unknown".into(),
        };
        let nullable = if truthy(f.nullable) { "|null" } else { "" };
        let line = doc.line();
        line.push_str("@enum {");
        line.push_str(&ty);
        line.push_str(nullable);
        line.push('}');
    }
    let mut text = doc.body;
    match doc.count {
        0 => return None,
        1 if !text[MULTI_LINE_START.len()..].contains('\n') => {
            text.replace_range(..MULTI_LINE_START.len(), "* ");
            text.push(' ');
        }
        _ => text.push_str("\n "),
    }
    Some(if text.contains("*/") { text.replace("*/", "*\\/") } else { text })
}

fn prop(name: &str, optional: bool, readonly: bool, ty: Ty, doc: Option<&Value>) -> Member {
    let mut m = Member::prop(property_index(name), optional, readonly, ty);
    m.comment = doc.and_then(jsdoc);
    m
}

fn str_of(v: Option<&Value>) -> String {
    v.map_or_else(|| "undefined".into(), Value::js_string)
}

/// `transformMediaTypeObject`.
fn media_type(ctx: &mut Ctx, media: &Value, path: &JsonPointer) -> Result<Ty> {
    match media.get("schema").filter(|s| s.truthy()) {
        Some(s) => transform_schema(ctx, s, path, false),
        None => Ok(UNKNOWN),
    }
}

/// `transformHeaderObject`.
fn header(ctx: &mut Ctx, h: &Value, path: &JsonPointer) -> Result<Ty> {
    if let Some(s) = h.get("schema").filter(|s| s.truthy()) {
        return transform_schema(ctx, s, path, false);
    }
    if let Some(content) = h.get("content").filter(|c| c.truthy()) {
        let mut members = Vec::new();
        for (ct, media) in entries(ctx, content) {
            let next = JsonPointer::from(format!(
                "{}/{}",
                if path.is_empty() { "#" } else { path.as_str() },
                escape_pointer(&ct)
            ));
            let ty = if media.has("$ref") {
                transform_schema(ctx, media, &next, false)?
            } else {
                media_type(ctx, media, &next)?
            };
            members.push(prop(&ct, false, ctx.opts.immutable, ty, Some(media)));
        }
        return Ok(Ty::Object(members));
    }
    Ok(UNKNOWN)
}

/// `transformRequestBodyObject`.
fn request_body(ctx: &mut Ctx, rb: &Value, path: &JsonPointer) -> Result<Ty> {
    let mut members = Vec::new();
    if let Some(content) = rb.get("content").filter(|c| !matches!(c, Value::Null)) {
        for (ct, media) in entries(ctx, content) {
            let next = create_ref(&[path.as_str(), "content", &ct]);
            let ty = if media.has("$ref") {
                transform_schema(ctx, media, &next, false)?
            } else {
                media_type(ctx, media, &next)?
            };
            members.push(prop(&ct, false, ctx.opts.immutable, ty, Some(media)));
        }
    }
    if members.is_empty() {
        members.push(Member::prop(property_index("*/*"), true, false, NEVER));
    }
    Ok(Ty::Object(vec![Member::prop(
        PropName::Ident("content".into()),
        false,
        ctx.opts.immutable,
        Ty::Object(members),
    )]))
}

/// `transformResponseObject`.
fn response(ctx: &mut Ctx, r: &Value, path: &JsonPointer) -> Result<Ty> {
    let immutable = ctx.opts.immutable;
    let mut headers = Vec::new();
    if let Some(hs) = r.get("headers").filter(|h| h.truthy()) {
        for (name, h) in entries(ctx, hs) {
            let is_ref = h.has("$ref");
            let optional = !(is_ref || h.get("required").is_some_and(Value::truthy));
            let ty = if is_ref {
                oapi_ref(&str_of(h.get("$ref")), path)?
            } else {
                header(ctx, h, &create_ref(&[path.as_str(), "headers", &name]))?
            };
            headers.push(prop(&name, optional, immutable, ty, Some(h)));
        }
    }
    headers.push(Member::index("name", immutable, STRING, UNKNOWN));
    let mut members = vec![Member::prop(PropName::Ident("headers".into()), false, false, Ty::Object(headers))];
    let mut content = Vec::new();
    if let Some(c) = r.get("content").filter(|c| c.truthy()) {
        for (ct, media) in entries(ctx, c) {
            let ty = media_type(ctx, media, &create_ref(&[path.as_str(), "content", &ct]))?;
            content.push(prop(&ct, false, immutable, ty, Some(media)));
        }
    }
    if content.is_empty() {
        members.push(Member::prop(PropName::Ident("content".into()), true, false, NEVER));
    } else {
        members.push(Member::prop(PropName::Ident("content".into()), false, false, Ty::Object(content)));
    }
    Ok(Ty::Object(members))
}

/// `transformResponsesObject`.
fn responses(ctx: &mut Ctx, rs: &Value, path: &JsonPointer) -> Result<Ty> {
    let mut members = Vec::new();
    for (code, r) in entries(ctx, rs) {
        let ty = if r.has("$ref") {
            oapi_ref(&str_of(r.get("$ref")), path)?
        } else {
            response(ctx, r, &create_ref(&[path.as_str(), "responses", &code]))?
        };
        members.push(prop(&code, false, ctx.opts.immutable, ty, Some(r)));
    }
    Ok(if members.is_empty() { NEVER } else { Ty::Object(members) })
}

/// `transformParameterObject`.
fn parameter(ctx: &mut Ctx, p: &Value, path: &JsonPointer) -> Result<Ty> {
    match p.get("schema").filter(|s| s.truthy()) {
        Some(s) => transform_schema(ctx, s, path, false),
        None => Ok(STRING),
    }
}

/// `transformParametersArray`.
fn parameters_array(ctx: &mut Ctx, params: &[&Value], path: &JsonPointer) -> Result<Vec<Member>> {
    let immutable = ctx.opts.immutable;
    let mut generated: Vec<Value> = Vec::new();
    if ctx.opts.generate_path_params && !path.is_empty() {
        for name in brace_params(path) {
            let exists = params.iter().copied().chain(&generated).any(|p| {
                let resolved = match p.get("$ref") {
                    Some(r) => resolve_ref_quiet(ctx.root, &js_text(r)),
                    None => Some(p),
                };
                resolved.is_some_and(|r| {
                    r.get("in").and_then(Value::as_str) == Some("path")
                        && r.get("name").and_then(Value::as_str) == Some(name.as_str())
                })
            });
            if !exists {
                let mut m = spectype_document::value::Map::new();
                m.insert("name".into(), Value::String(name));
                m.insert("in".into(), Value::String("path".into()));
                m.insert("required".into(), Value::Bool(true));
                let mut s = spectype_document::value::Map::new();
                s.insert("type".into(), Value::String("string".into()));
                m.insert("schema".into(), Value::Object(s));
                generated.push(Value::Object(m));
            }
        }
    }
    let working: Vec<&Value> = params.iter().copied().chain(&generated).collect();
    let found: Vec<Option<&Value>> = working
        .iter()
        .map(|&p| match p.get("$ref") {
            Some(r) => resolve_ref_quiet(ctx.root, &js_text(r)),
            None => Some(p),
        })
        .collect();

    let mut locations = Vec::new();
    for loc in ParamLocation::ALL {
        let mut resolved_params: Vec<(&Value, Option<&Value>)> = Vec::with_capacity(working.len());
        for (&original, &found) in working.iter().zip(&found) {
            let resolved = match (found, original.get("$ref")) {
                (None, Some(r)) => ctx.resolve(&js_text(r)),
                _ => found,
            };
            resolved_params.push((original, resolved));
        }
        if ctx.opts.alphabetize {
            resolved_params.sort_by(|(_, a), (_, b)| {
                let name = |v: Option<&Value>| v.and_then(|v| v.get("name")).map(Value::js_string).unwrap_or_default();
                locale_compare(&name(*a), &name(*b))
            });
        }
        if ctx.opts.exclude_deprecated {
            resolved_params.retain(|(_, r)| {
                !r.is_some_and(|r| {
                    r.get("deprecated").is_some_and(Value::truthy)
                        || r.get("schema").and_then(|s| s.get("deprecated")).is_some_and(Value::truthy)
                })
            });
        }
        let mut members = Vec::new();
        for (original, resolved) in resolved_params {
            let Some(resolved) = resolved else { continue };
            if resolved.get("in").and_then(Value::as_str) != Some(loc.as_str()) {
                continue;
            }
            let optional = loc != ParamLocation::Path && !resolved.get("required").is_some_and(Value::truthy);
            let name = str_of(resolved.get("name"));
            let ty = if let Some(r) = original.get("$ref") {
                let param = is_parameter(resolved).then_some((loc.as_str(), name.as_str()));
                oapi_ref_with(&js_text(r), param, false, &[], path)?
            } else {
                let segment = if resolved.has("name") { name.as_str() } else { "" };
                parameter(ctx, resolved, &create_ref(&[path.as_str(), "parameters", loc.as_str(), segment]))?
            };
            let mut member = prop(&name, optional, immutable, ty, Some(resolved));
            if !resolved.get("name").is_some_and(|n| matches!(n, Value::String(_) | Value::Number(_))) {
                // tsPropertyIndex only special-cases strings and numbers; anything else is stringified
                // into a quoted literal.
                if let MemberKind::Property { name: prop_name, .. } = &mut member.kind {
                    *prop_name = PropName::Str(name.clone());
                }
            }
            members.push(member);
        }
        let all_optional = members.iter().all(Member::optional);
        let ty = if members.is_empty() { NEVER } else { Ty::Object(members) };
        locations.push(Member::prop(PropName::Ident(loc.as_str().into()), all_optional, immutable, ty));
    }
    Ok(vec![Member::prop(PropName::Ident("parameters".into()), false, immutable, Ty::Object(locations))])
}

fn is_parameter(v: &Value) -> bool {
    !v.get("$ref").is_some_and(Value::truthy) && v.get("in").is_some_and(Value::truthy)
}

fn resolve_ref_quiet<'a>(root: &'a Value, r: &str) -> Option<&'a Value> {
    match resolve_ref(root, r) {
        Resolved::Found(v) => Some(v),
        _ => None,
    }
}

/// The names inside `{...}` in a string, in order.
fn brace_params(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        match after.find('}') {
            Some(0) => rest = &after[1..],
            Some(end) => {
                out.push(after[..end].to_owned());
                rest = &after[end + 1..];
            }
            None => break,
        }
    }
    out
}

/// `transformOperationObject`.
fn operation(ctx: &mut Ctx, op: &Value, params: &[&Value], path: &JsonPointer) -> Result<Vec<Member>> {
    let immutable = ctx.opts.immutable;
    let mut members = parameters_array(ctx, params, path)?;
    match op.get("requestBody").filter(|rb| rb.truthy()) {
        Some(rb) => {
            let (ty, required) = match rb.get("$ref") {
                Some(r) => {
                    let r = r.js_string();
                    let ty = oapi_ref(&r, path)?;
                    (ty, ctx.resolve(&r).and_then(|v| v.get("required")).is_some_and(Value::truthy))
                }
                None => (
                    request_body(ctx, rb, &create_ref(&[path.as_str(), "requestBody"]))?,
                    rb.get("required").is_some_and(Value::truthy),
                ),
            };
            members.push(prop("requestBody", !required, immutable, ty, Some(rb)));
        }
        None => members.push(Member::prop(PropName::Ident("requestBody".into()), true, immutable, NEVER)),
    }
    let empty = Value::Object(spectype_document::value::Map::new());
    let rs = op.get("responses").filter(|r| !matches!(r, Value::Null)).unwrap_or(&empty);
    let ty = responses(ctx, rs, path)?;
    members.push(Member::prop(PropName::Ident("responses".into()), false, immutable, ty));
    Ok(members)
}

fn params_of(v: Option<&Value>) -> &[Value] {
    v.and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

/// `transformPathItemObject`.
fn path_item(ctx: &mut Ctx, item: &Value, path: &JsonPointer) -> Result<Ty> {
    let immutable = ctx.opts.immutable;
    let item_params = params_of(item.get("parameters"));
    let item_param_refs: Vec<&Value> = item_params.iter().collect();
    let mut members = parameters_array(ctx, &item_param_refs, &create_ref(&[path.as_str(), "parameters"]))?;
    for method in HttpMethod::ALL {
        let op = item.get(method.as_str()).filter(|o| o.truthy());
        let deprecated = ctx.opts.exclude_deprecated
            && op.is_some_and(|o| {
                let resolved = match o.get("$ref") {
                    Some(r) => ctx.resolve(&js_text(r)),
                    None => Some(o),
                };
                resolved.and_then(|r| r.get("deprecated")).is_some_and(Value::truthy)
            });
        let Some(op) = op.filter(|_| !deprecated) else {
            members.push(Member::prop(PropName::Ident(method.as_str().into()), true, immutable, NEVER));
            continue;
        };

        let mut keyed: JsMap<&Value> = JsMap::new();
        if !op.has("$ref") {
            for p in item_params.iter().chain(params_of(op.get("parameters"))) {
                let key = match p.get("$ref") {
                    Some(r) => {
                        let r = js_text(r);
                        let resolved = ctx.resolve(&r);
                        let loc = str_of(resolved.and_then(|v| v.get("in")));
                        let resolved = if resolved.is_some() { resolved } else { ctx.resolve(&r) };
                        format!("{loc}-{}", str_of(resolved.and_then(|v| v.get("name"))))
                    }
                    None => format!("{}-{}", str_of(p.get("in")), str_of(p.get("name"))),
                };
                keyed.insert(key, p);
            }
        }
        let params: Vec<&Value> = keyed.into_values().collect();
        let op_path = create_ref(&[path.as_str(), method.as_str()]);

        let ty = if let Some(r) = op.get("$ref") {
            oapi_ref(&js_text(r), path)?
        } else if let Some(id) = op.get("operationId").filter(|id| id.truthy()) {
            let id = id.js_string().replace('#', "/");
            let ty = if ctx.opts.fix.contains(Fix::OperationIdRefs) {
                index(Ty::Ref("operations".into(), Vec::new()), &id)
            } else {
                oapi_ref(&create_ref(&["operations", &id]), path)?
            };
            inject_operation(ctx, &id, op, &params, &op_path)?;
            ty
        } else {
            Ty::Object(operation(ctx, op, &params, &op_path)?)
        };
        members.push(prop(method.as_str(), false, immutable, ty, Some(op)));
    }
    Ok(Ty::Object(members))
}

/// `injectOperationObject`.
fn inject_operation(ctx: &mut Ctx, id: &str, op: &Value, params: &[&Value], path: &JsonPointer) -> Result<()> {
    let idx = if let Some(i) =
        ctx.footer.iter().position(|s| matches!(s, Stmt::Interface { name, .. } if name == "operations"))
    {
        i
    } else {
        ctx.push_footer(Stmt::Interface { name: "operations".into(), members: Vec::new() });
        ctx.footer.len() - 1
    };
    let members = operation(ctx, op, params, path)?;
    let member = Member::prop(property_index(id), false, ctx.opts.immutable, Ty::Object(members));
    if let Some(Stmt::Interface { members, .. }) = ctx.footer.get_mut(idx) {
        members.push(member);
    }
    Ok(())
}

/// `transformPathsObject`.
fn paths(ctx: &mut Ctx, paths: &Value) -> Result<Ty> {
    let immutable = ctx.opts.immutable;
    let mut members = Vec::new();
    for (url, item) in entries(ctx, paths) {
        if !item.truthy() || !item.is_js_object() {
            continue;
        }
        if let Some(r) = item.get("$ref") {
            let ty = oapi_ref(&js_text(r), "#/paths")?;
            members.push(prop(&url, false, immutable, ty, Some(item)));
            continue;
        }
        let ty = path_item(ctx, item, &create_ref(&["paths", &url]))?;
        if ctx.opts.path_params_as_types && url.contains('{') {
            let params = extract_path_params(ctx, item);
            let matches = brace_matches(&url);
            if !matches.is_empty() {
                let mut raw = format!("`{url}`");
                for m in matches {
                    let name = &m[1..m.len() - 1];
                    let kind = params.get(name).cloned().flatten();
                    let replacement = match kind.as_deref() {
                        Some("number" | "integer") => "${number}",
                        Some("boolean") => "${boolean}",
                        _ => "${string}",
                    };
                    raw = raw.replacen(&m, replacement, 1);
                }
                members.push(Member::index("path", immutable, Ty::Raw(raw), ty));
                continue;
            }
        }
        members.push(Member::prop(property_index(&url), false, immutable, ty));
    }
    Ok(Ty::Object(members))
}

/// Matches of `/\{[^}]+\}/g`.
fn brace_matches(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        match after.find('}') {
            Some(0) => rest = after,
            Some(end) => {
                out.push(format!("{{{}}}", &after[..end]));
                rest = &after[end + 1..];
            }
            None => break,
        }
    }
    out
}

/// `extractPathParams`, reduced to each path parameter's `schema.type`.
fn extract_path_params(ctx: &mut Ctx, item: &Value) -> FxHashMap<String, Option<String>> {
    fn collect(ctx: &mut Ctx, list: &[Value], out: &mut FxHashMap<String, Option<String>>) {
        for p in list {
            let resolved = match p.get("$ref").filter(|r| r.truthy()) {
                Some(r) => ctx.resolve(&js_text(r)),
                None => Some(p),
            };
            if let Some(r) = resolved.filter(|r| r.get("in").and_then(Value::as_str) == Some("path")) {
                let kind = r.get("schema").and_then(|s| s.get("type")).and_then(Value::as_str).map(str::to_owned);
                out.insert(str_of(r.get("name")), kind);
            }
        }
    }
    let mut out = FxHashMap::default();
    collect(ctx, params_of(item.get("parameters")), &mut out);
    for method in HttpMethod::ALL {
        let Some(op) = item.get(method.as_str()) else { continue };
        let params = match op.get("$ref").filter(|r| r.truthy()) {
            Some(r) => ctx.resolve(&js_text(r)).map_or(&[][..], |o| params_of(o.get("parameters"))),
            None => params_of(op.get("parameters")),
        };
        collect(ctx, params, &mut out);
    }
    out
}

fn webhooks(ctx: &mut Ctx, hooks: &Value) -> Result<Ty> {
    let mut members = Vec::new();
    for (name, item) in entries(ctx, hooks) {
        let ty = path_item(ctx, item, &create_ref(&["webhooks", &name]))?;
        members.push(Member::prop(property_index(&name), false, ctx.opts.immutable, ty));
    }
    Ok(Ty::Object(members))
}

fn is_enum_schema(v: &Value) -> bool {
    matches!(v.get("enum"), Some(Value::Array(_)))
        && (!v.has("type") || v.get("type").and_then(Value::as_str) != Some("object"))
        && !v.has("properties")
        && !v.has("additionalProperties")
}

/// `transformComponentsObject`.
fn components<'a>(ctx: &mut Ctx<'a, '_>, components: &'a Value) -> Result<(Ty, Vec<Stmt>)> {
    let immutable = ctx.opts.immutable;
    let mut members = Vec::new();
    let mut aliases: JsMap<Stmt> = JsMap::new();
    let mut fixed_aliases = Vec::new();
    for kind in ComponentKind::TRANSFORMED {
        let key = kind.as_str();
        let is_schemas = kind == ComponentKind::Schemas;
        let singular = if kind == ComponentKind::RequestBodies { "requestBody" } else { &key[..key.len() - 1] };
        let component_key = pascal_case(singular);
        let mut items = Vec::new();
        if let Some(group) = components.get(key).filter(|g| g.truthy()) {
            for (name, item) in entries(ctx, group) {
                let path = create_ref(&["components", key, &name]);
                ctx.hook_schema = Some(item);
                let ty = match kind {
                    ComponentKind::Schemas => transform_schema(ctx, item, &path, false)?,
                    ComponentKind::Responses => response(ctx, item, &path)?,
                    ComponentKind::Parameters => parameter(ctx, item, &path)?,
                    ComponentKind::RequestBodies => request_body(ctx, item, &path)?,
                    ComponentKind::Headers => header(ctx, item, &path)?,
                    _ => path_item(ctx, item, &path)?,
                };
                let (ty, optional) = match ctx.transform_hook(item, &path)? {
                    Some(r) => (r.ty, r.question_token),
                    None => (ty, false),
                };
                ctx.hook_schema = None;
                items.push(prop(&name, optional, immutable, ty, Some(item)));

                if ctx.opts.root_types && !(ctx.opts.enum_ && is_schemas && is_enum_schema(item)) {
                    let component_name = if ctx.opts.root_types_keep_casing && is_schemas {
                        name.to_string()
                    } else {
                        pascal_case(&name)
                    };
                    if ctx.opts.fix.contains(Fix::RootTypesNames) {
                        let mut alias = format!("{component_key}{component_name}");
                        if ctx.opts.root_types_no_schema_prefix && is_schemas {
                            alias = alias.replacen(&component_key, "", 1);
                        }
                        let alias = schema::valid_identifier(&alias, &component_key);
                        let target = index(index(Ty::Ref("components".into(), Vec::new()), key), &name);
                        fixed_aliases.push(Stmt::TypeAlias { name: alias, ty: target });
                        continue;
                    }
                    let mut alias = format!("{component_key}{component_name}");
                    let mut counter = 1;
                    while aliases.get(&alias).is_some() {
                        counter += 1;
                        alias = format!("{component_key}{component_name}_{counter}");
                    }
                    let target = Ty::Ref(format!("components['{key}']['{name}']"), Vec::new());
                    if ctx.opts.root_types_no_schema_prefix && is_schemas {
                        alias = alias.replacen(&component_key, "", 1);
                    }
                    if alias != "__proto__" {
                        aliases.insert(alias.clone(), Stmt::TypeAlias { name: alias, ty: target });
                    }
                }
            }
        }
        let ty = if items.is_empty() { NEVER } else { Ty::Object(items) };
        members.push(Member::prop(PropName::Ident(key.into()), false, false, ty));
    }
    let mut out: Vec<Stmt> = aliases.into_values().collect();
    out.append(&mut fixed_aliases);
    Ok((Ty::Object(members), out))
}

/// Names a root type alias must not take: they are used by the generated file itself, are
/// TypeScript built-ins people reach for, or cannot name a type at all.
const RESERVED_ALIAS_NAMES: &[&str] = &[
    "paths",
    "webhooks",
    "components",
    "$defs",
    "operations",
    "Record",
    "Omit",
    "Extract",
    "Exclude",
    "Readonly",
    "ReadonlyArray",
    "WithRequired",
    "FlattenedDeepRequired",
    "NonNullable",
    "Readable",
    "Writable",
    "$Read",
    "$Write",
    "Array",
    "Partial",
    "Required",
    "Pick",
    "Date",
    "Promise",
    "Map",
    "Set",
    "Error",
    "Object",
    "String",
    "Number",
    "Boolean",
    "Function",
    "Symbol",
    "RegExp",
    "JSON",
    "any",
    "unknown",
    "never",
    "string",
    "number",
    "boolean",
    "object",
    "symbol",
    "bigint",
    "undefined",
    "null",
    "void",
    "this",
    "true",
    "false",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "finally",
    "for",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "new",
    "return",
    "super",
    "switch",
    "throw",
    "try",
    "typeof",
    "var",
    "while",
    "with",
    "implements",
    "interface",
    "let",
    "package",
    "private",
    "protected",
    "public",
    "static",
    "yield",
    "await",
];

/// Gives every root type alias a name no other declaration (or reserved name) uses, appending
/// `_2`, `_3`, … in document order.
fn rename_colliding_aliases(out: &mut [Stmt], aliases: &std::ops::Range<usize>) {
    let mut taken: FxHashSet<String> = RESERVED_ALIAS_NAMES.iter().map(|s| (*s).to_owned()).collect();
    for (i, s) in out.iter().enumerate() {
        if !aliases.contains(&i) {
            taken.insert(s.name().to_owned());
        }
    }
    for s in &mut out[aliases.clone()] {
        if let Stmt::TypeAlias { name, .. } = s {
            let base = name.clone();
            let mut n = 1;
            while taken.contains(name.as_str()) {
                n += 1;
                *name = format!("{base}_{n}");
            }
            taken.insert(name.clone());
        }
    }
}

/// Every `$ref` that parameters and request bodies reach, directly or through other `$ref`s.
fn request_refs(doc: &Value) -> FxHashSet<String> {
    fn collect(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Object(m) => {
                if let Some(Value::String(r)) = m.get("$ref") {
                    out.push(r.clone());
                }
                m.iter().for_each(|(_, v)| collect(v, out));
            }
            Value::Array(a) => a.iter().for_each(|v| collect(v, out)),
            _ => {}
        }
    }
    let mut queue = Vec::new();
    let components = doc.get("components");
    for kind in ["parameters", "requestBodies"] {
        if let Some(v) = components.and_then(|c| c.get(kind)) {
            collect(v, &mut queue);
        }
    }
    let groups = [doc.get("paths"), doc.get("webhooks"), components.and_then(|c| c.get("pathItems"))];
    for item in groups.into_iter().flatten().filter_map(Value::as_object).flat_map(|g| g.iter().map(|(_, v)| v)) {
        let Some(item) = item.as_object() else { continue };
        for (k, v) in item.iter() {
            if k == "parameters" {
                collect(v, &mut queue);
            } else if k.parse::<HttpMethod>().is_ok() {
                for part in ["parameters", "requestBody"] {
                    if let Some(p) = v.get(part) {
                        collect(p, &mut queue);
                    }
                }
            }
        }
    }
    let mut seen = FxHashSet::default();
    while let Some(r) = queue.pop() {
        if let Some(target) = resolve_ref_quiet(doc, &r)
            && seen.insert(r)
        {
            collect(target, &mut queue);
        }
    }
    seen
}

fn record_alias(name: &str) -> Stmt {
    Stmt::TypeAlias { name: name.into(), ty: record(STRING, NEVER) }
}

const READ_WRITE_HELPERS: [(&str, &str); 4] = [
    ("$Read", "export type $Read<T> = {\n    readonly $read: T;\n};"),
    ("$Write", "export type $Write<T> = {\n    readonly $write: T;\n};"),
    (
        "Readable",
        "export type Readable<T> = T extends $Write<any> ? never : T extends $Read<infer U> ? Readable<U> : T extends (infer E)[] ? Readable<E>[] : T extends object ? {\n    [K in keyof T as NonNullable<T[K]> extends $Write<any> ? never : K]: Readable<T[K]>;\n} : T;",
    ),
    (
        "Writable",
        "export type Writable<T> = T extends $Read<any> ? never : T extends $Write<infer U> ? Writable<U> : T extends (infer E)[] ? Writable<E>[] : T extends object ? {\n    [K in keyof T as NonNullable<T[K]> extends $Read<any> ? never : K]: Writable<T[K]>;\n} & {\n    [K in keyof T as NonNullable<T[K]> extends $Read<any> ? K : never]?: never;\n} : T;",
    ),
];

/// `transformSchema` (`transform/index.ts`): the whole document.
pub fn transform_document(
    doc: &Value,
    opts: &Options,
    discriminators: &Discriminators,
    host: &mut dyn Host,
) -> Result<(Vec<Stmt>, Vec<String>)> {
    let mut ctx = Ctx {
        opts,
        root: doc,
        host,
        hook_schema: None,
        discriminators,
        footer: Vec::new(),
        footer_names: FxHashSet::default(),
        regex_state: InvalidCharsRegexState::default(),
        enum_cache: FxHashMap::default(),
        warnings: Vec::new(),
        request_refs: if opts.fix.contains(Fix::RequestDefaults) { request_refs(doc) } else { FxHashSet::default() },
    };
    let mut out = Vec::new();
    let mut alias_range = 0..0;
    if opts.read_write_markers {
        for (name, text) in READ_WRITE_HELPERS {
            let text = if opts.fix.contains(Fix::ReadWritePrimitives) {
                text.replace(
                    " : T extends object ?",
                    " : T extends string | number | boolean | bigint | symbol ? T : T extends object ?",
                )
            } else {
                text.into()
            };
            out.push(Stmt::Raw { name: name.into(), text });
        }
    }
    if let Some(inject) = opts.inject.as_deref().map(js_trim).filter(|s| !s.is_empty()) {
        out.push(Stmt::Raw { name: String::new(), text: inject.to_owned() });
    }
    for root in ["paths", "webhooks", "components", "$defs"] {
        let Some(node) = doc.get(root).filter(|n| n.truthy() && n.is_js_object()) else {
            out.push(record_alias(root));
            continue;
        };
        let (ty, extra) = match root {
            "paths" => (paths(&mut ctx, node)?, Vec::new()),
            "webhooks" => (webhooks(&mut ctx, node)?, Vec::new()),
            "components" => components(&mut ctx, node)?,
            _ => {
                ctx.hook_schema = Some(node);
                let ty = transform_schema(&mut ctx, node, &create_ref(&["$defs"]), false)?;
                ctx.hook_schema = None;
                (ty, Vec::new())
            }
        };
        match ty {
            Ty::Object(members) if !members.is_empty() => {
                if opts.export_type {
                    out.push(Stmt::TypeAlias { name: root.into(), ty: Ty::Object(members) });
                } else {
                    out.push(Stmt::Interface { name: root.into(), members });
                }
            }
            _ => out.push(record_alias(root)),
        }
        if root == "components" {
            alias_range = out.len()..out.len() + extra.len();
        }
        out.extend(extra);
    }
    let has_operations = ctx.footer.iter().any(|s| matches!(s, Stmt::Interface { name, .. } if name == "operations"));
    out.append(&mut ctx.footer);
    if !has_operations {
        out.push(record_alias("operations"));
    }
    if opts.make_paths_enum
        && let Some(p) = doc.get("paths").filter(|p| p.truthy())
    {
        out.push(paths_enum(&mut ctx, p));
    }
    if opts.fix.contains(Fix::RootTypesNames) {
        rename_colliding_aliases(&mut out, &alias_range);
    }
    Ok((out, ctx.warnings))
}

/// `makeApiPathsEnum`.
fn paths_enum(ctx: &mut Ctx, paths: &Value) -> Stmt {
    let mut members = Vec::new();
    for (url, item) in raw_entries(paths) {
        for (method, op) in raw_entries(item) {
            if method.parse::<HttpMethod>().is_err() {
                continue;
            }
            let name = match op.get("operationId").filter(|id| id.truthy()) {
                Some(id) => id.js_string(),
                None => format!("{method}{url}")
                    .split('/')
                    .map(|part| {
                        let mut chars = part.chars();
                        let capitalised = match chars.next() {
                            Some(c) if c.len_utf16() == 1 => c.to_uppercase().collect::<String>() + chars.as_str(),
                            _ => part.to_owned(),
                        };
                        strip_first_path_noise(&capitalised)
                    })
                    .collect(),
            };
            members.push(EnumMember {
                name: ctx.regex_state.enum_member_name(&name),
                value: EnumValue::Str(url.to_string()),
                comment: None,
            });
        }
    }
    Stmt::Enum { name: ctx.regex_state.enum_name("ApiPaths"), members }
}

/// `part.replace(/{.*}|:.*|[^a-zA-Z\d_]+/, "")`: removes the first match only.
fn strip_first_path_noise(part: &str) -> String {
    let bytes = part.as_bytes();
    let line_end =
        |from: usize| part[from..].find(['\n', '\r', '\u{2028}', '\u{2029}']).map_or(part.len(), |i| from + i);
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'{' {
            let end = line_end(i);
            if let Some(close) = part[i..end].rfind('}') {
                return format!("{}{}", &part[..i], &part[i + close + 1..]);
            }
        }
        if b == b':' {
            let end = line_end(i);
            return format!("{}{}", &part[..i], &part[end..]);
        }
        if !(b.is_ascii_alphanumeric() || b == b'_') {
            let end = part[i..].find(|c: char| c.is_ascii_alphanumeric() || c == '_').map_or(part.len(), |j| i + j);
            return format!("{}{}", &part[..i], &part[end..]);
        }
    }
    part.to_owned()
}

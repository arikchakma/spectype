//! `no-invalid-media-type-examples`: examples validated against their schema the way Redocly's
//! Ajv 2020 instance (`@redocly/ajv` 8.11, `allErrors`, `discriminator`, no formats) does it.
//!
//! Ajv compiles a schema before validating, and any compile error ("nullable" without "type", a
//! keyword with the wrong value type, an unsupported discriminator) replaces all validation
//! errors with one "Example validation errored" warning. Keywords run in Ajv's rule order
//! (untyped, then number, string, array and object keywords), which decides the error order.
//! With `allowAdditionalProperties: false` every object schema with `properties` behaves as if
//! it had `unevaluatedProperties: false`, except directly under `allOf`.

use rustc_hash::FxHashSet;

use super::{Cx, Docs, Loc, Rule, Target, V, js_string};
use crate::bundle::Node;
use crate::bundle::types::Ty;
use spectype_document::pointer::escape_pointer;
use spectype_document::value::{Value, js_number, js_string_to_number};

pub(super) struct MediaTypeExamples {
    pub allow_additional: bool,
    /// Schemas (by node identity) Ajv already compiled successfully, as its instance caches them.
    pub compiled: FxHashSet<usize>,
}

impl Rule for MediaTypeExamples {
    fn visitors(&self) -> &'static [V] {
        const VS: &[V] =
            &[V { target: Target::T(Ty::MediaType), enter: None, leave: Some(0), weak: &[], children: &[] }];
        VS
    }

    fn visit(&mut self, _: u8, media: &Node, cx: &mut Cx<'_>) {
        let Some(schema) = media.get("schema").filter(Node::truthy) else { return };
        if let Some(example) = media.get("example") {
            let at = cx.location.child("example");
            self.check(&example, at, false, &schema, cx);
        } else if let Some(examples) = media.get("examples").filter(Node::truthy) {
            for name in super::rules::keys(&examples) {
                let Some(example) = examples.get(&name) else { continue };
                let at = cx.location.child("examples").child(&name).child("value");
                self.check(&example, at, true, &schema, cx);
            }
        }
    }
}

impl MediaTypeExamples {
    fn check(&mut self, example: &Node, at: Loc, multiple: bool, schema: &Node, cx: &mut Cx<'_>) {
        let (example, at) = if example.is_ref() {
            let Some((node, loc)) = cx.resolve(example) else { return };
            let at = if multiple { loc.child("value") } else { loc };
            (node, at)
        } else {
            (example.clone(), at)
        };
        let data = if multiple {
            match example.get("value") {
                Some(v) => v,
                None => return,
            }
        } else {
            example
        };
        let schema_at = cx.location.child("schema");
        let docs = cx.docs();
        let root = Base::root(schema, cx.absolute(&schema_at));
        let mut ajv = Ajv { docs, compiled: &mut self.compiled, compiling: FxHashSet::default(), mutated: false };
        let compiled = ajv.compile_root(schema, &root);
        if ajv.mutated {
            cx.mark_mutated();
        }
        if let Err(message) = compiled {
            if message != "discriminator: requires oneOf or anyOf composite keyword" {
                cx.report_at(&format!("Example validation errored: {message}."), &schema_at);
            }
            return;
        }
        let base = at.pointer();
        let mut run = Run {
            docs,
            forced: !self.allow_additional,
            errors: Vec::new(),
            active: FxHashSet::default(),
            depth: 0,
            overflow: false,
        };
        let level = Level { allof: false, called_from_allof: false, top_data: true };
        run.validate(schema, &root, &data, &base, level, true);
        if run.overflow {
            cx.report_at("Example validation errored: Maximum call stack size exceeded.", &schema_at);
            return;
        }
        for e in run.errors {
            let rel = e.path.get(base.len() + 1..).unwrap_or_default();
            let prop = rel.rsplit('/').next().unwrap_or_default();
            let mut message = e.message;
            if e.keyword == "type" {
                message = format!("type {message}");
            }
            if !prop.is_empty() {
                message = format!("`{prop}` property {message}");
            }
            let mut path = e.path;
            if let Some(extra) = e.property {
                message = format!("{message} `{extra}`");
                path = format!("{path}/{}", escape_pointer(&extra));
            }
            let loc = Loc::new(at.doc, &path);
            cx.report_at(&format!("Example value must conform to the schema: {message}."), &loc);
        }
    }
}

/// Every keyword Ajv 2020 registers (`RULES.all`).
const KEYWORDS: &[&str] = &[
    "$async",
    "$dynamicAnchor",
    "$dynamicRef",
    "$recursiveAnchor",
    "$recursiveRef",
    "$schema",
    "$id",
    "$defs",
    "$vocabulary",
    "$comment",
    "definitions",
    "id",
    "$ref",
    "type",
    "nullable",
    "const",
    "enum",
    "not",
    "anyOf",
    "oneOf",
    "allOf",
    "if",
    "then",
    "else",
    "propertyNames",
    "additionalProperties",
    "dependencies",
    "properties",
    "patternProperties",
    "prefixItems",
    "items",
    "contains",
    "format",
    "title",
    "description",
    "default",
    "deprecated",
    "readOnly",
    "writeOnly",
    "examples",
    "contentMediaType",
    "contentEncoding",
    "contentSchema",
    "dependentRequired",
    "dependentSchemas",
    "maxContains",
    "minContains",
    "unevaluatedProperties",
    "unevaluatedItems",
    "maximum",
    "minimum",
    "exclusiveMaximum",
    "exclusiveMinimum",
    "multipleOf",
    "maxLength",
    "minLength",
    "pattern",
    "maxProperties",
    "minProperties",
    "required",
    "maxItems",
    "minItems",
    "uniqueItems",
    "discriminator",
];

/// Keywords with code or a schema type, in the order Ajv runs them.
const ORDER: &[(&str, &str)] = &[
    ("$dynamicAnchor", "[\"string\"]"),
    ("$dynamicRef", "[\"string\"]"),
    ("$ref", "[\"string\"]"),
    ("id", ""),
    ("type", "[\"string\",\"array\"]"),
    ("nullable", "[\"boolean\"]"),
    ("const", ""),
    ("enum", "[\"array\"]"),
    ("not", "[\"object\",\"boolean\"]"),
    ("anyOf", "[\"array\"]"),
    ("oneOf", "[\"array\"]"),
    ("allOf", "[\"array\"]"),
    ("if", "[\"object\",\"boolean\"]"),
    ("then", "[\"object\",\"boolean\"]"),
    ("else", "[\"object\",\"boolean\"]"),
    ("maximum", "[\"number\"]"),
    ("minimum", "[\"number\"]"),
    ("exclusiveMaximum", "[\"number\"]"),
    ("exclusiveMinimum", "[\"number\"]"),
    ("multipleOf", "[\"number\"]"),
    ("format", "[\"string\"]"),
    ("maxLength", "[\"number\"]"),
    ("minLength", "[\"number\"]"),
    ("pattern", "[\"string\"]"),
    ("maxItems", "[\"number\"]"),
    ("minItems", "[\"number\"]"),
    ("prefixItems", "[\"array\"]"),
    ("items", "[\"object\",\"boolean\"]"),
    ("contains", "[\"object\",\"boolean\"]"),
    ("uniqueItems", "[\"boolean\"]"),
    ("maxContains", "[\"number\"]"),
    ("minContains", "[\"number\"]"),
    ("unevaluatedItems", "[\"boolean\",\"object\"]"),
    ("maxProperties", "[\"number\"]"),
    ("minProperties", "[\"number\"]"),
    ("required", "[\"array\"]"),
    ("propertyNames", "[\"object\",\"boolean\"]"),
    ("additionalProperties", "[\"boolean\",\"object\"]"),
    ("dependencies", "[\"object\"]"),
    ("properties", "[\"object\"]"),
    ("patternProperties", "[\"object\"]"),
    ("dependentRequired", "[\"object\"]"),
    ("dependentSchemas", "[\"object\"]"),
    ("unevaluatedProperties", "[\"boolean\",\"object\"]"),
    ("discriminator", "[\"object\"]"),
];

const OBJECT_PROTO: [&str; 12] = [
    "constructor",
    "__defineGetter__",
    "__defineSetter__",
    "hasOwnProperty",
    "__lookupGetter__",
    "__lookupSetter__",
    "isPrototypeOf",
    "propertyIsEnumerable",
    "toString",
    "valueOf",
    "__proto__",
    "toLocaleString",
];

fn in_group(keyword: &str, group: &str) -> bool {
    keyword == "format" && matches!(group, "number" | "string") || group_of(keyword) == Some(group)
}

fn group_of(keyword: &str) -> Option<&'static str> {
    Some(match keyword {
        "maximum" | "minimum" | "exclusiveMaximum" | "exclusiveMinimum" | "multipleOf" => "number",
        "maxLength" | "minLength" | "pattern" => "string",
        "maxItems" | "minItems" | "prefixItems" | "items" | "contains" | "uniqueItems" | "maxContains"
        | "minContains" | "unevaluatedItems" => "array",
        "maxProperties"
        | "minProperties"
        | "required"
        | "propertyNames"
        | "additionalProperties"
        | "dependencies"
        | "properties"
        | "patternProperties"
        | "dependentRequired"
        | "dependentSchemas"
        | "unevaluatedProperties"
        | "discriminator" => "object",
        _ => return None,
    })
}

fn keys(n: &Node) -> Vec<String> {
    match n {
        Node::Obj(m) => m.borrow().keys().cloned().collect(),
        _ => Vec::new(),
    }
}

fn has_rules(s: &Node) -> bool {
    match s {
        Node::Prim(Value::Bool(b)) => !b,
        Node::Obj(m) => m.borrow().keys().any(|k| KEYWORDS.contains(&k.as_str())),
        _ => false,
    }
}

fn has_rules_but_ref(s: &Node) -> bool {
    matches!(s, Node::Obj(m) if m.borrow().keys().any(|k| k != "$ref" && KEYWORDS.contains(&k.as_str())))
}

/// `alwaysValidSchema` on a `null` subschema throws (`Object.keys(null)`).
fn not_null(s: &Node) -> Result<(), String> {
    if matches!(s, Node::Prim(Value::Null)) {
        return Err("Cannot convert undefined or null to object".into());
    }
    Ok(())
}

/// `alwaysValidSchema`. Strings and numbers count as schemas without rules.
fn always_valid(s: &Node) -> bool {
    match s {
        Node::Prim(Value::Bool(b)) => *b,
        Node::Obj(_) => !has_rules(s),
        Node::Prim(Value::Null) => false,
        _ => true,
    }
}

/// `validSchemaType`.
fn valid_schema_type(v: &Node, ty: &str) -> bool {
    if ty.is_empty() {
        return true;
    }
    let ok = |t: &str| match t {
        "array" => matches!(v, Node::Arr(_)),
        "object" => matches!(v, Node::Obj(_)),
        "string" => matches!(v, Node::Prim(Value::String(_))),
        "number" => matches!(v, Node::Prim(Value::Number(_))),
        "boolean" => matches!(v, Node::Prim(Value::Bool(_))),
        _ => false,
    };
    ty.trim_matches(['[', ']']).split(',').any(|t| ok(t.trim_matches('"')))
}

const JSON_TYPES: [&str; 7] = ["string", "number", "integer", "boolean", "null", "object", "array"];

/// `getSchemaTypes`, with `nullable: true` adding `null`.
fn schema_types(s: &Node) -> Result<Vec<String>, String> {
    let raw: Vec<Node> = match s.get("type") {
        Some(Node::Arr(a)) => a.borrow().clone(),
        Some(t) if t.truthy() => vec![t],
        _ => Vec::new(),
    };
    let mut types = Vec::new();
    for t in &raw {
        let Some(t) = t.as_str().filter(|t| JSON_TYPES.contains(&t.as_str())) else {
            let list =
                raw.iter().map(|t| if matches!(t, Node::Prim(Value::Null)) { String::new() } else { js_string(t) });
            return Err(format!("type must be JSONType or JSONType[]: {}", list.collect::<Vec<_>>().join(",")));
        };
        types.push(t);
    }
    let nullable = s.get("nullable");
    if types.iter().any(|t| t == "null") {
        if matches!(nullable, Some(Node::Prim(Value::Bool(false)))) {
            return Err("type: null contradicts nullable: false".into());
        }
    } else {
        if types.is_empty() && nullable.is_some() {
            return Err("\"nullable\" cannot be used without \"type\"".into());
        }
        if matches!(nullable, Some(Node::Prim(Value::Bool(true)))) {
            types.push("null".into());
        }
    }
    Ok(types)
}

fn data_has_type(d: &Node, t: &str) -> bool {
    match t {
        "number" => matches!(d, Node::Prim(Value::Number(_))),
        "integer" => matches!(d, Node::Prim(Value::Number(n)) if n.fract() == 0.0),
        "string" => matches!(d, Node::Prim(Value::String(_))),
        "boolean" => matches!(d, Node::Prim(Value::Bool(_))),
        "null" => matches!(d, Node::Prim(Value::Null)),
        "object" => matches!(d, Node::Obj(_)),
        "array" => matches!(d, Node::Arr(_)),
        _ => false,
    }
}

/// Whether the schema uses a keyword of `group`'s data type (`shouldUseGroup`).
fn uses_group(s: &Node, group: &str) -> bool {
    keys(s).iter().any(|k| in_group(k, group))
}

struct Ajv<'a> {
    docs: &'a dyn Docs,
    compiled: &'a mut FxHashSet<usize>,
    compiling: FxHashSet<usize>,
    mutated: bool,
}

impl Ajv<'_> {
    /// `getSchemaTypes` pushes `"null"` onto a `type` array it was handed by reference.
    fn schema_types(&mut self, s: &Node) -> Result<Vec<String>, String> {
        let types = schema_types(s)?;
        if let Some(Node::Arr(list)) = s.get("type")
            && types.last().is_some_and(|t| t == "null")
            && !list.borrow().iter().any(|t| t.as_str().as_deref() == Some("null"))
        {
            list.borrow_mut().push(Node::Prim(Value::String("null".into())));
            self.mutated = true;
        }
        Ok(types)
    }

    fn compile_root(&mut self, s: &Node, base: &Base) -> Result<(), String> {
        if !matches!(s, Node::Obj(_)) {
            return Ok(());
        }
        schema_refs(s, &base.id)?;
        self.compile_obj(s, base, true)
    }

    fn compile(&mut self, s: &Node, base: &Base) -> Result<(), String> {
        if matches!(s, Node::Obj(_)) && has_rules(s) { self.compile_obj(s, &base.enter(s), false) } else { Ok(()) }
    }

    fn compile_ref(&mut self, r: &str, base: &Base) -> Result<(), String> {
        let Some((target, target_base)) = base.lookup(self.docs, r) else {
            return Err(format!("can't resolve reference {r} from id {}", base.id));
        };
        let Some(target) = target.filter(|t| matches!(t, Node::Obj(_))) else { return Ok(()) };
        let Some(id) = target.id() else { return Ok(()) };
        if self.compiled.contains(&id) || !self.compiling.insert(id) {
            return Ok(());
        }
        let result = schema_refs(&target, &target_base.id).and_then(|()| self.compile_obj(&target, &target_base, true));
        self.compiling.remove(&id);
        if result.is_ok() {
            self.compiled.insert(id);
        }
        result
    }

    /// The compile half of `subschemaCode`: everything that can throw before data is seen.
    fn compile_obj(&mut self, s: &Node, base: &Base, root: bool) -> Result<(), String> {
        self.schema_types(s)?;
        if let Some(r) = s.get("$ref")
            && r.as_str().is_some()
            && !root
            && !has_rules_but_ref(s)
        {
            return self.compile_ref(&js_string(&r), base);
        }
        for (kw, schema_type) in ORDER {
            let Some(v) = s.get(kw) else { continue };
            if !valid_schema_type(&v, schema_type) {
                return Err(format!("{kw} value must be {schema_type}"));
            }
            match *kw {
                "id" => return Err("NOT SUPPORTED: keyword \"id\", use \"$id\" for schema ID".into()),
                "$ref" => self.compile_ref(&js_string(&v), base)?,
                "enum" if matches!(&v, Node::Arr(a) if a.borrow().is_empty()) => {
                    return Err("enum must have non-empty array".into());
                }
                "not"
                | "items"
                | "propertyNames"
                | "additionalProperties"
                | "unevaluatedProperties"
                | "unevaluatedItems"
                | "contains" => {
                    if !always_valid(&v) {
                        self.compile(&v, base)?;
                    }
                }
                "anyOf" => {
                    list(&v).iter().try_for_each(not_null)?;
                    for b in list(&v) {
                        self.compile(&b, base)?;
                    }
                }
                "oneOf" if s.get("discriminator").is_none() => {
                    for b in list(&v) {
                        not_null(&b)?;
                        if !always_valid(&b) {
                            self.compile(&b, base)?;
                        }
                    }
                }
                "allOf" | "prefixItems" => {
                    for b in list(&v) {
                        not_null(&b)?;
                        if !always_valid(&b) {
                            self.compile(&b, base)?;
                        }
                    }
                }
                "if" => {
                    for clause in ["then", "else"] {
                        if let Some(c) = s.get(clause) {
                            not_null(&c)?;
                        }
                    }
                    let then = s.get("then").filter(|t| !always_valid(t));
                    let els = s.get("else").filter(|t| !always_valid(t));
                    if then.is_some() || els.is_some() {
                        self.compile(&v, base)?;
                        for clause in [then, els].into_iter().flatten() {
                            self.compile(&clause, base)?;
                        }
                    }
                }
                "uniqueItems" if v.truthy() => {
                    if let Some(items) = s.get("items") {
                        self.schema_types(&items)?;
                    }
                }
                "properties" | "patternProperties" | "dependentSchemas" | "dependencies" => {
                    if matches!(*kw, "properties" | "patternProperties") {
                        let all = keys(&v).into_iter().filter(|k| k != "__proto__").filter_map(|k| v.get(&k));
                        all.collect::<Vec<_>>().iter().try_for_each(not_null)?;
                    }
                    for k in keys(&v) {
                        if k == "__proto__" {
                            continue;
                        }
                        let Some(sub) = v.get(&k) else { continue };
                        if *kw == "dependencies" && matches!(sub, Node::Arr(_)) {
                            continue;
                        }
                        not_null(&sub)?;
                        if !always_valid(&sub) {
                            self.compile(&sub, base)?;
                        }
                    }
                }
                "discriminator" => {
                    let mapping = self.mapping(s, &v, base, true)?;
                    let keyword = if s.get("oneOf").is_some() { "oneOf" } else { "anyOf" };
                    let variants = list(&s.get(keyword).unwrap_or(Node::Prim(Value::Null)));
                    for (_, i) in mapping {
                        if let Some(b) = variants.get(i) {
                            self.compile(b, base)?;
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// The discriminator's `getMapping`: tag value → variant index.
    fn mapping(&mut self, s: &Node, d: &Node, base: &Base, compile: bool) -> Result<Vec<(String, usize)>, String> {
        let Some(tag) = d.get("propertyName").and_then(|t| t.as_str()) else {
            return Err("discriminator: requires propertyName".into());
        };
        let keyword = if s.get("oneOf").is_some_and(|v| v.truthy()) {
            "oneOf"
        } else if s.get("anyOf").is_some_and(|v| v.truthy()) {
            "anyOf"
        } else {
            return Err("discriminator: requires oneOf or anyOf composite keyword".into());
        };
        let variants = list(&s.get(keyword).unwrap_or(Node::Prim(Value::Null)));
        let required = |sch: &Node| matches!(sch.get("required"), Some(Node::Arr(a)) if a.borrow().iter().any(|r| r.as_str().as_deref() == Some(&tag)));
        let top_required = required(s);
        let mut tag_required = true;
        let mut out: Vec<(String, usize)> = Vec::new();
        let add = |value: &Node, i: usize, out: &mut Vec<(String, usize)>| {
            let Some(v) =
                value.as_str().filter(|v| !out.iter().any(|(k, _)| k == v) && !OBJECT_PROTO.contains(&v.as_str()))
            else {
                return Err(format!("discriminator: \"{tag}\" values must be unique strings"));
            };
            out.push((v, i));
            Ok(())
        };
        for (i, variant) in variants.iter().enumerate() {
            let mut sch = variant.clone();
            let sch_ref = sch.get("$ref").and_then(|r| r.as_str());
            if let (Some(r), Some(mapping)) = (&sch_ref, d.get("mapping")) {
                let matched: Vec<String> = keys(&mapping)
                    .into_iter()
                    .filter(|k| mapping.get(k).and_then(|m| m.as_str()).as_ref() == Some(r))
                    .collect();
                if !matched.is_empty() {
                    for k in matched {
                        add(&Node::Prim(Value::String(k)), i, &mut out)?;
                    }
                    continue;
                }
            }
            if let Some(r) = &sch_ref
                && !has_rules_but_ref(&sch)
            {
                if compile {
                    self.compile_ref(r, base)?;
                }
                sch = base.lookup(self.docs, r).and_then(|(t, _)| t).unwrap_or(Node::Prim(Value::Null));
            }
            let prop = sch.get("properties").and_then(|p| p.get(&tag));
            let Some(prop @ (Node::Obj(_) | Node::Arr(_) | Node::Prim(Value::Null))) = prop else {
                return Err(format!(
                    "discriminator: {keyword} subschemas (or referenced schemas) must have \"properties/{tag}\" or match mapping"
                ));
            };
            tag_required = tag_required && (top_required || required(&sch));
            if let Some(c) = prop.get("const").filter(Node::truthy) {
                add(&c, i, &mut out)?;
            } else if let Some(values) = prop.get("enum").filter(Node::truthy) {
                for v in list(&values) {
                    add(&v, i, &mut out)?;
                }
            } else {
                return Err(format!("discriminator: \"properties/{tag}\" must have \"const\" or \"enum\""));
            }
        }
        if !tag_required {
            return Err(format!("discriminator: \"{tag}\" must be required"));
        }
        let mut sorted: Vec<(String, usize)> = out.iter().filter(|(k, _)| array_index(k)).cloned().collect();
        sorted.sort_by_key(|(k, _)| k.parse::<u64>().unwrap_or_default());
        sorted.extend(out.into_iter().filter(|(k, _)| !array_index(k)));
        Ok(sorted)
    }
}

fn array_index(k: &str) -> bool {
    k.parse::<u32>().is_ok_and(|n| n.to_string() == k && n < u32::MAX)
}

fn list(v: &Node) -> Vec<Node> {
    match v {
        Node::Arr(a) => a.borrow().clone(),
        _ => Vec::new(),
    }
}

/// Ajv's `baseId` for the schema being compiled, which decides where `$ref`s resolve. A
/// `$ref` is handed to Redocly's resolver relative to the file part of it.
#[derive(Clone)]
struct Base {
    id: String,
}

impl Base {
    /// `addSchema({ $id: location, ...schema })`: the schema's own `$id` wins.
    fn root(schema: &Node, location: String) -> Self {
        let own = schema.get("$id").and_then(|i| i.as_str());
        Self { id: normalize_id(&own.unwrap_or(location)) }
    }

    /// `updateContext`: a subschema's `$id` resolves against the current base.
    fn enter(&self, s: &Node) -> Self {
        match s.get("$id").and_then(|i| i.as_str()) {
            Some(id) => Self { id: resolve_url(&self.id, &normalize_id(&id)) },
            None => self.clone(),
        }
    }

    /// Redocly's `loadSchemaSync`: the target and its `{ $id: absoluteRef + '#' + ref, ...node }`.
    fn lookup(&self, docs: &dyn Docs, r: &str) -> Option<(Option<Node>, Self)> {
        let file = self.id.split('#').next().unwrap_or_default();
        let doc = docs.doc_index(file)?;
        let (node, target_doc, _) = docs.resolve(doc, r)?;
        let full = resolve_url(&self.id, &normalize_id(r));
        let own = node.as_ref().and_then(|n| n.get("$id")).and_then(|i| i.as_str());
        let id = own.map_or_else(|| format!("{}#{full}", docs.source(target_doc)), |own| normalize_id(&own));
        Some((node, Self { id }))
    }
}

fn normalize_id(id: &str) -> String {
    id.strip_suffix("#/").or_else(|| id.strip_suffix('#')).unwrap_or(id).to_owned()
}

/// `uri-js` reference resolution, enough for file paths and `http(s)` URLs. A base without a
/// scheme is a path-only URI, which it serializes without the leading `/`.
fn resolve_url(base: &str, r: &str) -> String {
    let has_scheme = |s: &str| {
        s.split_once(':').is_some_and(|(scheme, _)| {
            !scheme.is_empty() && scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
        })
    };
    if has_scheme(r) {
        return r.to_owned();
    }
    let base_file = base.split('#').next().unwrap_or_default();
    let (prefix, path) = match base_file.find("://") {
        Some(i) => {
            let host_end = base_file[i + 3..].find('/').map_or(base_file.len(), |j| i + 3 + j);
            (&base_file[..host_end], &base_file[host_end..])
        }
        None => ("", base_file),
    };
    let (r_path, fragment) = match r.find('#') {
        Some(i) => (&r[..i], &r[i..]),
        None => (r, ""),
    };
    let merged = if r_path.is_empty() {
        path.to_owned()
    } else if r_path.starts_with('/') {
        r_path.to_owned()
    } else {
        match path.rfind('/') {
            Some(i) => format!("{}{r_path}", &path[..=i]),
            None => r_path.to_owned(),
        }
    };
    let mut segments: Vec<&str> = Vec::new();
    for seg in merged.split('/') {
        match seg {
            "." => {}
            ".." => {
                segments.pop();
            }
            s => segments.push(s),
        }
    }
    let mut path = segments.join("/");
    if prefix.is_empty() {
        path = path.trim_start_matches('/').replace(' ', "%20");
    }
    format!("{prefix}{path}{fragment}")
}

/// `getSchemaRefs` when Ajv adds a schema: every nested `$id` and `$anchor` must be unique.
fn schema_refs(schema: &Node, base: &str) -> Result<(), String> {
    fn walk(n: &Node, base: &str, top: bool, seen: &mut FxHashSet<String>) -> Result<(), String> {
        let Node::Obj(map) = n else { return Ok(()) };
        let mut base = base.to_owned();
        if !top {
            let mut add = |r: &str, base: &str| -> Result<String, String> {
                let full = normalize_id(&if base.is_empty() { r.to_owned() } else { resolve_url(base, r) });
                if !seen.insert(full.clone()) {
                    return Err(format!("reference \"{full}\" resolves to more than one schema"));
                }
                Ok(full)
            };
            if let Some(id) = n.get("$id").and_then(|i| i.as_str()) {
                base = add(&id, &base)?;
            }
            for key in ["$anchor", "$dynamicAnchor"] {
                if let Some(anchor) = n.get(key).and_then(|a| a.as_str()) {
                    let mut chars = anchor.chars();
                    let valid = chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                        && chars.all(|c| c.is_ascii_alphanumeric() || "-._".contains(c));
                    if !valid {
                        return Err(format!("invalid anchor \"{anchor}\""));
                    }
                    add(&format!("#{anchor}"), &base)?;
                }
            }
        }
        let entries: Vec<(String, Node)> = map.borrow().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        for (k, v) in entries {
            if let Node::Arr(items) = &v {
                if matches!(k.as_str(), "items" | "allOf" | "anyOf" | "oneOf") {
                    for item in items.borrow().iter() {
                        walk(item, &base, false, seen)?;
                    }
                }
            } else if matches!(
                k.as_str(),
                "$defs" | "definitions" | "properties" | "patternProperties" | "dependencies"
            ) {
                if let Node::Obj(props) = &v {
                    let values: Vec<Node> = props.borrow().values().cloned().collect();
                    for value in values {
                        walk(&value, &base, false, seen)?;
                    }
                }
            } else if !matches!(
                k.as_str(),
                "default"
                    | "enum"
                    | "const"
                    | "required"
                    | "maximum"
                    | "minimum"
                    | "exclusiveMaximum"
                    | "exclusiveMinimum"
                    | "multipleOf"
                    | "maxLength"
                    | "minLength"
                    | "pattern"
                    | "format"
                    | "maxItems"
                    | "minItems"
                    | "uniqueItems"
                    | "maxProperties"
                    | "minProperties"
            ) {
                walk(&v, &base, false, seen)?;
            }
        }
        Ok(())
    }
    walk(schema, base, true, &mut FxHashSet::default())
}

struct AjvError {
    keyword: &'static str,
    message: String,
    path: String,
    /// `additionalProperty` / `unevaluatedProperty`.
    property: Option<String>,
}

/// Which properties a schema evaluated, for `unevaluatedProperties`.
#[derive(Clone)]
enum Props {
    Some(Vec<String>),
    All,
}

impl Props {
    fn merge(&mut self, other: Self) {
        match (self, other) {
            (Self::All, _) => {}
            (me, Self::All) => *me = Self::All,
            (Self::Some(a), Self::Some(b)) => a.extend(b),
        }
    }

    fn has(&self, k: &str) -> bool {
        match self {
            Self::All => true,
            Self::Some(a) => a.iter().any(|x| x == k),
        }
    }
}

#[derive(Clone, Copy)]
struct Level {
    /// A direct `allOf` subschema (`it.isAllOfVariant`).
    allof: bool,
    /// The enclosing `$ref` function was called from an `allOf` subschema.
    called_from_allof: bool,
    /// Still at the data of the enclosing function's top level.
    top_data: bool,
}

struct Run<'a> {
    docs: &'a dyn Docs,
    forced: bool,
    errors: Vec<AjvError>,
    active: FxHashSet<(usize, usize)>,
    depth: usize,
    overflow: bool,
}

const MAX_DEPTH: usize = 1_000;

impl Run<'_> {
    fn error(&mut self, keyword: &'static str, message: String, path: &str) {
        self.errors.push(AjvError { keyword, message, path: path.to_owned(), property: None });
    }

    /// Validates `data` at `path`; returns whether no errors were added and the properties
    /// evaluated.
    fn validate(&mut self, s: &Node, base: &Base, data: &Node, path: &str, lv: Level, root: bool) -> (bool, Props) {
        if self.overflow {
            return (true, Props::Some(Vec::new()));
        }
        let start = self.errors.len();
        let mut props = Props::Some(Vec::new());
        match s {
            _ if root && !matches!(s, Node::Obj(_)) => return (true, props),
            Node::Prim(Value::Bool(false)) => {
                self.error("false schema", "boolean schema is false".into(), path);
                return (false, props);
            }
            Node::Obj(_) if root || has_rules(s) => {}
            _ => return (true, props),
        }
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            self.overflow = true;
            self.depth -= 1;
            return (true, props);
        }
        self.keywords(s, base, data, path, lv, root, &mut props);
        self.depth -= 1;
        (self.errors.len() == start, props)
    }

    fn sub(&mut self, s: &Node, base: &Base, data: &Node, path: &str, lv: Level) -> (bool, Props) {
        self.validate(s, &base.enter(s), data, path, lv, false)
    }

    #[expect(clippy::too_many_arguments, reason = "walker state threaded through the recursion")]
    fn keywords(&mut self, s: &Node, base: &Base, data: &Node, path: &str, lv: Level, root: bool, props: &mut Props) {
        let types = schema_types(s).unwrap_or_default();
        let single_group = types.len() == 1 && matches!(types[0].as_str(), "number" | "string" | "array" | "object");
        let checked = !(types.is_empty() || single_group && uses_group(s, &types[0]));
        if checked && !types.iter().any(|t| data_has_type(data, t)) {
            let ty = s.get("type").map(|t| js_string(&t)).unwrap_or_default();
            self.error("type", format!("must be {ty}"), path);
        }
        let only_ref = !root && s.get("$ref").and_then(|r| r.as_str()).is_some() && !has_rules_but_ref(s);
        let same = Level { allof: false, ..lv };
        if let Some(r) = s.get("$ref").and_then(|r| r.as_str()) {
            self.call_ref(&r, base, data, path, lv, props);
            if only_ref {
                return;
            }
        }
        if let Some(c) = s.get("const")
            && !deep_equal(data, &c)
        {
            self.error("const", "must be equal to constant".into(), path);
        }
        if let Some(Node::Arr(values)) = s.get("enum")
            && !values.borrow().iter().any(|v| deep_equal(data, v))
        {
            let allowed =
                values.borrow().iter().map(|v| format!("\"{}\"", js_string(v))).collect::<Vec<_>>().join(", ");
            self.error("enum", format!("must be equal to one of the allowed values {allowed}"), path);
        }
        if let Some(not) = s.get("not") {
            let mark = self.errors.len();
            let valid = always_valid(&not) || self.sub(&not, base, data, path, same).0;
            self.errors.truncate(mark);
            if valid {
                self.error("not", "must NOT be valid".into(), path);
            }
        }
        if let Some(any) = s.get("anyOf") {
            let mark = self.errors.len();
            let mut ok = false;
            for b in list(&any) {
                let (v, p) = self.sub(&b, base, data, path, same);
                if v {
                    ok = true;
                    props.merge(p);
                }
            }
            if ok {
                self.errors.truncate(mark);
            } else {
                self.error("anyOf", "must match a schema in anyOf".into(), path);
            }
        }
        if let Some(one) = s.get("oneOf")
            && s.get("discriminator").is_none()
        {
            let mark = self.errors.len();
            let mut passing = 0;
            for b in list(&one) {
                if always_valid(&b) {
                    passing += 1;
                    continue;
                }
                let (v, p) = self.sub(&b, base, data, path, same);
                if v {
                    passing += 1;
                    props.merge(p);
                }
            }
            if passing == 1 {
                self.errors.truncate(mark);
            } else {
                self.error("oneOf", "must match exactly one schema in oneOf".into(), path);
            }
        }
        if let Some(all) = s.get("allOf") {
            for b in list(&all) {
                if always_valid(&b) {
                    continue;
                }
                let (_, p) = self.sub(&b, base, data, path, Level { allof: true, ..lv });
                props.merge(p);
            }
        }
        if let Some(cond) = s.get("if") {
            let then = s.get("then").filter(|t| !always_valid(t));
            let els = s.get("else").filter(|t| !always_valid(t));
            if then.is_some() || els.is_some() {
                let mark = self.errors.len();
                let (cond_ok, p) = self.sub(&cond, base, data, path, same);
                self.errors.truncate(mark);
                props.merge(p);
                let (clause, name) = if cond_ok { (then, "then") } else { (els, "else") };
                if let Some(clause) = clause {
                    let (v, p) = self.sub(&clause, base, data, path, same);
                    if v {
                        props.merge(p);
                    } else {
                        self.error("if", format!("must match \"{name}\" schema"), path);
                    }
                }
            }
        }
        let type_error_in_group = |group: &str| !checked && single_group && types[0] == group;
        let ty_message = || format!("must be {}", s.get("type").map(|t| js_string(&t)).unwrap_or_default());
        if uses_group(s, "number") {
            if let Node::Prim(Value::Number(n)) = data {
                self.numbers(s, *n, path);
            } else if type_error_in_group("number") {
                self.error("type", ty_message(), path);
            }
        }
        if uses_group(s, "string") {
            if let Node::Prim(Value::String(text)) = data {
                self.strings(s, text, path);
            } else if type_error_in_group("string") {
                self.error("type", ty_message(), path);
            }
        }
        if uses_group(s, "array") {
            if let Node::Arr(items) = data {
                let items = items.borrow().clone();
                self.arrays(s, base, &items, path, lv);
            } else if type_error_in_group("array") {
                self.error("type", ty_message(), path);
            }
        }
        if uses_group(s, "object") {
            if let Node::Obj(_) = data {
                self.objects(s, base, data, path, lv, props);
            } else if type_error_in_group("object") {
                self.error("type", ty_message(), path);
            }
        }
    }

    fn call_ref(&mut self, r: &str, base: &Base, data: &Node, path: &str, lv: Level, props: &mut Props) {
        let Some((target, target_base)) = base.lookup(self.docs, r) else { return };
        let Some(target) = target else { return };
        let guard = match (target.id(), data.id()) {
            (Some(t), Some(d)) => Some((t, d)),
            _ => None,
        };
        if let Some(g) = guard
            && !self.active.insert(g)
        {
            return;
        }
        let level = Level { allof: false, called_from_allof: lv.allof, top_data: true };
        let (valid, p) = self.validate(&target, &target_base, data, path, level, true);
        if let Some(g) = guard {
            self.active.remove(&g);
        }
        if valid {
            props.merge(p);
        }
    }

    fn numbers(&mut self, s: &Node, n: f64, path: &str) {
        for (kw, op) in [("maximum", "<="), ("minimum", ">="), ("exclusiveMaximum", "<"), ("exclusiveMinimum", ">")] {
            let Some(Node::Prim(Value::Number(limit))) = s.get(kw) else { continue };
            let ok = match op {
                "<=" => n <= limit,
                ">=" => n >= limit,
                "<" => n < limit,
                _ => n > limit,
            };
            if !ok {
                self.error("limit", format!("must be {op} {}", js_number(limit)), path);
            }
        }
        if let Some(Node::Prim(Value::Number(m))) = s.get("multipleOf") {
            let res = n / m;
            if m == 0.0 || res.is_nan() || res.to_bits() != parse_int(&js_number(res)).to_bits() {
                self.error("multipleOf", format!("must be multiple of {}", js_number(m)), path);
            }
        }
    }

    fn strings(&mut self, s: &Node, text: &str, path: &str) {
        let len = text.chars().count();
        #[expect(clippy::cast_precision_loss, reason = "JavaScript numbers are f64, so the rounding matches")]
        let len = len as f64;
        if let Some(Node::Prim(Value::Number(max))) = s.get("maxLength")
            && len > max
        {
            self.error("maxLength", format!("must NOT have more than {} characters", js_number(max)), path);
        }
        if let Some(Node::Prim(Value::Number(min))) = s.get("minLength")
            && len < min
        {
            self.error("minLength", format!("must NOT have fewer than {} characters", js_number(min)), path);
        }
        if let Some(pattern) = s.get("pattern").and_then(|p| p.as_str())
            && let Some(re) = js_regex(&pattern)
            && !re.is_match(text)
        {
            self.error("pattern", format!("must match pattern \"{pattern}\""), path);
        }
    }

    fn arrays(&mut self, s: &Node, base: &Base, items: &[Node], path: &str, lv: Level) {
        #[expect(clippy::cast_precision_loss, reason = "JavaScript numbers are f64, so the rounding matches")]
        let len = items.len() as f64;
        let child = Level { allof: false, called_from_allof: lv.called_from_allof, top_data: false };
        if let Some(Node::Prim(Value::Number(max))) = s.get("maxItems")
            && len > max
        {
            self.error("maxItems", format!("must NOT have more than {} items", js_number(max)), path);
        }
        if let Some(Node::Prim(Value::Number(min))) = s.get("minItems")
            && len < min
        {
            self.error("minItems", format!("must NOT have fewer than {} items", js_number(min)), path);
        }
        let prefix = s.get("prefixItems").map(|p| list(&p));
        if let Some(prefix) = &prefix {
            for (i, (b, item)) in prefix.iter().zip(items).enumerate() {
                if !always_valid(b) {
                    self.sub(b, base, item, &format!("{path}/{i}"), child);
                }
            }
        }
        if let Some(schema) = s.get("items")
            && !always_valid(&schema)
        {
            let from = prefix.as_ref().map_or(0, Vec::len);
            if prefix.is_some() && matches!(schema, Node::Prim(Value::Bool(false))) {
                if items.len() > from {
                    self.error("items", format!("must NOT have more than {from} items"), path);
                }
            } else {
                for (i, item) in items.iter().enumerate().skip(from) {
                    self.sub(&schema, base, item, &format!("{path}/{i}"), child);
                }
            }
        }
        if let Some(contains) = s.get("contains") {
            self.contains(s, &contains, base, items, path, child);
        }
        if s.get("uniqueItems").is_some_and(|u| u.truthy())
            && let Some((i, j)) = duplicate(s, items)
        {
            self.error(
                "uniqueItems",
                format!("must NOT have duplicate items (items ## {j} and {i} are identical)"),
                path,
            );
        }
    }

    fn contains(&mut self, s: &Node, contains: &Node, base: &Base, items: &[Node], path: &str, child: Level) {
        let num = |k: &str| match s.get(k) {
            Some(Node::Prim(Value::Number(n))) => Some(n),
            _ => None,
        };
        let min = num("minContains").unwrap_or(1.0);
        let max = num("maxContains");
        let message = match max {
            None => format!("must contain at least {} valid item(s)", js_number(min)),
            Some(m) => {
                format!("must contain at least {} and no more than {} valid item(s)", js_number(min), js_number(m))
            }
        };
        if max.is_none() && min == 0.0 {
            return;
        }
        if max.is_some_and(|m| min > m) {
            self.error("contains", message, path);
            return;
        }
        #[expect(clippy::cast_precision_loss, reason = "JavaScript numbers are f64, so the rounding matches")]
        let len = items.len() as f64;
        if always_valid(contains) {
            if len < min || max.is_some_and(|m| len > m) {
                self.error("contains", message, path);
            }
            return;
        }
        let mark = self.errors.len();
        let mut count = 0.0;
        let mut valid = min == 0.0;
        for (i, item) in items.iter().enumerate() {
            if min == 0.0 && max.is_some() && items.is_empty() {
                break;
            }
            let (v, _) = self.sub(contains, base, item, &format!("{path}/{i}"), child);
            if !v {
                continue;
            }
            count += 1.0;
            match max {
                None => {
                    if count >= min {
                        valid = true;
                        break;
                    }
                }
                Some(m) => {
                    if count > m {
                        valid = false;
                        break;
                    }
                    if count >= min {
                        valid = true;
                    }
                }
            }
        }
        if valid {
            self.errors.truncate(mark);
        } else {
            self.error("contains", message, path);
        }
    }

    fn objects(&mut self, s: &Node, base: &Base, data: &Node, path: &str, lv: Level, props: &mut Props) {
        let data_keys = keys(data);
        let child = Level { allof: false, called_from_allof: lv.called_from_allof, top_data: false };
        let at = |k: &str| format!("{path}/{}", escape_pointer(k));
        #[expect(clippy::cast_precision_loss, reason = "JavaScript numbers are f64, so the rounding matches")]
        let count = data_keys.len() as f64;
        if let Some(Node::Prim(Value::Number(max))) = s.get("maxProperties")
            && count > max
        {
            self.error("maxProperties", format!("must NOT have more than {} properties", js_number(max)), path);
        }
        if let Some(Node::Prim(Value::Number(min))) = s.get("minProperties")
            && count < min
        {
            self.error("minProperties", format!("must NOT have fewer than {} properties", js_number(min)), path);
        }
        let present = |k: &str| data.get(k).is_some() || OBJECT_PROTO.contains(&k);
        if let Some(required) = s.get("required") {
            for r in list(&required) {
                let r = js_string(&r);
                if !present(&r) {
                    self.error("required", format!("must have required property '{r}'"), path);
                }
            }
        }
        if let Some(names) = s.get("propertyNames")
            && !always_valid(&names)
        {
            for k in &data_keys {
                let (v, _) = self.sub(&names, base, &Node::Prim(Value::String(k.clone())), path, child);
                if !v {
                    self.error("propertyNames", "property name must be valid".into(), path);
                }
            }
        }
        let defined: Vec<String> = s.get("properties").map(|p| keys(&p)).unwrap_or_default();
        let patterns: Vec<String> = s.get("patternProperties").map(|p| keys(&p)).unwrap_or_default();
        let matches_pattern = |k: &str| patterns.iter().any(|p| js_regex(p).is_some_and(|re| re.is_match(k)));
        if let Some(extra) = s.get("additionalProperties") {
            *props = Props::All;
            if !always_valid(&extra) {
                for k in &data_keys {
                    if defined.contains(k) || matches_pattern(k) {
                        continue;
                    }
                    if matches!(extra, Node::Prim(Value::Bool(false))) {
                        self.errors.push(AjvError {
                            keyword: "additionalProperties",
                            message: "must NOT have additional properties".into(),
                            path: path.to_owned(),
                            property: Some(k.clone()),
                        });
                    } else if let (Node::Obj(_), Some(v)) = (&extra, data.get(k)) {
                        self.sub(&extra, base, &v, &at(k), child);
                    }
                }
            }
        }
        if let Some(deps) = s.get("dependencies") {
            self.dependencies(&deps, base, data, path, lv, props, true);
        }
        if let Some(properties) = s.get("properties") {
            if !defined.is_empty() {
                props.merge(Props::Some(defined.iter().filter(|k| *k != "__proto__").cloned().collect()));
            }
            for k in &defined {
                if k == "__proto__" {
                    continue;
                }
                let (Some(schema), Some(v)) = (properties.get(k), data.get(k)) else { continue };
                if !always_valid(&schema) {
                    self.sub(&schema, base, &v, &at(k), child);
                }
            }
        }
        if let Some(pattern_props) = s.get("patternProperties") {
            for p in &patterns {
                let Some(re) = js_regex(p) else { continue };
                let Some(schema) = pattern_props.get(p) else { continue };
                for k in &data_keys {
                    if !re.is_match(k) {
                        continue;
                    }
                    if !always_valid(&schema)
                        && let Some(v) = data.get(k)
                    {
                        self.sub(&schema, base, &v, &at(k), child);
                    }
                    props.merge(Props::Some(vec![k.clone()]));
                }
            }
        }
        if let Some(deps) = s.get("dependentRequired") {
            self.dependencies(&deps, base, data, path, lv, props, false);
        }
        if let Some(deps) = s.get("dependentSchemas") {
            for k in keys(&deps) {
                let Some(schema) = deps.get(&k) else { continue };
                if always_valid(&schema) || data.get(&k).is_none() {
                    continue;
                }
                let (v, p) = self.sub(&schema, base, data, path, Level { allof: false, ..lv });
                if v {
                    props.merge(p);
                }
            }
        }
        let explicit = s.get("unevaluatedProperties");
        let has_props =
            s.get("properties").is_some_and(|p| p.truthy()) || s.get("patternProperties").is_some_and(|p| p.truthy());
        let forced =
            explicit.is_none() && self.forced && has_props && !lv.allof && !(lv.top_data && lv.called_from_allof);
        if forced || explicit.is_some() {
            let schema = explicit.clone().unwrap_or(Node::Prim(Value::Bool(false)));
            if !props.has_all() {
                for k in &data_keys {
                    if props.has(k) {
                        continue;
                    }
                    if matches!(schema, Node::Prim(Value::Bool(false))) {
                        self.errors.push(AjvError {
                            keyword: "unevaluatedProperties",
                            message: "must NOT have unevaluated properties".into(),
                            path: path.to_owned(),
                            property: Some(k.clone()),
                        });
                    } else if !always_valid(&schema)
                        && let Some(v) = data.get(k)
                    {
                        self.sub(&schema, base, &v, &at(k), child);
                    }
                }
            }
            if explicit.is_some() {
                *props = Props::All;
            }
        }
        if let Some(d) = s.get("discriminator") {
            self.discriminate(s, &d, base, data, path, lv, props);
        }
    }

    #[expect(clippy::too_many_arguments, reason = "walker state threaded through the recursion")]
    fn dependencies(
        &mut self,
        deps: &Node,
        base: &Base,
        data: &Node,
        path: &str,
        lv: Level,
        props: &mut Props,
        schemas: bool,
    ) {
        let all = keys(deps);
        let (arrays, rest): (Vec<String>, Vec<String>) =
            all.into_iter().filter(|k| k != "__proto__").partition(|k| matches!(deps.get(k), Some(Node::Arr(_))));
        for k in arrays.into_iter().chain(rest) {
            let Some(dep) = deps.get(&k) else { continue };
            if let Node::Arr(list) = &dep {
                let list: Vec<String> = list.borrow().iter().map(js_string).collect();
                if list.is_empty() || data.get(&k).is_none() {
                    continue;
                }
                let word = if list.len() == 1 { "property" } else { "properties" };
                for d in &list {
                    if data.get(d).is_none() {
                        let message = format!("must have {word} {} when property {k} is present", list.join(", "));
                        self.error("dependencies", message, path);
                    }
                }
            } else if schemas && !always_valid(&dep) && data.get(&k).is_some() {
                let (v, p) = self.sub(&dep, base, data, path, Level { allof: false, ..lv });
                if v {
                    props.merge(p);
                }
            }
        }
    }

    #[expect(clippy::too_many_arguments, reason = "walker state threaded through the recursion")]
    fn discriminate(&mut self, s: &Node, d: &Node, base: &Base, data: &Node, path: &str, lv: Level, props: &mut Props) {
        let Some(tag) = d.get("propertyName").and_then(|t| t.as_str()) else { return };
        let Some(value) = data.get(&tag).and_then(|v| v.as_str()) else {
            self.error("discriminator", format!("tag \"{tag}\" must be string"), path);
            return;
        };
        let keyword = if s.get("oneOf").is_some_and(|v| v.truthy()) { "oneOf" } else { "anyOf" };
        let variants = list(&s.get(keyword).unwrap_or(Node::Prim(Value::Null)));
        let mut compiled = FxHashSet::default();
        let mut ajv = Ajv { docs: self.docs, compiled: &mut compiled, compiling: FxHashSet::default(), mutated: false };
        let mapping = ajv.mapping(s, d, base, false).unwrap_or_default();
        match mapping.iter().find(|(k, _)| *k == value) {
            Some((_, i)) => {
                if let Some(b) = variants.get(*i) {
                    let (_, p) = self.sub(b, base, data, path, Level { allof: false, ..lv });
                    props.merge(p);
                }
            }
            None => self.error("discriminator", format!("value of tag \"{tag}\" must be in oneOf or anyOf"), path),
        }
    }
}

impl Props {
    fn has_all(&self) -> bool {
        matches!(self, Self::All)
    }
}

/// `parseInt(String(x))`.
fn parse_int(s: &str) -> f64 {
    let t = s.trim_start();
    let (sign, digits) = match t.strip_prefix('-') {
        Some(rest) => (-1.0, rest),
        None => (1.0, t.strip_prefix('+').unwrap_or(t)),
    };
    let end = digits.find(|c: char| !c.is_ascii_digit()).unwrap_or(digits.len());
    if end == 0 {
        return f64::NAN;
    }
    sign * js_string_to_number(&digits[..end])
}

/// The first duplicate pair Ajv's `uniqueItems` reports, as `(i, j)` in its message order.
fn duplicate(s: &Node, items: &[Node]) -> Option<(usize, usize)> {
    let types = s.get("items").and_then(|i| schema_types(&i).ok()).unwrap_or_default();
    let scalar = !types.is_empty() && !types.iter().any(|t| t == "object" || t == "array");
    if scalar {
        let mut seen: Vec<(String, usize)> = Vec::new();
        for i in (0..items.len()).rev() {
            let item = &items[i];
            if !types.iter().any(|t| data_has_type(item, t)) {
                continue;
            }
            let mut key = js_string(item);
            if types.len() > 1 && matches!(item, Node::Prim(Value::String(_))) {
                key.push('_');
            }
            if let Some((_, j)) = seen.iter().find(|(k, _)| *k == key) {
                return Some((i, *j));
            }
            seen.push((key, i));
        }
        return None;
    }
    for i in (0..items.len()).rev() {
        for j in (0..i).rev() {
            if deep_equal(&items[i], &items[j]) {
                return Some((i, j));
            }
        }
    }
    None
}

/// `fast-deep-equal`.
#[expect(clippy::float_cmp, reason = "mirrors JavaScript `===` on numbers")]
fn deep_equal(a: &Node, b: &Node) -> bool {
    match (a, b) {
        (Node::Prim(Value::Number(x)), Node::Prim(Value::Number(y))) => x == y || (x.is_nan() && y.is_nan()),
        (Node::Prim(x), Node::Prim(y)) => x == y,
        (Node::Arr(x), Node::Arr(y)) => {
            let (x, y) = (x.borrow(), y.borrow());
            x.len() == y.len() && x.iter().zip(y.iter()).all(|(a, b)| deep_equal(a, b))
        }
        (Node::Obj(x), Node::Obj(y)) => {
            let (x, y) = (x.borrow(), y.borrow());
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| deep_equal(v, w)))
        }
        _ => false,
    }
}

/// `new RegExp(pattern, "u")`, translated to the `regex` crate's dialect where they differ.
fn js_regex(pattern: &str) -> Option<regex::Regex> {
    let mut out = String::new();
    let mut chars = pattern.chars().peekable();
    let mut in_class = false;
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('d') => out.push_str("[0-9]"),
                Some('D') => out.push_str("[^0-9]"),
                Some('w') => out.push_str("[0-9A-Za-z_]"),
                Some('W') => out.push_str("[^0-9A-Za-z_]"),
                Some('b') if !in_class => out.push_str("(?-u:\\b)"),
                Some('B') if !in_class => out.push_str("(?-u:\\B)"),
                Some(n) => {
                    out.push('\\');
                    out.push(n);
                }
                None => out.push('\\'),
            },
            '[' if !in_class => {
                in_class = true;
                out.push('[');
                if chars.peek() == Some(&'^') {
                    out.push('^');
                    chars.next();
                }
                if chars.peek() == Some(&']') {
                    chars.next();
                    out.push_str("\\]");
                }
            }
            '[' => out.push_str("\\["),
            ']' if in_class => {
                in_class = false;
                out.push(']');
            }
            '.' if !in_class => out.push_str("[^\\n\\r\\u{2028}\\u{2029}]"),
            '&' | '~' | '-' if in_class && chars.peek() == Some(&c) => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    regex::Regex::new(&out).ok()
}

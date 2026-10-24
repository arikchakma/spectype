//! `transform/schema-object.ts`.

use std::borrow::Cow;

use super::{Ctx, entries, oapi_ref, oapi_ref_with, resolve_ref_quiet};
use crate::Fix;
use crate::names::property_index;
use crate::ts::{BOOLEAN, EnumMember, EnumValue, Member, NEVER, NUMBER, STRING, Stmt, Ty, UNKNOWN};
use spectype_document::oas::HttpMethod;
use spectype_document::pointer::{JsonPointer, create_ref, parse_ref};
use spectype_document::value::{Map, Value, js_number, js_trim};
use spectype_document::{Error, Result};

/// `tsLiteral`.
pub fn literal(value: Option<&Value>) -> Ty {
    match value {
        Some(Value::String(s)) => Ty::Raw(spectype_document::value::json_quote(s)),
        Some(Value::Number(n)) => number_literal(*n),
        Some(Value::Bool(b)) => Ty::Bool(*b),
        Some(Value::Null) => Ty::Null,
        Some(Value::Array(a)) if a.is_empty() => Ty::Array(Box::new(NEVER)),
        Some(Value::Array(a)) => Ty::Tuple(a.iter().map(|v| literal(Some(v))).collect()),
        Some(Value::Object(m)) if m.is_empty() => record(STRING, NEVER),
        Some(Value::Object(m)) => {
            Ty::Object(m.iter().map(|(k, v)| Member::prop(property_index(k), false, false, literal(Some(v)))).collect())
        }
        None => UNKNOWN,
    }
}

pub fn number_literal(n: f64) -> Ty {
    if n < 0.0 { Ty::Num(format!("-{}", js_number(-n))) } else { Ty::Num(js_number(n)) }
}

fn enum_value(v: &Value) -> EnumValue {
    match v {
        Value::Number(n) if *n < 0.0 => EnumValue::Num(format!("-{}", js_number(-n))),
        Value::Number(n) => EnumValue::Num(js_number(*n)),
        other => EnumValue::Str(other.js_string()),
    }
}

pub fn record(key: Ty, value: Ty) -> Ty {
    Ty::Ref("Record".into(), vec![key, value])
}

fn is_primitive(t: &Ty) -> bool {
    matches!(t, Ty::Keyword("boolean" | "never" | "number" | "string" | "undefined") | Ty::Null)
}

/// `tsDedupe`: drops repeated primitive keywords and `null`.
fn dedupe(types: Vec<Ty>) -> Vec<Ty> {
    let mut seen: Vec<Ty> = Vec::new();
    let mut out = Vec::with_capacity(types.len());
    for t in types {
        if is_primitive(&t) {
            if seen.contains(&t) {
                continue;
            }
            seen.push(t.clone());
        }
        out.push(t);
    }
    out
}

/// `tsUnion`.
pub fn union(mut types: Vec<Ty>) -> Ty {
    match types.len() {
        0 => NEVER,
        1 => types.remove(0),
        _ => Ty::Union(dedupe(types)),
    }
}

/// `tsIntersection`.
pub fn intersection(mut types: Vec<Ty>) -> Ty {
    match types.len() {
        0 => NEVER,
        1 => types.remove(0),
        _ => Ty::Intersection(dedupe(types)),
    }
}

/// `tsWithRequired`.
fn with_required(ctx: &mut Ctx, ty: Ty, keys: &[String]) -> Ty {
    if keys.is_empty() {
        return ty;
    }
    ctx.inject_helper("WithRequired", "type WithRequired<T, K extends keyof T> = T & {\n    [P in K]-?: T[P];\n};");
    let keys = union(keys.iter().map(|k| Ty::Raw(spectype_document::value::json_quote(k))).collect());
    Ty::Ref("WithRequired".into(), vec![ty, keys])
}

fn omit(ty: Ty, key: Option<&Value>) -> Ty {
    Ty::Ref("Omit".into(), vec![ty, Ty::Union(vec![literal(key)])])
}

/// `Object.keys(x).length` for the shapes `OpenAPI` documents contain.
fn key_count(v: Option<&Value>) -> usize {
    match v {
        Some(Value::Object(m)) => m.len(),
        Some(Value::Array(a)) => a.len(),
        Some(Value::String(s)) => s.encode_utf16().count(),
        _ => 0,
    }
}

fn truthy(v: Option<&Value>) -> bool {
    v.is_some_and(Value::truthy)
}

/// `schemaObject.required?.includes(key)`.
fn required_includes(required: Option<&Value>, key: &str) -> bool {
    match required {
        Some(Value::Array(a)) => a.iter().any(|v| v.as_str() == Some(key)),
        Some(Value::String(s)) => s.contains(key),
        _ => false,
    }
}

fn string_list(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a.iter().map(Value::js_string).collect(),
        _ => Vec::new(),
    }
}

fn type_is(schema: &Map, name: &str) -> bool {
    schema.get("type").and_then(Value::as_str) == Some(name)
}

/// `transformSchemaObject`.
pub fn transform_schema(ctx: &mut Ctx, schema: &Value, path: &JsonPointer, from_additional: bool) -> Result<Ty> {
    let ty = with_composition(ctx, schema, path, from_additional)?;
    ctx.post_transform_hook(ty, path)
}

/// `transformSchemaObjectWithComposition`.
fn with_composition(ctx: &mut Ctx, schema: &Value, path: &JsonPointer, from_additional: bool) -> Result<Ty> {
    let obj = match schema {
        v if !v.truthy() => return Ok(NEVER),
        Value::Bool(true) => return Ok(UNKNOWN),
        Value::Object(m) => m,
        Value::Array(_) => return Err(Error::at(path, "Expected SchemaObject, received Array")),
        other => {
            let kind = if matches!(other, Value::Number(_)) { "number" } else { "string" };
            return Err(Error::at(path, format!("Expected SchemaObject, received {kind}")));
        }
    };

    if let Some(r) = obj.get("$ref") {
        return schema_ref(ctx, &super::js_text(r), path);
    }

    if let Some(c) = obj.get("const").filter(|c| !matches!(c, Value::Null)) {
        return Ok(literal(Some(c)));
    }

    if let Some(Value::Array(values)) = obj.get("enum")
        && (!obj.contains_key("type") || !type_is(obj, "object"))
        && !obj.contains_key("properties")
        && !obj.contains_key("additionalProperties")
    {
        return transform_enum(ctx, obj, values, path, from_additional);
    }

    let core = transform_core(ctx, schema, obj, path)?;
    let all_of = match obj.get("allOf") {
        Some(Value::Array(items)) => collect_all_of(ctx, items, obj.get("required"), path)?,
        _ => Vec::new(),
    };
    let mut final_ty = match (core, all_of.is_empty()) {
        (core, true) => core,
        (None, false) => Some(intersection(all_of)),
        (Some(core), false) => Some(intersection(vec![core, intersection(all_of)])),
    };

    let any_of = match obj.get("anyOf") {
        Some(Value::Array(items)) => collect_union(ctx, items, "anyOf", path)?,
        _ => Vec::new(),
    };
    if !any_of.is_empty() {
        if ctx.opts.fix.contains(Fix::AnyOfProperties) && !any_of.iter().all(is_primitive) {
            final_ty = Some(intersection(final_ty.into_iter().chain([union(any_of)]).collect()));
        } else {
            final_ty = Some(union(final_ty.into_iter().chain(any_of).collect()));
        }
    }

    let one_of_items = match obj.get("oneOf") {
        Some(v) if v.truthy() => v.as_array().map(Vec::as_slice),
        _ if type_is(obj, "object") => match obj.get("enum") {
            Some(Value::Array(items)) => Some(items.as_slice()),
            _ => None,
        },
        _ => None,
    };
    let one_of = match one_of_items {
        Some(items) => collect_union(ctx, items, "oneOf", path)?,
        None => Vec::new(),
    };
    if !one_of.is_empty() {
        if one_of.iter().all(is_primitive) {
            final_ty = Some(union(final_ty.into_iter().chain(one_of).collect()));
        } else {
            final_ty = Some(intersection(final_ty.into_iter().chain([union(one_of)]).collect()));
        }
    }

    let final_ty = match final_ty {
        Some(t) => t,
        None if obj.contains_key("type") => {
            record(STRING, if ctx.opts.empty_objects_unknown { UNKNOWN } else { NEVER })
        }
        None => UNKNOWN,
    };

    if final_ty != UNKNOWN && truthy(obj.get("nullable")) {
        return Ok(Ty::Union(vec![final_ty, Ty::Null]));
    }
    Ok(final_ty)
}

fn transform_enum(ctx: &mut Ctx, obj: &Map, values: &[Value], path: &JsonPointer, from_additional: bool) -> Result<Ty> {
    if should_transform_to_ts_enum(ctx, obj, values) {
        let name = parse_ref(path).join("/").replacen("components/schemas", "", 1);
        let meta = |key: &str, alt: &str, i: usize| -> Option<Value> {
            let pick = |k: &str| match obj.get(k) {
                Some(Value::Array(a)) => a.get(i).filter(|v| !matches!(v, Value::Null)).cloned(),
                _ => None,
            };
            pick(key).or_else(|| pick(alt))
        };
        let metadata: Vec<(Option<String>, Option<String>)> = (0..values.len())
            .map(|i| {
                (
                    meta("x-enum-varnames", "x-enumNames", i).map(|v| v.js_string()),
                    meta("x-enum-descriptions", "x-enumDescriptions", i).map(|v| v.js_string()),
                )
            })
            .collect();
        let has_null = values.iter().any(|v| matches!(v, Value::Null))
            || ctx.opts.fix.contains(Fix::EnumNames)
                && (truthy(obj.get("nullable"))
                    || obj
                        .get("type")
                        .and_then(Value::as_array)
                        .is_some_and(|t| t.iter().any(|t| t.as_str() == Some("null"))));
        let valid: Vec<&Value> = values.iter().filter(|v| !matches!(v, Value::Null)).collect();
        let enum_name = ts_enum(ctx, &name, &valid, &metadata);
        let r = Ty::Ref(enum_name, Vec::new());
        return Ok(if has_null { union(vec![r, Ty::Null]) } else { r });
    }

    let mut types: Vec<Ty> = values.iter().map(|v| literal(Some(v))).collect();
    let type_has_null =
        matches!(obj.get("type"), Some(Value::Array(t)) if t.iter().any(|v| v.as_str() == Some("null")));
    if type_has_null || truthy(obj.get("nullable")) {
        types.push(Ty::Null);
    }
    let union_ty = union(types);

    if ctx.opts.enum_values
        && ctx.opts.fix.contains(Fix::EnumValuesType)
        && values.iter().all(|v| matches!(v, Value::String(_) | Value::Number(_)))
    {
        let var_name = format!("{}Values", parse_ref(path).join("/").replacen("components/schemas", "", 1));
        let base = ctx.regex_state.array_name(&var_name);
        let mut name = base.clone();
        let mut n = 1;
        while ctx.footer_names.contains(&name) {
            n += 1;
            name = format!("{base}_{n}");
        }
        let element = union(values.iter().map(|v| literal(Some(v))).collect());
        let ty = Ty::Readonly(Box::new(Ty::Array(Box::new(element))));
        ctx.push_footer(Stmt::ConstArray { name, ty, values: values.iter().map(enum_value).collect() });
    } else if ctx.opts.enum_values && values.iter().all(|v| matches!(v, Value::String(_) | Value::Number(_))) {
        let pointer = parse_ref(path);
        let var_name = format!("{}Values", pointer.join("/").replacen("components/schemas", "", 1));
        let mut cleaned: Vec<&str> = Vec::new();
        let mut extract: Vec<&str> = Vec::new();
        let mut i = 0;
        while i < pointer.len() {
            let seg = &pointer[i];
            if (seg == "anyOf" || seg == "oneOf") && i < pointer.len() - 1 {
                let next = &pointer[i + 1];
                if !next.is_empty() && next.bytes().all(|b| b.is_ascii_digit()) {
                    i += 1;
                    for s in &pointer[i + 1..] {
                        if s != "anyOf" && s != "oneOf" && !s.bytes().all(|b| b.is_ascii_digit()) {
                            extract.push(s);
                        }
                    }
                    i += 1;
                    continue;
                }
            }
            cleaned.push(seg);
            i += 1;
        }
        let cleaned_ref = create_ref(&cleaned);
        let mut element = oapi_ref_with(&cleaned_ref, None, true, &extract, path)?;
        if from_additional {
            element = Ty::Indexed(Box::new(element), Box::new(Ty::Ref("string".into(), Vec::new())));
        }
        let name = ctx.regex_state.array_name(&var_name);
        ctx.inject_helper(
            "FlattenedDeepRequired",
            "type FlattenedDeepRequired<T> = {\n    [K in keyof T]-?: FlattenedDeepRequired<T[K] extends unknown[] | undefined | null ? Extract<T[K], unknown[]>[number] : T[K]>;\n};",
        );
        ctx.inject_helper(
            "ReadonlyArray",
            "type ReadonlyArray<T> = [\n    Exclude<T, undefined>\n] extends [\n    unknown[]\n] ? Readonly<Exclude<T, undefined>> : Readonly<Exclude<T, undefined>[]>;",
        );
        let ty = Ty::Ref("ReadonlyArray".into(), vec![element]);
        ctx.push_footer(Stmt::ConstArray { name, ty, values: values.iter().map(enum_value).collect() });
    }
    Ok(union_ty)
}

fn should_transform_to_ts_enum(ctx: &Ctx, obj: &Map, values: &[Value]) -> bool {
    if !ctx.opts.enum_ {
        return false;
    }
    let fix = ctx.opts.fix.contains(Fix::EnumNames);
    let allowed = |v: &Value| matches!(v, Value::String(_) | Value::Number(_)) || (fix && matches!(v, Value::Null));
    if !values.iter().all(allowed) || (fix && values.iter().all(|v| matches!(v, Value::Null))) {
        return false;
    }
    if ctx.opts.conditional_enums {
        let has = ["x-enum-varnames", "x-enumNames", "x-enum-descriptions", "x-enumDescriptions"]
            .iter()
            .any(|k| matches!(obj.get(k), Some(Value::Array(_))));
        if !has {
            return false;
        }
    }
    true
}

/// Upstream quotes a fully invalid name without escaping it (and emits nothing for `""`); a real
/// string literal is always a valid member name.
fn fixed_member_name(sanitized: &str, raw: &str) -> String {
    if sanitized.is_empty() || (sanitized.len() >= 2 && sanitized.starts_with('"') && sanitized.ends_with('"')) {
        crate::ts::ts_string_literal(raw)
    } else {
        sanitized.to_owned()
    }
}

fn dedupe_member_names(mut members: Vec<EnumMember>) -> Vec<EnumMember> {
    let mut seen = rustc_hash::FxHashSet::default();
    for m in &mut members {
        let base = m.name.clone();
        let mut n = 1;
        while !seen.insert(m.name.clone()) {
            n += 1;
            m.name = match base.strip_suffix('"') {
                Some(open) => format!("{open}_{n}\""),
                None => format!("{base}_{n}"),
            };
        }
    }
    members
}

/// Makes `name` a valid TypeScript identifier: invalid characters become `_`, a leading digit gets
/// a `_` prefix, and a name with nothing left falls back to `fallback`.
pub fn valid_identifier(name: &str, fallback: &str) -> String {
    let mut out: String =
        name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '$' { c } else { '_' }).collect();
    if out.chars().all(|c| c == '_') {
        fallback.clone_into(&mut out);
    }
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

/// `tsEnum`; returns the declared (or cached) enum name.
fn ts_enum(ctx: &mut Ctx, name: &str, values: &[&Value], metadata: &[(Option<String>, Option<String>)]) -> String {
    let mut enum_name = ctx.regex_state.enum_name(name);
    let mut key = String::new();
    if ctx.opts.dedupe_enums {
        let mut sorted: Vec<String> = values.iter().map(|v| v.js_string()).collect();
        sorted.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
        key = sorted
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let (n, d) = metadata.get(i).cloned().unwrap_or_default();
                format!("{}:{}", n.unwrap_or_else(|| v.clone()), d.unwrap_or_default())
            })
            .collect::<Vec<_>>()
            .join(",");
        if let Some(cached) = ctx.enum_cache.get(&key) {
            return cached.clone();
        }
    }
    let members = values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let (n, d) = metadata.get(i).cloned().unwrap_or_default();
            let raw_name = n.unwrap_or_else(|| v.js_string());
            let comment = d.map(|d| js_trim(&d).to_owned()).filter(|d| !d.is_empty()).map(|d| format!(" {d}"));
            let name = if ctx.opts.fix.contains(Fix::EnumNames) {
                fixed_member_name(&ctx.regex_state.enum_member_name(&raw_name), &raw_name)
            } else {
                ctx.regex_state.enum_member_name(&raw_name)
            };
            EnumMember { name, value: enum_value(v), comment }
        })
        .collect::<Vec<_>>();
    let members = if ctx.opts.fix.contains(Fix::EnumNames) { dedupe_member_names(members) } else { members };
    if ctx.opts.fix.contains(Fix::EnumNames) {
        enum_name = valid_identifier(&enum_name, "Enum");
        let base = enum_name.clone();
        let mut n = 1;
        while ctx.footer_names.contains(&enum_name) {
            n += 1;
            enum_name = format!("{base}_{n}");
        }
    }
    if ctx.opts.dedupe_enums {
        ctx.enum_cache.insert(key, enum_name.clone());
    }
    ctx.push_footer(Stmt::Enum { name: enum_name.clone(), members });
    enum_name
}

fn collect_union(ctx: &mut Ctx, items: &[Value], key: &str, path: &JsonPointer) -> Result<Vec<Ty>> {
    let mut out = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        out.push(transform_schema(ctx, item, &create_ref(&[path.as_str(), key, &i.to_string()]), false)?);
    }
    Ok(out)
}

fn collect_all_of(ctx: &mut Ctx, items: &[Value], required: Option<&Value>, path: &JsonPointer) -> Result<Vec<Ty>> {
    let mut parent_required = string_list(required);
    let drop_bare =
        ctx.opts.fix.contains(Fix::AllOfEmptyObjects) && items.iter().any(|item| bare_object(ctx, item).is_none());
    if drop_bare {
        for key in items.iter().filter_map(|item| bare_object(ctx, item)).flat_map(|o| string_list(o.get("required"))) {
            if !parent_required.contains(&key) {
                parent_required.push(key);
            }
        }
    }
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        if drop_bare && bare_object(ctx, item).is_some() {
            continue;
        }
        let item_ty;
        let item_ref = item.get("$ref").map(Value::js_string);
        if let Some(r) = &item_ref {
            let mut ty = transform_schema(ctx, item, path, false)?;
            if let Some(Value::Object(resolved)) = ctx.resolve(r)
                && resolved.contains_key("properties")
                && (ctx.opts.fix.contains(Fix::AllOfRequired) || !ctx.discriminators.refs_handled.contains(r))
            {
                let props = resolved.get("properties");
                let valid: Vec<String> = parent_required
                    .iter()
                    .filter(|k| props.and_then(|p| p.get(k)).is_some_and(Value::truthy))
                    .cloned()
                    .collect();
                if !valid.is_empty() {
                    ty = with_required(ctx, ty, &valid);
                }
            }
            item_ty = ty;
        } else {
            let mut req: Vec<Value> = parent_required.iter().cloned().map(Value::String).collect();
            if let Some(Value::Array(own)) = item.get("required") {
                req.extend(own.iter().cloned());
            }
            let mut merged = item.as_object().cloned().unwrap_or_default();
            merged.insert("required".into(), Value::Array(req));
            item_ty = transform_schema(ctx, &Value::Object(merged), path, false)?;
        }
        let extends_only = ctx.opts.fix.contains(Fix::DiscriminatorOneOfBase)
            && ctx.discriminators.objects.get(path.as_str()).is_none();
        let discriminator = item_ref
            .as_ref()
            .filter(|_| !extends_only)
            .and_then(|r| ctx.discriminators.objects.get(r))
            .cloned()
            .or_else(|| item.get("discriminator").filter(|d| d.truthy()).cloned());
        match discriminator {
            Some(d) => out.push(omit(item_ty, d.get("propertyName"))),
            None => out.push(item_ty),
        }
    }
    Ok(out)
}

/// The `allOf` member (or its `$ref` target) when it is an object with no shape of its own, which
/// prints as `Record<string, never>` and turns every sibling property into `never`.
fn bare_object<'a>(ctx: &Ctx<'a, '_>, item: &'a Value) -> Option<&'a Map> {
    let mut obj = item.as_object()?;
    if let Some(r) = obj.get("$ref") {
        if obj.len() > 1 {
            return None;
        }
        obj = resolve_ref_quiet(ctx.root, &super::js_text(r))?.as_object()?;
    }
    let typed = obj.get("type").is_none_or(|t| t.as_str() == Some("object"));
    let shaped = [
        "$ref",
        "additionalProperties",
        "patternProperties",
        "$defs",
        "allOf",
        "anyOf",
        "oneOf",
        "enum",
        "const",
        "items",
        "prefixItems",
        "discriminator",
        "nullable",
    ]
    .iter()
    .any(|k| obj.contains_key(k))
        || key_count(obj.get("properties")) > 0;
    (typed && !shaped && (obj.contains_key("type") || obj.contains_key("required"))).then_some(obj)
}

fn schema_ref(ctx: &Ctx, r: &str, path: &JsonPointer) -> Result<Ty> {
    let ty = oapi_ref(r, path)?;
    Ok(if ctx.opts.fix.contains(Fix::DefsOptional) { non_nullable_defs(ty) } else { ty })
}

/// Wraps each `X["$defs"]` in a `$ref` type in `NonNullable`, since a schema's `$defs` is optional.
fn non_nullable_defs(ty: Ty) -> Ty {
    let Ty::Indexed(obj, key) = ty else { return ty };
    let obj = non_nullable_defs(*obj);
    let obj = match obj {
        Ty::Indexed(_, ref k) if **k == Ty::Str("$defs".into()) => Ty::Ref("NonNullable".into(), vec![obj]),
        obj => obj,
    };
    Ty::Indexed(Box::new(obj), key)
}

fn read_write(ctx: &Ctx, ty: Ty, read_only: bool, write_only: bool) -> Ty {
    if !ctx.opts.read_write_markers || (read_only && write_only) {
        return ty;
    }
    if read_only {
        return Ty::Ref("$Read".into(), vec![ty]);
    }
    if write_only {
        return Ty::Ref("$Write".into(), vec![ty]);
    }
    ty
}

/// `transformSchemaObjectCore`.
fn transform_core(ctx: &mut Ctx, schema: &Value, obj: &Map, path: &JsonPointer) -> Result<Option<Ty>> {
    if truthy(obj.get("type")) {
        if let Some(r) = ctx.transform_hook(schema, path)? {
            if r.question_token {
                return Ok(Some(Ty::Union(vec![r.ty, crate::ts::UNDEFINED])));
            }
            return Ok(Some(r.ty));
        }
        let ty = obj.get("type");
        match ty.and_then(Value::as_str) {
            Some("null") => return Ok(Some(Ty::Null)),
            Some("string") => return Ok(Some(STRING)),
            Some("number" | "integer") => return Ok(Some(NUMBER)),
            Some("boolean") => return Ok(Some(BOOLEAN)),
            Some("array") => return transform_array(ctx, obj, path).map(Some),
            _ => {}
        }
        if let Some(Value::Array(types)) = ty {
            let mut unique = Vec::with_capacity(types.len());
            let one_of = obj.get("oneOf").and_then(Value::as_array);
            for t in types {
                let is_null = matches!(t, Value::Null) || t.as_str() == Some("null");
                if let Some(one_of) = one_of {
                    let primitive = matches!(t.as_str(), Some("boolean" | "string" | "number" | "integer" | "null"));
                    if primitive && one_of.iter().any(|o| o.is_object() && o.get("type") == Some(t)) {
                        continue;
                    }
                    if is_null {
                        unique.push(Ty::Null);
                    } else {
                        let mut copy = obj.clone();
                        copy.insert("type".into(), t.clone());
                        copy.insert("oneOf".into(), Value::Null);
                        unique.push(transform_schema(ctx, &Value::Object(copy), path, false)?);
                    }
                } else if is_null {
                    unique.push(Ty::Null);
                } else {
                    let mut copy = obj.clone();
                    copy.insert("type".into(), t.clone());
                    unique.push(transform_schema(ctx, &Value::Object(copy), path, false)?);
                }
            }
            return Ok(Some(union(unique)));
        }
    }

    let mut core: Vec<Member> = Vec::with_capacity(key_count(obj.get("properties")));

    for k in ["allOf", "anyOf"] {
        if !truthy(obj.get(k)) {
            continue;
        }
        let discriminator = (!truthy(obj.get("discriminator"))
            && !ctx.discriminators.refs_handled.contains(path.as_str()))
        .then(|| ctx.discriminators.objects.get(path.as_str()).cloned())
        .flatten();
        if let Some(d) = discriminator {
            core.insert(0, discriminator_property(&d, path, ctx.opts.immutable));
            break;
        }
    }

    let has_props = key_count(obj.get("properties")) > 0 && truthy(obj.get("properties"));
    if has_props
        || truthy(obj.get("additionalProperties"))
        || truthy(obj.get("patternProperties"))
        || truthy(obj.get("$defs"))
    {
        if key_count(obj.get("properties")) > 0
            && let Some(props) = obj.get("properties")
        {
            for (k, v) in entries(ctx, props) {
                transform_property(ctx, obj, path, &k, v, &mut core)?;
            }
        }

        if let Some(defs @ (Value::Object(_) | Value::Array(_))) = obj.get("$defs")
            && key_count(Some(defs)) > 0
        {
            let mut def_members = Vec::new();
            for (k, v) in super::raw_entries(defs) {
                if !v.is_object() {
                    return Err(Error::at(path, format!("invalid $defs entry {k}: expected a Schema Object")));
                }
                let ro = truthy(v.get("readOnly"));
                let wo = truthy(v.get("writeOnly"));
                let inner = transform_schema(ctx, v, &create_ref(&[path.as_str(), "$defs", &k]), false)?;
                let ty = read_write(ctx, inner, ro, wo);
                let readonly = ctx.opts.immutable || (!ctx.opts.read_write_markers && ro);
                let m = Member::prop(property_index(&k), false, readonly, ty);
                def_members.push(ctx.property_hook(m, v, &create_ref(&[path.as_str(), "$defs", &k]))?);
            }
            let optional = ctx.opts.fix.contains(Fix::DefsOptional);
            core.push(Member::prop(property_index("$defs"), optional, false, Ty::Object(def_members)));
        }

        let ap = obj.get("additionalProperties");
        let has_extra = matches!(ap, Some(Value::Object(m)) if !m.is_empty())
            || matches!(ap, Some(Value::Array(a)) if !a.is_empty());
        let allows_any_extra = matches!(ap, Some(Value::Bool(true)))
            || matches!(ap, Some(Value::Object(m)) if m.is_empty())
            || matches!(ap, Some(Value::Array(a)) if a.is_empty());
        let patterns = obj.get("patternProperties");
        let has_patterns = key_count(patterns) > 0 && patterns.is_some_and(Value::is_js_object);
        let mut index_types = Vec::new();
        if has_extra && let Some(extra) = ap {
            index_types.push(transform_schema(ctx, extra, path, true)?);
        }
        if allows_any_extra || (!truthy(ap) && ctx.opts.additional_properties) {
            index_types.push(UNKNOWN);
        }
        if has_patterns && let Some(patterns) = patterns {
            for (_, v) in entries(ctx, patterns) {
                index_types.push(transform_schema(ctx, v, path, false)?);
            }
        }
        if index_types.is_empty() {
            return Ok((!core.is_empty()).then_some(Ty::Object(core)));
        }
        let index = Member::index("key", ctx.opts.immutable, STRING, union(index_types));
        let mut parts = Vec::new();
        if !core.is_empty() {
            parts.push(Ty::Object(core));
        }
        parts.push(Ty::Object(vec![index]));
        return Ok(Some(intersection(parts)));
    }

    Ok((!core.is_empty()).then_some(Ty::Object(core)))
}

/// Whether defaults must not make properties required here. Upstream looks for the substrings
/// `parameters`/`requestBody`/`requestBodies` anywhere in the pointer, which also matches schema
/// and property names; the fix checks where the schema actually sits.
fn is_request_input(ctx: &Ctx, path: &JsonPointer) -> bool {
    if !ctx.opts.fix.contains(Fix::DefaultNonNullableScope) {
        return path.contains("parameters") || path.contains("requestBody") || path.contains("requestBodies");
    }
    let segs = parse_ref(path);
    let seg = |i: usize| segs.get(i).map(AsRef::as_ref);
    let rest = match (seg(0), seg(1)) {
        (Some("components"), Some("parameters" | "requestBodies")) => return true,
        (Some("components"), Some("pathItems")) => 3,
        (Some("paths" | "webhooks"), _) => 2,
        _ => return false,
    };
    let is_method = |s: Option<&str>| s.is_some_and(|s| s.parse::<HttpMethod>().is_ok());
    seg(rest) == Some("parameters")
        || (is_method(seg(rest)) && matches!(seg(rest + 1), Some("parameters" | "requestBody")))
}

fn transform_property(
    ctx: &mut Ctx,
    obj: &Map,
    path: &JsonPointer,
    k: &str,
    v: &Value,
    core: &mut Vec<Member>,
) -> Result<()> {
    let (r, read_only, write_only, has_default) = match v {
        Value::Object(m) => (
            m.get("$ref").filter(|r| r.truthy()).map(Value::js_string),
            truthy(m.get("readOnly")),
            truthy(m.get("writeOnly")),
            m.contains_key("default"),
        ),
        Value::Bool(_) => (None, false, false, false),
        other => {
            let kind = match other {
                Value::Array(_) => "Array",
                Value::Number(_) => "number",
                Value::Null => "object",
                _ => "string",
            };
            return Err(Error::at(
                path,
                format!("{path}: invalid property {k}. Expected Schema Object or boolean, got {kind}"),
            ));
        }
    };

    if ctx.opts.exclude_deprecated {
        let deprecated = match &r {
            Some(r) => ctx.resolve(r).is_some_and(|s| truthy(s.get("deprecated"))),
            None => truthy(v.get("deprecated")),
        };
        if deprecated {
            return Ok(());
        }
    }

    let mut required = required_includes(obj.get("required"), k)
        || (!obj.contains_key("required") && ctx.opts.properties_required_by_default)
        || (has_default && ctx.opts.default_non_nullable && !is_request_input(ctx, path));
    let prop_path = create_ref(&[path.as_str(), k]);
    let mut ty = match &r {
        Some(r) => schema_ref(ctx, r, path)?,
        None => transform_schema(ctx, v, &prop_path, false)?,
    };
    if let Some(result) = ctx.transform_hook(v, path)? {
        ty = result.ty;
        required &= !result.question_token;
    }
    let ty = read_write(ctx, ty, read_only, write_only);
    let is_readonly = ctx.opts.immutable || (!ctx.opts.read_write_markers && read_only);
    let member = Member::prop(property_index(k), !required, is_readonly, ty);
    core.push(ctx.property_hook(member, v, &prop_path)?);
    Ok(())
}

#[expect(
    clippy::while_float,
    clippy::suboptimal_flops,
    reason = "counts and rounds like openapi-typescript's JavaScript"
)]
fn transform_array(ctx: &mut Ctx, obj: &Map, path: &JsonPointer) -> Result<Ty> {
    let mut item_ty = UNKNOWN;
    let mut built_here = false;
    let items = obj.get("items");
    if truthy(obj.get("prefixItems")) || matches!(items, Some(Value::Array(_))) {
        let prefix = match obj.get("prefixItems") {
            Some(p) if !matches!(p, Value::Null) => p,
            _ => items.unwrap_or(&Value::Null),
        };
        let Value::Array(prefix) = prefix else {
            return Err(Error::at(path, "prefixItems must be an array"));
        };
        let mut elements = Vec::with_capacity(prefix.len());
        for item in prefix {
            elements.push(transform_schema(ctx, item, path, false)?);
        }
        let closed = matches!(items, Some(Value::Bool(false)))
            || matches!(obj.get("unevaluatedItems"), Some(Value::Bool(false)))
            || matches!(obj.get("maxItems"), Some(Value::Number(n)) if *n <= f64::from(u32::try_from(prefix.len()).unwrap_or(u32::MAX)));
        if ctx.opts.fix.contains(Fix::PrefixItems) && truthy(obj.get("prefixItems")) && !closed {
            let rest = match items {
                Some(items @ Value::Object(_)) => transform_schema(ctx, items, path, false)?,
                _ => UNKNOWN,
            };
            elements.push(Ty::Rest(Box::new(Ty::Array(Box::new(rest)))));
        }
        item_ty = Ty::Tuple(elements);
        built_here = true;
    } else if let Some(items) = items.filter(|i| i.truthy()) {
        let inner = transform_schema(ctx, items, path, false)?;
        built_here = items.get("type").and_then(Value::as_str) == Some("array");
        item_ty = if built_here { Ty::Array(Box::new(inner)) } else { inner };
    }

    let min = match obj.get("minItems") {
        Some(Value::Number(n)) if *n >= 0.0 => *n,
        _ => 0.0,
    };
    let max = match obj.get("maxItems") {
        Some(Value::Number(n)) if *n >= 0.0 && min <= *n => Some(*n),
        _ => None,
    };
    let estimate = match max {
        None => min,
        Some(max) => (max * (max + 1.0) - min * (min - 1.0)) / 2.0,
    };
    if ctx.opts.array_length && (min != 0.0 || max.is_some()) && estimate < 30.0 {
        let repeat = |n: f64| -> Vec<Ty> {
            let mut out = Vec::new();
            let mut i = 0.0;
            while i < n {
                out.push(item_ty.clone());
                i += 1.0;
            }
            out
        };
        if Some(min) == max {
            return Ok(union(vec![Ty::Tuple(repeat(min))]));
        }
        if matches!(obj.get("maxItems"), Some(Value::Number(n)) if *n > 0.0) {
            let mut members = Vec::new();
            let mut i = 0.0;
            while i <= max.unwrap_or(0.0) - min {
                let mut elements = Vec::new();
                let mut j = if ctx.opts.fix.contains(Fix::ArrayLength) { 0.0 } else { min };
                while j < i + min {
                    elements.push(item_ty.clone());
                    j += 1.0;
                }
                members.push(Ty::Tuple(elements));
                i += 1.0;
            }
            return Ok(union(members));
        }
        let mut elements = repeat(min);
        elements.push(Ty::Rest(Box::new(Ty::Array(Box::new(item_ty.clone())))));
        return Ok(Ty::Tuple(elements));
    }

    let keep = if ctx.opts.fix.contains(Fix::ConstItems) {
        built_here
    } else {
        matches!(item_ty, Ty::Tuple(_) | Ty::Array(_))
    };
    let final_ty = if keep { item_ty } else { Ty::Array(Box::new(item_ty)) };
    Ok(if ctx.opts.immutable { Ty::Readonly(Box::new(final_ty)) } else { final_ty })
}

/// `createDiscriminatorProperty`.
pub fn discriminator_property(discriminator: &Value, path: &JsonPointer, readonly: bool) -> Member {
    let mut value = path.segments().last();
    if let Some(Value::Object(mapping)) = discriminator.get("mapping") {
        let matched = mapping.iter().find(|(_, v)| {
            let Some(v) = v.as_str() else { return false };
            if v.starts_with('#') { parse_ref(v).pop() == value } else { Some(v) == value.as_deref() }
        });
        if let Some((k, _)) = matched {
            value = Some(Cow::Borrowed(k));
        }
    }
    let name = discriminator.get("propertyName").map_or_else(|| "undefined".into(), Value::js_string);
    let ty = value.map_or(UNKNOWN, |v| literal(Some(&Value::String(v.into_owned()))));
    Member::prop(property_index(&name), false, readonly, ty)
}

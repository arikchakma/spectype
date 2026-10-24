//! `scanDiscriminators`: collects discriminator objects and patches mapped schemas with the
//! discriminator enum property, mutating the document exactly as openapi-typescript does.

use rustc_hash::FxHashSet;

use spectype_document::pointer::{Resolved, create_ref, get_path_mut, resolve_ref, resolve_ref_path};
use spectype_document::value::{JsMap, Map, Value};

use crate::{Fix, Fixes};

/// What `scanDiscriminators` found: discriminator objects by the pointer of the schema holding
/// them, and the `$ref`s of the schemas it patched.
#[derive(Debug, Default)]
pub struct Discriminators {
    pub(crate) objects: Map,
    pub(crate) refs_handled: FxHashSet<String>,
    refs_handled_list: Vec<String>,
}

impl Discriminators {
    /// `ctx.discriminators` as JSON: `{ objects, refsHandled }` in upstream's order.
    #[must_use]
    pub fn to_json(&self) -> String {
        let refs = self.refs_handled_list.iter().map(|r| Value::String(r.clone())).collect();
        let mut m = Map::new();
        m.insert("objects".into(), Value::Object(self.objects.clone()));
        m.insert("refsHandled".into(), Value::Array(refs));
        Value::Object(m).json_stringify(0)
    }

    fn handled(&mut self, r: &str) {
        self.refs_handled.insert(r.to_owned());
        self.refs_handled_list.push(r.to_owned());
    }
}

enum Seg<'a> {
    Key(&'a str),
    Idx(usize),
}

/// The paths of objects with a discriminator `propertyName`, and of objects with an `allOf` array.
#[derive(Default)]
struct Found {
    discriminators: Vec<Vec<String>>,
    all_of: Vec<Vec<String>>,
}

fn walk<'a>(node: &'a Value, path: &mut Vec<Seg<'a>>, out: &mut Found) {
    match node {
        Value::Array(a) => {
            for (i, v) in a.iter().enumerate() {
                if v.is_js_object() {
                    path.push(Seg::Idx(i));
                    walk(v, path, out);
                    path.pop();
                }
            }
        }
        Value::Object(m) => {
            let has_discriminator =
                m.get("discriminator").and_then(|d| d.get("propertyName")).is_some_and(Value::truthy);
            let has_all_of = matches!(m.get("allOf"), Some(Value::Array(_)));
            if has_discriminator || has_all_of {
                let segments: Vec<String> = path
                    .iter()
                    .map(|s| match s {
                        Seg::Key(k) => (*k).to_owned(),
                        Seg::Idx(i) => i.to_string(),
                    })
                    .collect();
                if has_discriminator {
                    out.discriminators.push(segments.clone());
                }
                if has_all_of {
                    out.all_of.push(segments);
                }
            }
            for (k, v) in m.iter() {
                if v.is_js_object() {
                    path.push(Seg::Key(k));
                    walk(v, path, out);
                    path.pop();
                }
            }
        }
        _ => {}
    }
}

fn node_at<'a>(root: &'a Value, path: &[String]) -> Option<&'a Value> {
    let mut node = root;
    for key in path {
        node = node.index(key)?;
    }
    Some(node)
}

fn discriminator_enum(values: &[String], prev: Option<&Value>) -> Value {
    let description = match prev.and_then(|p| p.get("description")).filter(|d| d.truthy()) {
        Some(d) => format!("{} (enum property replaced by openapi-typescript)", d.js_string()),
        None => "discriminator enum property added by openapi-typescript".into(),
    };
    let mut m = Map::new();
    m.insert("type".into(), Value::String("string".into()));
    m.insert("enum".into(), Value::Array(values.iter().cloned().map(Value::String).collect()));
    m.insert("description".into(), Value::String(description));
    Value::Object(m)
}

fn property_name(discriminator: &Value) -> String {
    discriminator.get("propertyName").map_or_else(|| "undefined".into(), Value::js_string)
}

/// `patchDiscriminatorEnum`.
fn patch(
    root: &mut Value,
    r: &str,
    values: &[String],
    discriminator: &Value,
    discriminator_ref: &str,
    warnings: &mut Vec<String>,
) -> bool {
    let prop = property_name(discriminator);
    let target = match resolve_ref(root, r) {
        Resolved::Found(_) => resolve_ref_path(root, r),
        Resolved::Missing => {
            warnings.push(format!("Could not resolve $ref \"{r}\""));
            None
        }
        Resolved::Circular => {
            warnings.push(format!("Could not resolve circular $ref \"{r}\""));
            None
        }
    };
    if let Some(Value::Object(schema)) = target.and_then(|p| get_path_mut(root, &p)) {
        if let Some(Value::Array(all_of)) = schema.get_mut("allOf") {
            let mut item = Map::new();
            item.insert("type".into(), Value::String("object".into()));
            item.insert("required".into(), Value::Array(vec![Value::String(prop.clone())]));
            let mut props = Map::new();
            props.insert(prop, discriminator_enum(values, None));
            item.insert("properties".into(), Value::Object(props));
            all_of.push(Value::Object(item));
            return true;
        }
        if schema.get("type") == Some(&Value::String("object".into())) {
            if !schema.get("properties").is_some_and(Value::truthy) {
                schema.insert("properties".into(), Value::Object(Map::new()));
            }
            match schema.get_mut("required") {
                Some(Value::Array(req)) => {
                    if !req.iter().any(|v| v.as_str() == Some(&prop)) {
                        req.push(Value::String(prop.clone()));
                    }
                }
                Some(v) if v.truthy() => {}
                _ => {
                    schema.insert("required".into(), Value::Array(vec![Value::String(prop.clone())]));
                }
            }
            if let Some(Value::Object(props)) = schema.get_mut("properties") {
                let replaced = discriminator_enum(values, props.get(&prop));
                props.insert(prop, replaced);
            }
            return true;
        }
    }
    warnings.push(format!(
        "Discriminator mapping has an invalid schema (neither an object schema nor an allOf array): {r} => {} (Discriminator: {discriminator_ref})",
        values.join(", ")
    ));
    false
}

/// Whether `r` is a variant of the discriminator at `owner`: always when `owner` has no `oneOf` or
/// `anyOf`, otherwise only when one of them or the mapping names it.
fn is_variant(root: &Value, owner: &str, r: &str) -> bool {
    let Resolved::Found(owner) = resolve_ref(root, owner) else { return true };
    let unions: Vec<&Value> =
        ["oneOf", "anyOf"].iter().filter_map(|k| owner.get(k)).filter_map(Value::as_array).flatten().collect();
    if unions.is_empty() {
        return true;
    }
    let names = |v: &Value| v.as_str() == Some(r);
    unions.iter().any(|v| v.get("$ref").is_some_and(names))
        || owner
            .get("discriminator")
            .and_then(|d| d.get("mapping"))
            .and_then(Value::as_object)
            .is_some_and(|m| m.values().any(names))
}

#[derive(Default)]
struct Mapping {
    inferred: Option<String>,
    defined: Option<Vec<String>>,
}

/// Scans and patches `root`, returning what the transforms need and the warnings upstream logs.
pub fn scan(root: &mut Value, fixes: Fixes) -> (Discriminators, Vec<String>) {
    let mut d = Discriminators::default();
    let mut warnings = Vec::new();

    let mut found = Found::default();
    walk(root, &mut Vec::new(), &mut found);
    for path in found.discriminators {
        let Some(obj) = node_at(root, &path) else { continue };
        let Some(discriminator) = obj.get("discriminator").cloned() else { continue };
        let r = create_ref(&path).into_string();
        d.objects.insert(r.clone(), discriminator.clone());
        let Some(Value::Array(one_of)) = obj.get("oneOf") else { continue };

        let mut mapping: JsMap<Mapping> = JsMap::new();
        for item in one_of {
            if let Some(item_ref) = item.get("$ref") {
                let item_ref = item_ref.js_string();
                let value = item_ref.rsplit('/').next().unwrap_or_default().to_owned();
                if !value.is_empty() {
                    match mapping.get_mut(&item_ref) {
                        Some(m) => m.inferred = Some(value),
                        None => mapping.insert(item_ref, Mapping { inferred: Some(value), defined: None }),
                    }
                }
            }
        }
        if let Some(Value::Object(defined)) = discriminator.get("mapping") {
            for (mapped_value, mapped_ref) in defined.iter() {
                if !mapped_ref.truthy() {
                    continue;
                }
                let mapped_ref = mapped_ref.js_string();
                if mapping.get(&mapped_ref).is_none_or(|m| m.defined.is_none()) {
                    mapping.insert(mapped_ref.clone(), Mapping { inferred: None, defined: Some(Vec::new()) });
                }
                if let Some(Mapping { defined: Some(list), .. }) = mapping.get_mut(&mapped_ref) {
                    list.push(mapped_value.clone());
                }
            }
        }
        for (mapped_ref, m) in mapping.iter() {
            if d.refs_handled.contains(mapped_ref) {
                continue;
            }
            let values = match (&m.defined, &m.inferred) {
                (Some(defined), _) => defined.clone(),
                (None, Some(inferred)) => vec![inferred.clone()],
                (None, None) => continue,
            };
            if patch(root, mapped_ref, &values, &discriminator, &r, &mut warnings) {
                d.handled(mapped_ref);
            }
        }
    }

    for path in found.all_of {
        let r = create_ref(&path).into_string();
        let mut i = 0;
        while let Some(item) =
            node_at(root, &path).and_then(|n| n.get("allOf")).and_then(|a| a.as_array()).and_then(|a| a.get(i))
        {
            i += 1;
            let item_ref = item.get("$ref").map(Value::js_string);
            let own = item.get("discriminator").filter(|x| x.get("propertyName").is_some_and(Value::truthy)).cloned();
            if let Some(item_ref) = item_ref {
                let Some(discriminator) = d.objects.get(&item_ref).cloned() else { break };
                if let Some(Value::Object(mapping)) = discriminator.get("mapping") {
                    let values: Vec<String> = mapping
                        .iter()
                        .filter(|(_, v)| v.as_str() == Some(r.as_str()))
                        .map(|(k, _)| k.clone())
                        .collect();
                    if !values.is_empty() && patch(root, &r, &values, &discriminator, &item_ref, &mut warnings) {
                        d.handled(&r);
                    }
                }
                if fixes.contains(Fix::DiscriminatorOneOfBase) && !is_variant(root, &item_ref, &r) {
                    continue;
                }
                d.objects.insert(r.clone(), discriminator);
            } else if let Some(disc) = own {
                d.objects.insert(r.clone(), disc);
            }
        }
    }
    (d, warnings)
}

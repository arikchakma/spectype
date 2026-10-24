//! Redocly's `struct` rule: every node against its type in the OAS3 type tree. The scalar
//! property specs (`{ type: 'string' }`, `{ enum: [...] }`, ...) are listed here because the
//! bundler's copy of the tree only needs to know which keys are walked.

use super::{Cx, Rule, Target, V, js_string, matches_type, oas_type_of};
use crate::bundle::Node;
use crate::bundle::types::{Pt, Ty, type_def};
use spectype_document::value::{Value, js_string_to_number};

pub(super) struct Struct;

/// The Redocly type name that struct messages print.
pub(super) fn type_name(ty: Ty) -> String {
    format!("{ty:?}")
}

#[derive(Default, Clone, Copy)]
struct Spec {
    ty: Option<&'static str>,
    enum_: Option<&'static [&'static str]>,
    items_ty: Option<&'static str>,
    items_enum: Option<&'static [&'static str]>,
    minimum: bool,
}

const fn of(ty: &'static str) -> Spec {
    Spec { ty: Some(ty), enum_: None, items_ty: None, items_enum: None, minimum: false }
}

const fn one_of(values: &'static [&'static str]) -> Spec {
    Spec { ty: None, enum_: Some(values), items_ty: None, items_enum: None, minimum: false }
}

const STR: Spec = of("string");
const BOOL: Spec = of("boolean");
const NUM: Spec = of("number");
const ARR: Spec = of("array");
const OBJ: Spec = of("object");
const STRINGS: Spec =
    Spec { ty: Some("array"), enum_: None, items_ty: Some("string"), items_enum: None, minimum: false };
const COUNT: Spec = Spec { ty: Some("integer"), enum_: None, items_ty: None, items_enum: None, minimum: true };
const NONE: Spec = Spec { ty: None, enum_: None, items_ty: None, items_enum: None, minimum: false };

const IN: &[&str] = &["query", "header", "path", "cookie"];
const STYLE: &[&str] = &["form", "simple", "label", "matrix", "spaceDelimited", "pipeDelimited", "deepObject"];
const TYPES_30: &[&str] = &["object", "array", "string", "number", "integer", "boolean"];
const TYPES_31: &[&str] = &["object", "array", "string", "number", "integer", "boolean", "null"];

/// The unnamed property spec for `key` in `ty`, for keys the type tree treats as scalars. Arms
/// follow Redocly's per-type tables rather than being merged by result.
#[expect(clippy::match_same_arms, reason = "arms mirror Redocly's per-type tables")]
fn spec(ty: Ty, v31: bool, key: &str, value: &Node) -> Spec {
    match (ty, key) {
        (Ty::Root, "x-ignoredHeaderParameters") | (Ty::TagGroup | Ty::Operation, "tags") => STRINGS,
        (Ty::Root, "jsonSchemaDialect") => STR,
        (Ty::Tag, "x-traitTag") => BOOL,
        (
            Ty::Tag | Ty::TagGroup | Ty::Contact | Ty::License | Ty::Xml,
            "name" | "description" | "x-displayName" | "url" | "email" | "identifier" | "namespace" | "prefix",
        ) => STR,
        (Ty::ExternalDocs | Ty::Server, "description" | "url") => STR,
        (Ty::ServerVariable, "enum") => STRINGS,
        (Ty::ServerVariable, "default" | "description") => STR,
        (Ty::SecurityRequirement | Ty::DependentRequired, _) => STRINGS,
        (Ty::Info, "title" | "version" | "description" | "termsOfService" | "summary") => STR,
        (Ty::Logo, "url" | "altText" | "backgroundColor" | "href") => STR,
        (Ty::PathItem, "$ref" | "summary" | "description") => STR,
        (Ty::Parameter, "in") => one_of(IN),
        (Ty::Parameter | Ty::Header | Ty::Encoding, "style") => one_of(STYLE),
        (Ty::Parameter, "name") => STR,
        (
            Ty::Parameter | Ty::Header | Ty::RequestBody | Ty::Encoding,
            "required" | "deprecated" | "allowEmptyValue" | "explode" | "allowReserved",
        ) => BOOL,
        (Ty::Parameter | Ty::Header | Ty::RequestBody | Ty::Response | Ty::Link, "description") => STR,
        (Ty::Operation, "summary" | "description" | "operationId") => STR,
        (Ty::Operation, "deprecated" | "x-hideTryItPanel") => BOOL,
        (Ty::XCodeSample, "lang" | "label" | "source") => STR,
        (Ty::Example, "summary" | "description" | "externalValue") => STR,
        (Ty::Encoding, "contentType") => STR,
        (Ty::Response, "x-summary") => STR,
        (Ty::Link, "operationRef" | "operationId") => STR,
        (Ty::Xml, "attribute" | "wrapped") => BOOL,
        (Ty::DiscriminatorMapping, _) => STR,
        (Ty::Discriminator, "propertyName") => STR,
        (Ty::ImplicitFlow | Ty::PasswordFlow | Ty::ClientCredentials | Ty::AuthorizationCode, "scopes") => OBJ,
        (
            Ty::ImplicitFlow | Ty::PasswordFlow | Ty::ClientCredentials | Ty::AuthorizationCode,
            "refreshUrl" | "authorizationUrl" | "tokenUrl",
        ) => STR,
        (Ty::SecurityScheme, "type") if v31 => one_of(&["apiKey", "http", "oauth2", "openIdConnect", "mutualTLS"]),
        (Ty::SecurityScheme, "type") => one_of(&["apiKey", "http", "oauth2", "openIdConnect"]),
        (Ty::SecurityScheme, "in") => one_of(&["query", "header", "cookie"]),
        (
            Ty::SecurityScheme,
            "description" | "name" | "scheme" | "bearerFormat" | "openIdConnectUrl" | "x-defaultClientId",
        ) => STR,
        (Ty::XUsePkce, _) => BOOL,
        (Ty::Schema, _) => schema_spec(v31, key, value),
        _ => NONE,
    }
}

fn schema_spec(v31: bool, key: &str, value: &Node) -> Spec {
    match key {
        "title" | "pattern" | "description" | "format" | "x-additionalPropertiesName" => STR,
        "$id" | "$anchor" | "id" | "$schema" | "$vocabulary" | "summary" | "contentEncoding" | "contentMediaType"
        | "$comment" | "$dynamicAnchor" | "$dynamicRef"
            if v31 =>
        {
            STR
        }
        "multipleOf" => Spec { minimum: true, ..NUM },
        "maximum" | "minimum" => NUM,
        "exclusiveMaximum" | "exclusiveMinimum" => {
            if v31 {
                NUM
            } else {
                BOOL
            }
        }
        "maxLength" | "minLength" | "maxItems" | "minItems" | "maxProperties" | "minProperties" => COUNT,
        "minContains" | "maxContains" if v31 => COUNT,
        "uniqueItems" | "readOnly" | "writeOnly" | "deprecated" | "x-explicitMappingOnly" => BOOL,
        "nullable" if !v31 => BOOL,
        "required" | "x-tags" => STRINGS,
        "enum" => ARR,
        "examples" if v31 => ARR,
        "type" if v31 && matches!(value, Node::Arr(_)) => {
            Spec { ty: Some("array"), enum_: None, items_ty: None, items_enum: Some(TYPES_31), minimum: false }
        }
        "type" if v31 => one_of(TYPES_31),
        "type" => one_of(TYPES_30),
        _ => NONE,
    }
}

fn required(ty: Ty, v31: bool, node: &Node) -> &'static [&'static str] {
    match ty {
        Ty::Root if v31 => &["openapi", "info"],
        Ty::Root => &["openapi", "paths", "info"],
        Ty::Tag | Ty::License => &["name"],
        Ty::ExternalDocs | Ty::Server => &["url"],
        Ty::ServerVariable => &["default"],
        Ty::Info => &["title", "version"],
        Ty::Parameter => &["name", "in"],
        Ty::Operation if !v31 => &["responses"],
        Ty::RequestBody => &["content"],
        Ty::Response => &["description"],
        Ty::Discriminator => &["propertyName"],
        Ty::ImplicitFlow => &["authorizationUrl", "scopes"],
        Ty::PasswordFlow | Ty::ClientCredentials => &["tokenUrl", "scopes"],
        Ty::AuthorizationCode => &["authorizationUrl", "tokenUrl", "scopes"],
        Ty::SecurityScheme => match scheme_type(node).as_deref() {
            Some("apiKey") => &["type", "name", "in"],
            Some("http") => &["type", "scheme"],
            Some("oauth2") => &["type", "flows"],
            Some("openIdConnect") => &["type", "openIdConnectUrl"],
            _ => &["type"],
        },
        _ => &[],
    }
}

fn required_one_of(ty: Ty, v31: bool) -> Option<&'static [&'static str]> {
    match ty {
        Ty::Root if v31 => Some(&["paths", "components", "webhooks"]),
        Ty::Parameter | Ty::Header => Some(&["schema", "content"]),
        _ => None,
    }
}

fn scheme_type(node: &Node) -> Option<String> {
    node.get("type").and_then(|t| t.as_str())
}

fn allowed(ty: Ty, v31: bool, node: &Node) -> Option<&'static [&'static str]> {
    if ty != Ty::SecurityScheme {
        return None;
    }
    let all_flows: &'static [&'static str] =
        &["type", "flows", "authorizationUrl", "refreshUrl", "tokenUrl", "description", "scopes"];
    Some(match scheme_type(node).as_deref() {
        Some("apiKey") => &["type", "name", "in", "description"],
        Some("http") => &["type", "scheme", "bearerFormat", "description"],
        Some("oauth2") if !v31 => &["type", "flows", "description"],
        Some("oauth2") => match node.get("flows").and_then(|f| f.as_str()).as_deref() {
            Some("implicit") => &["type", "flows", "authorizationUrl", "refreshUrl", "description", "scopes"],
            Some("password" | "clientCredentials") => {
                &["type", "flows", "tokenUrl", "refreshUrl", "description", "scopes"]
            }
            _ => all_flows,
        },
        Some("openIdConnect") => &["type", "openIdConnectUrl", "description"],
        _ => &["type", "description"],
    })
}

/// `Number(value)`, as `>` coerces it.
fn to_number(n: &Node) -> f64 {
    match n {
        Node::Prim(Value::Number(x)) => *x,
        Node::Prim(Value::Bool(b)) => f64::from(u8::from(*b)),
        Node::Prim(Value::Null) => 0.0,
        Node::Prim(_) | Node::Arr(_) => js_string_to_number(&js_string(n)),
        Node::Obj(_) => f64::NAN,
    }
}

fn check_enum(values: &[&str], value: Option<&Node>, prop: &str, at: &super::Loc, cx: &mut Cx<'_>) {
    if value.and_then(Node::as_str).is_some_and(|s| values.contains(&s.as_str())) {
        return;
    }
    let list = values.iter().map(|v| format!("\"{v}\"")).collect::<Vec<_>>().join(", ");
    cx.report_at(&format!("`{prop}` can be one of the following only: {list}."), at);
}

impl Rule for Struct {
    fn visitors(&self) -> &'static [V] {
        const VS: &[V] = &[V::on(Target::Any, 0)];
        VS
    }

    fn visit(&mut self, _: u8, node: &Node, cx: &mut Cx<'_>) {
        let ty = cx.ty;
        let td = type_def(ty, cx.v31);
        let node_type = oas_type_of(node);
        if td.items.is_some() {
            if node_type != "array" {
                cx.report(&format!("Expected type `{}` (array) but got `{node_type}`", type_name(ty)));
                cx.ignore_next_visitors();
            }
            return;
        }
        let Node::Obj(map) = node else {
            if ty != Ty::SpecExtension {
                cx.report(&format!("Expected type `{}` (object) but got `{node_type}`", type_name(ty)));
            }
            cx.ignore_next_visitors();
            return;
        };
        let keys: Vec<String> = map.borrow().keys().cloned().collect();
        for field in required(ty, cx.v31, node) {
            if !keys.iter().any(|k| k == field) {
                cx.report(&format!("The field `{field}` must be present on this level."));
            }
        }
        if let Some(allowed) = allowed(ty, cx.v31, node) {
            for k in &keys {
                if allowed.contains(&k.as_str())
                    || (td.ext && k.starts_with("x-"))
                    || !td.props.iter().any(|(p, _)| p == k)
                {
                    continue;
                }
                let at = cx.location.child(k);
                cx.report_at(&format!("The field `{k}` is not allowed here."), &at);
            }
        }
        if let Some(one_of) = required_one_of(ty, cx.v31)
            && !one_of.iter().any(|f| keys.iter().any(|k| k == f))
        {
            cx.report(&format!("Must contain at least one of the following fields: {}.", one_of.join(", ")));
        }
        for k in &keys {
            let Some(raw) = map.borrow().get(k).cloned() else { continue };
            let at = cx.location.child(k);
            if k == "constructor" {
                constructor_quirk(&raw, &at, cx);
                continue;
            }
            if OBJECT_PROTO.contains(&k.as_str()) {
                continue;
            }
            let spec = match td.prop(k, &raw) {
                Some(Pt::Ty(Ty::Scalar)) => spec(ty, cx.v31, k, &raw),
                Some(Pt::Direct) => STR,
                Some(Pt::Ty(_) | Pt::Null) => continue,
                None => {
                    if !k.starts_with("x-") {
                        cx.report_at(&format!("Property `{k}` is not expected here."), &at);
                    }
                    continue;
                }
            };
            let raw_type = oas_type_of(&raw);
            let value = if raw.is_ref() { cx.resolve(&raw).map(|(n, _)| n) } else { Some(raw.clone()) };
            if let (Some(items), Some(Node::Arr(a))) = (spec.items_enum, &value) {
                let items_list: Vec<Node> = a.borrow().clone();
                for (i, item) in items_list.iter().enumerate() {
                    check_enum(items, Some(item), k, &at.index(i), cx);
                }
            }
            if let Some(values) = spec.enum_ {
                check_enum(values, value.as_ref(), k, &at, cx);
            } else if let Some(t) = spec.ty
                && !value.as_ref().is_some_and(|v| matches_type(v, t, false))
            {
                cx.report_at(&format!("Expected type `{t}` but got `{raw_type}`."), &at);
                cx.ignore_next_visitors();
            } else if let (true, Some(item_ty), Some(Node::Arr(a))) = (raw_type == "array", spec.items_ty, &value) {
                let items_list: Vec<Node> = a.borrow().clone();
                for (i, item) in items_list.iter().enumerate() {
                    if !matches_type(item, item_ty, false) {
                        let msg = format!("Expected type `{item_ty}` but got `{}`.", oas_type_of(item));
                        cx.report_at(&msg, &at.index(i));
                    }
                }
            }
            if spec.minimum && 0.0 > to_number(&raw) {
                cx.report_at(&format!("The value of the {k} field must be greater than or equal to 0"), &at);
            }
            if k == "nullable" && !node.get("type").is_some_and(|t| t.truthy()) {
                cx.report_at("The `type` field must be defined when the `nullable` field is used.", &at);
            }
        }
    }
}

const OBJECT_PROTO: [&str; 11] = [
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

/// A `constructor` key finds `Object` on the type's `properties`, so the property spec becomes
/// `Object(value)`: the value checked against its own `enum` and `type`.
fn constructor_quirk(value: &Node, at: &super::Loc, cx: &mut Cx<'_>) {
    let Node::Obj(_) = value else { return };
    if value.ref_str().is_some() || value.get("name").and_then(|n| n.as_str()).is_some() {
        return;
    }
    if let Some(Node::Arr(values)) = value.get("enum").filter(Node::truthy) {
        let list = values.borrow().iter().map(|v| format!("\"{}\"", js_string(v))).collect::<Vec<_>>().join(", ");
        cx.report_at(&format!("`constructor` can be one of the following only: {list}."), at);
    } else if let Some(t) = value.get("type").filter(Node::truthy)
        && !matches_type(value, &js_string(&t), false)
    {
        cx.report_at(&format!("Expected type `{}` but got `object`.", js_string(&t)), at);
        cx.ignore_next_visitors();
    }
}

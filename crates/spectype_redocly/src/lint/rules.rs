//! Redocly's built-in OAS3 rules that `minimal` and `recommended` enable, with their messages and
//! report locations copied from `lib/rules/{common,oas3}`.

use rustc_hash::{FxHashMap, FxHashSet};

use super::{Cx, Loc, Rule, Target, V, is_plain_object, js_string, matches_type, oas_type_of, str_at, truthy_at};
use crate::bundle::Node;
use crate::bundle::types::Ty;
use spectype_document::oas::ComponentKind;
use spectype_document::value::{Map, Value};

/// The rule order of `rules/oas3/index.js`, which is the order visitors run in.
pub(super) const ORDER: [&str; 59] = [
    "struct",
    "info-contact",
    "info-license",
    "info-license-url",
    "info-license-strict",
    "operation-2xx-response",
    "operation-4xx-response",
    "operation-4xx-problem-details-rfc7807",
    "assertions",
    "operation-operationId-unique",
    "operation-parameters-unique",
    "operation-tag-defined",
    "no-example-value-and-externalValue",
    "no-enum-type-mismatch",
    "no-path-trailing-slash",
    "no-empty-servers",
    "path-declaration-must-exist",
    "operation-operationId-url-safe",
    "operation-operationId",
    "operation-summary",
    "tags-alphabetical",
    "no-server-example.com",
    "no-server-trailing-slash",
    "tag-description",
    "operation-description",
    "no-unused-components",
    "path-not-include-query",
    "path-parameters-defined",
    "path-params-defined",
    "parameter-description",
    "operation-singular-tag",
    "security-defined",
    "no-unresolved-refs",
    "paths-kebab-case",
    "boolean-parameter-prefixes",
    "path-http-verbs-order",
    "no-invalid-media-type-examples",
    "no-identical-paths",
    "no-ambiguous-paths",
    "no-undefined-server-variable",
    "no-server-variables-empty-enum",
    "no-http-verbs-in-paths",
    "path-excludes-patterns",
    "request-mime-type",
    "response-mime-type",
    "path-segment-plural",
    "no-invalid-schema-examples",
    "no-invalid-parameter-examples",
    "response-contains-header",
    "response-contains-property",
    "scalar-property-missing-example",
    "spec-components-invalid-map-name",
    "required-string-property-missing-min-length",
    "spec-strict-refs",
    "component-name-unique",
    "array-parameter-serialization",
    "no-required-schema-properties-undefined",
    "no-schema-type-mismatch",
    "spec",
];

pub(super) fn is_supported(id: &str) -> bool {
    make(id, &Map::new()).is_some()
}

pub(super) fn make(id: &str, opts: &Map) -> Option<Box<dyn Rule>> {
    let flag = |k: &str| opts.get(k).is_some_and(Value::truthy);
    Some(match id {
        "struct" => Box::new(super::structure::Struct),
        "info-license" => Box::new(Simple(Kind::InfoLicense)),
        "info-license-strict" => Box::new(InfoLicenseStrict),
        "operation-2xx-response" => Box::new(ResponseCodes { range: '2', webhooks: flag("validateWebhooks") }),
        "operation-4xx-response" => Box::new(ResponseCodes { range: '4', webhooks: flag("validateWebhooks") }),
        "operation-operationId-unique" => Box::new(OperationIdUnique(FxHashSet::default())),
        "operation-parameters-unique" => Box::new(ParametersUnique::default()),
        "no-example-value-and-externalValue" => Box::new(Simple(Kind::ExampleValue)),
        "no-enum-type-mismatch" => Box::new(Simple(Kind::EnumTypeMismatch)),
        "no-path-trailing-slash" => Box::new(Simple(Kind::PathTrailingSlash)),
        "no-empty-servers" => Box::new(Simple(Kind::EmptyServers)),
        "path-declaration-must-exist" => Box::new(Simple(Kind::PathDeclaration)),
        "operation-operationId-url-safe" => Box::new(Simple(Kind::OperationIdUrlSafe)),
        "operation-operationId" => Box::new(OperationOperationId),
        "operation-summary" => Box::new(Simple(Kind::OperationSummary)),
        "no-server-example.com" => Box::new(Simple(Kind::ServerExample)),
        "no-server-trailing-slash" => Box::new(Simple(Kind::ServerTrailingSlash)),
        "tag-description" => Box::new(Simple(Kind::TagDescription)),
        "no-unused-components" => Box::new(UnusedComponents::default()),
        "path-not-include-query" => Box::new(PathNotIncludeQuery),
        "path-parameters-defined" | "path-params-defined" => Box::new(PathParamsDefined::default()),
        "security-defined" => Box::new(SecurityDefined::new(opts.get("exceptions"))),
        "no-unresolved-refs" => Box::new(UnresolvedRefs),
        "no-invalid-media-type-examples" => {
            let allow = opts.get("allowAdditionalProperties").is_some_and(Value::truthy);
            Box::new(super::ajv::MediaTypeExamples { allow_additional: allow, compiled: FxHashSet::default() })
        }
        "no-identical-paths" => Box::new(Simple(Kind::IdenticalPaths)),
        "no-ambiguous-paths" => Box::new(Simple(Kind::AmbiguousPaths)),
        "no-undefined-server-variable" => Box::new(Simple(Kind::UndefinedServerVariable)),
        "no-server-variables-empty-enum" => Box::new(Simple(Kind::ServerVariablesEmptyEnum)),
        "spec-components-invalid-map-name" => Box::new(ComponentsMapName),
        "no-schema-type-mismatch" => Box::new(Simple(Kind::SchemaTypeMismatch)),
        _ => return None,
    })
}

/// Rules with a single top-level visitor and no state.
#[derive(Clone, Copy)]
enum Kind {
    InfoLicense,
    ExampleValue,
    EnumTypeMismatch,
    PathTrailingSlash,
    EmptyServers,
    PathDeclaration,
    OperationIdUrlSafe,
    OperationSummary,
    ServerExample,
    ServerTrailingSlash,
    TagDescription,
    IdenticalPaths,
    AmbiguousPaths,
    UndefinedServerVariable,
    ServerVariablesEmptyEnum,
    SchemaTypeMismatch,
}

struct Simple(Kind);

fn top(ty: Ty) -> &'static [V] {
    const INFO: &[V] = &[V::on(Target::T(Ty::Info), 0)];
    const EXAMPLE: &[V] = &[V::on(Target::T(Ty::Example), 0)];
    const SCHEMA: &[V] = &[V::on(Target::T(Ty::Schema), 0)];
    const PATH_ITEM: &[V] = &[V::on(Target::T(Ty::PathItem), 0)];
    const ROOT: &[V] = &[V::on(Target::T(Ty::Root), 0)];
    const OPERATION: &[V] = &[V::on(Target::T(Ty::Operation), 0)];
    const SERVER: &[V] = &[V::on(Target::T(Ty::Server), 0)];
    const TAG: &[V] = &[V::on(Target::T(Ty::Tag), 0)];
    const PATHS: &[V] = &[V::on(Target::T(Ty::Paths), 0)];
    match ty {
        Ty::Info => INFO,
        Ty::Example => EXAMPLE,
        Ty::Schema => SCHEMA,
        Ty::PathItem => PATH_ITEM,
        Ty::Root => ROOT,
        Ty::Operation => OPERATION,
        Ty::Server => SERVER,
        Ty::Tag => TAG,
        _ => PATHS,
    }
}

impl Rule for Simple {
    fn visitors(&self) -> &'static [V] {
        top(match self.0 {
            Kind::InfoLicense => Ty::Info,
            Kind::ExampleValue => Ty::Example,
            Kind::EnumTypeMismatch | Kind::SchemaTypeMismatch => Ty::Schema,
            Kind::PathTrailingSlash | Kind::PathDeclaration => Ty::PathItem,
            Kind::EmptyServers | Kind::ServerVariablesEmptyEnum => Ty::Root,
            Kind::OperationIdUrlSafe | Kind::OperationSummary => Ty::Operation,
            Kind::ServerExample | Kind::ServerTrailingSlash | Kind::UndefinedServerVariable => Ty::Server,
            Kind::TagDescription => Ty::Tag,
            Kind::IdenticalPaths | Kind::AmbiguousPaths => Ty::Paths,
        })
    }

    fn visit(&mut self, _: u8, node: &Node, cx: &mut Cx<'_>) {
        match self.0 {
            Kind::InfoLicense => {
                if !truthy_at(node, "license") {
                    cx.report("Info object should contain `license` field.");
                }
            }
            Kind::ExampleValue => {
                if truthy_at(node, "value") && truthy_at(node, "externalValue") {
                    let at = cx.location.child("value");
                    cx.report_at("Example object can have either `value` or `externalValue` fields.", &at);
                }
            }
            Kind::EnumTypeMismatch => enum_type_mismatch(node, cx),
            Kind::PathTrailingSlash => {
                let key = cx.key.to_string();
                if key.ends_with('/') && key != "/" {
                    let at = cx.raw_location.clone();
                    cx.report_at(&format!("`{key}` should not have a trailing slash."), &at);
                }
            }
            Kind::EmptyServers => match node.get("servers") {
                None => {
                    let at = cx.location.child("openapi");
                    cx.report_at("Servers must be present.", &at);
                }
                Some(s) if !matches!(&s, Node::Arr(a) if !a.borrow().is_empty()) => {
                    let at = cx.location.child("servers");
                    cx.report_at("Servers must be a non-empty array.", &at);
                }
                Some(_) => {}
            },
            Kind::PathDeclaration => {
                if cx.key.to_string().contains("{}") {
                    cx.report("Path parameter declarations must be non-empty. `{}` is invalid.");
                }
            }
            Kind::OperationIdUrlSafe => {
                if let Some(id) = node.get("operationId").filter(Node::truthy)
                    && !js_string(&id)
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "-._~:/?#[]@!$&'()*+,;=".contains(c))
                {
                    let at = cx.location.child("operationId");
                    cx.report_at("Operation `operationId` should not have URL invalid characters.", &at);
                }
            }
            Kind::OperationSummary => defined_and_non_empty("summary", node, cx),
            Kind::TagDescription => defined_and_non_empty("description", node, cx),
            Kind::ServerExample => {
                if let Some(url) = node.get("url").filter(Node::truthy)
                    && points_to_example(&js_string(&url))
                {
                    let at = cx.location.child("url");
                    cx.report_at("Server `url` should not point to example.com or localhost.", &at);
                }
            }
            Kind::ServerTrailingSlash => {
                if str_at(node, "url").is_some_and(|u| u.ends_with('/') && u != "/") {
                    let at = cx.location.child("url");
                    cx.report_at("Server `url` should not have a trailing slash.", &at);
                }
            }
            Kind::IdenticalPaths => {
                let mut seen: FxHashMap<String, String> = FxHashMap::default();
                for path in keys(node) {
                    let id = replace_braces(&path);
                    if let Some(existing) = seen.get(&id) {
                        let msg = format!(
                            "The path already exists which differs only by path parameter name(s): `{existing}` and `{path}`."
                        );
                        let at = cx.location.child(&path);
                        cx.report_at(&msg, &at);
                    } else {
                        seen.insert(id, path);
                    }
                }
            }
            Kind::AmbiguousPaths => {
                let paths = keys(node);
                let mut seen: Vec<(PathParts<'_>, usize, &str)> = Vec::new();
                for path in &paths {
                    let parts: Vec<(&str, bool)> = path.split('/').map(|p| (p, is_var(p))).collect();
                    let vars = parts.iter().filter(|(_, v)| *v).count();
                    if let Some((_, _, other)) = seen.iter().find(|(o, ov, _)| *ov == vars && ambiguous(o, &parts)) {
                        let msg = format!(
                            "Paths should resolve unambiguously. Found two ambiguous paths: `{other}` and `{path}`."
                        );
                        let at = cx.location.child(path);
                        cx.report_at(&msg, &at);
                    }
                    seen.push((parts, vars, path));
                }
            }
            Kind::UndefinedServerVariable => undefined_server_variable(node, cx),
            Kind::ServerVariablesEmptyEnum => server_variables_empty_enum(node, cx),
            Kind::SchemaTypeMismatch => {
                let ty = str_at(node, "type");
                if ty.as_deref() == Some("object") && truthy_at(node, "items") {
                    let at = cx.location.child("items");
                    cx.report_at("Schema type mismatch: 'object' type should not contain 'items' field.", &at);
                }
                if ty.as_deref() == Some("array") && truthy_at(node, "properties") {
                    let at = cx.location.child("properties");
                    cx.report_at("Schema type mismatch: 'array' type should not contain 'properties' field.", &at);
                }
            }
        }
    }
}

pub(super) fn keys(node: &Node) -> Vec<String> {
    match node {
        Node::Obj(m) => m.borrow().keys().cloned().collect(),
        Node::Arr(a) => (0..a.borrow().len()).map(|i| i.to_string()).collect(),
        Node::Prim(Value::String(s)) => (0..s.encode_utf16().count()).map(|i| i.to_string()).collect(),
        Node::Prim(_) => Vec::new(),
    }
}

/// `validateDefinedAndNonEmpty`.
fn defined_and_non_empty(field: &str, node: &Node, cx: &mut Cx<'_>) {
    if !is_plain_object(node) {
        return;
    }
    let ty = super::structure::type_name(cx.ty);
    let msg = match node.get(field) {
        None => format!("{ty} object should contain `{field}` field."),
        Some(v) if !v.truthy() => format!("{ty} object `{field}` must be non-empty string."),
        Some(_) => return,
    };
    let at = cx.location.child(field);
    cx.report_at(&msg, &at);
}

fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// `/^(.*[\/.])?(example\.com|localhost)([\/:?].*|$)/`.
fn points_to_example(url: &str) -> bool {
    let line_end = url.find(is_line_terminator).unwrap_or(url.len());
    let host_at = |i: usize| {
        for host in ["example.com", "localhost"] {
            if let Some(rest) = url[i..].strip_prefix(host)
                && (rest.is_empty() || rest.starts_with(['/', ':', '?']))
            {
                return true;
            }
        }
        false
    };
    if host_at(0) {
        return true;
    }
    url.char_indices().take_while(|(i, _)| *i < line_end).any(|(i, c)| matches!(c, '/' | '.') && host_at(i + 1))
}

/// `pathName.replace(/{.+?}/g, '{VARIABLE}')`.
fn replace_braces(path: &str) -> String {
    let mut out = String::new();
    let mut rest = path;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let mut close = None;
        for (i, c) in after.char_indices() {
            if is_line_terminator(c) {
                break;
            }
            if c == '}' && i > 0 {
                close = Some(i);
                break;
            }
        }
        if let Some(i) = close {
            out.push_str(&rest[..open]);
            out.push_str("{VARIABLE}");
            rest = &after[i + 1..];
        } else {
            out.push_str(&rest[..=open]);
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// `/^{.+?}$/`.
fn is_var(part: &str) -> bool {
    part.len() > 2 && part.starts_with('{') && part.ends_with('}') && !part.contains(is_line_terminator)
}

type PathParts<'a> = Vec<(&'a str, bool)>;

fn ambiguous(a: &[(&str, bool)], b: &[(&str, bool)]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|((x, xv), (y, yv))| *xv || *yv || x == y)
}

fn enum_type_mismatch(schema: &Node, cx: &mut Cx<'_>) {
    let Some(values) = schema.get("enum").filter(Node::truthy) else { return };
    let Node::Arr(values) = values else { return };
    let values: Vec<Node> = values.borrow().clone();
    let Some(ty) = schema.get("type").filter(Node::truthy) else { return };
    let nullable = truthy_at(schema, "nullable");
    let index_of = |v: &Node| values.iter().position(|x| strict_equal(x, v));
    if let Node::Arr(types) = &ty {
        {
            let types: Vec<String> = types.borrow().iter().map(js_string).collect();
            let mut kept: Vec<String> = Vec::new();
            for v in &values {
                let k = js_string(v);
                if !kept.contains(&k) {
                    kept.push(k.clone());
                }
                if !types.iter().all(|t| !matches_type(v, t, nullable)) {
                    kept.retain(|x| *x != k);
                }
            }
            kept.sort_by_key(|k| array_index(k).map_or(u64::MAX, u64::from));
            let joined = js_string(&ty);
            for k in kept {
                let i = values.iter().position(|x| matches!(x, Node::Prim(Value::String(s)) if *s == k));
                let at = cx.location.child("enum");
                let at = match i {
                    Some(i) => at.index(i),
                    None => at.child("-1"),
                };
                cx.report_at(&format!("Enum value `{k}` must be of allowed types: `{joined}`."), &at);
            }
        }
    } else {
        {
            let t = js_string(&ty);
            for v in &values {
                if matches_type(v, &t, nullable) {
                    continue;
                }
                let msg = format!(
                    "All values of `enum` field must be of the same type as the `type` field: expected \"{t}\" but received \"{}\".",
                    oas_type_of(v)
                );
                let at = cx.location.child("enum");
                let at = index_of(v).map_or_else(|| at.child("-1"), |i| at.index(i));
                cx.report_at(&msg, &at);
            }
        }
    }
}

/// A canonical array index, which JS object keys enumerate first in ascending order.
fn array_index(k: &str) -> Option<u32> {
    let n: u32 = k.parse().ok()?;
    (n.to_string() == k && n < u32::MAX).then_some(n)
}

/// `===`.
#[expect(clippy::float_cmp, reason = "mirrors JavaScript `===` on numbers")]
fn strict_equal(a: &Node, b: &Node) -> bool {
    match (a, b) {
        (Node::Prim(Value::Number(x)), Node::Prim(Value::Number(y))) => x == y,
        (Node::Prim(x), Node::Prim(y)) => x == y,
        _ => a.id().is_some() && a.id() == b.id(),
    }
}

fn undefined_server_variable(server: &Node, cx: &mut Cx<'_>) {
    let Some(url) = server.get("url").filter(Node::truthy) else { return };
    let url = js_string(&url);
    let mut used = Vec::new();
    let mut rest = url.as_str();
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(0) => rest = after,
            Some(i) => {
                used.push(after[..i].to_owned());
                rest = &after[i + 1..];
            }
            None => break,
        }
    }
    let defined = server.get("variables").filter(Node::truthy).map(|v| keys(&v)).unwrap_or_default();
    for name in &used {
        if !defined.contains(name) {
            let at = cx.location.child("url");
            cx.report_at(&format!("The `{name}` variable is not defined in the `variables` objects."), &at);
        }
    }
    for name in &defined {
        if !used.contains(name) {
            let at = cx.location.child("variables").child(name);
            cx.report_at(&format!("The `{name}` variable is not used in the server's `url` field."), &at);
        }
    }
}

fn server_variables_empty_enum(root: &Node, cx: &mut Cx<'_>) {
    let Some(servers) = root.get("servers").filter(Node::truthy) else { return };
    let list: Vec<Node> = match &servers {
        Node::Arr(a) if a.borrow().is_empty() => return,
        Node::Arr(a) => a.borrow().clone(),
        other => vec![other.clone()],
    };
    let mut errors: Vec<bool> = Vec::new();
    for server in &list {
        let Some(vars) = server.get("variables") else { continue };
        let Node::Obj(vars) = vars else { continue };
        for (_, var) in vars.borrow().iter() {
            let Some(values) = var.get("enum").filter(Node::truthy) else { continue };
            if matches!(&values, Node::Arr(a) if a.borrow().is_empty()) {
                errors.push(true);
            }
            let Some(default) = var.get("default").filter(Node::truthy) else { continue };
            let included = match &values {
                Node::Arr(a) => a.borrow().iter().any(|v| strict_equal(v, &default)),
                Node::Prim(Value::String(s)) => s.contains(&js_string(&default)),
                _ => true,
            };
            if !included {
                errors.push(false);
            }
        }
    }
    for empty in errors {
        let at = cx.location.child("servers");
        if empty {
            cx.report_at("Server variable with `enum` must be a non-empty array.", &at);
        } else {
            cx.report_at("Server variable define `enum` and `default`. `enum` must include default value", &at);
        }
    }
}

struct InfoLicenseStrict;

impl Rule for InfoLicenseStrict {
    fn visitors(&self) -> &'static [V] {
        const LICENSE: &[V] =
            &[V { target: Target::T(Ty::License), enter: None, leave: Some(0), weak: &[Ty::Info], children: &[] }];
        const VS: &[V] = &[V { target: Target::T(Ty::Root), enter: None, leave: None, weak: &[], children: LICENSE }];
        VS
    }

    fn visit(&mut self, _: u8, license: &Node, cx: &mut Cx<'_>) {
        if !is_plain_object(license) {
            return;
        }
        if !cx.v31 {
            defined_and_non_empty("url", license, cx);
            return;
        }
        let fields = ["url", "identifier"];
        if !fields.iter().any(|f| license.get(f).is_some()) {
            cx.report("License object should contain one of the fields: `url`, `identifier`.");
        }
        for f in fields {
            if license.get(f).is_some_and(|v| !v.truthy()) {
                let at = cx.location.child(f);
                cx.report_at(&format!("License object `{f}` must be non-empty string."), &at);
            }
        }
    }
}

struct ResponseCodes {
    range: char,
    webhooks: bool,
}

impl Rule for ResponseCodes {
    fn visitors(&self) -> &'static [V] {
        const RESPONSES: &[V] = &[V::on(Target::T(Ty::Responses), 0)];
        const WEBHOOK_RESPONSES: &[V] = &[V::on(Target::T(Ty::Responses), 1)];
        const VS: &[V] = &[
            V { target: Target::T(Ty::Paths), enter: None, leave: None, weak: &[], children: RESPONSES },
            V { target: Target::T(Ty::WebhooksMap), enter: None, leave: None, weak: &[], children: WEBHOOK_RESPONSES },
        ];
        VS
    }

    fn visit(&mut self, id: u8, responses: &Node, cx: &mut Cx<'_>) {
        if id == 1 && !self.webhooks {
            return;
        }
        let codes = if responses.truthy() { keys(responses) } else { Vec::new() };
        let ok = codes.iter().any(|code| {
            let b = code.as_bytes();
            (self.range == '2' && code == "default")
                || (b.len() == 3
                    && char::from(b[0]) == self.range
                    && b[1..].iter().all(|c| c.is_ascii_digit() || *c == b'X' || *c == b'x'))
        });
        if !ok {
            cx.report(&format!("Operation must have at least one `{}XX` response.", self.range));
        }
    }
}

struct OperationIdUnique(FxHashSet<super::NodeKey>);

impl Rule for OperationIdUnique {
    fn visitors(&self) -> &'static [V] {
        top(Ty::Operation)
    }

    fn visit(&mut self, _: u8, op: &Node, cx: &mut Cx<'_>) {
        let Some(id) = op.get("operationId").filter(Node::truthy) else { return };
        let key = super::node_key(&id);
        if self.0.contains(&key) {
            let at = cx.location.child(&js_string(&id));
            cx.report_at("Every operation must have a unique `operationId`.", &at);
        }
        self.0.insert(key);
    }
}

/// `${parameter.in}___${parameter.name}`.
fn param_id(p: &Node) -> String {
    let part = |k: &str| p.get(k).map_or_else(|| "undefined".to_owned(), |v| js_string(&v));
    format!("{}___{}", part("in"), part("name"))
}

#[derive(Default)]
struct ParametersUnique {
    path: FxHashSet<String>,
    operation: FxHashSet<String>,
}

const PATH_ITEM_PARAMS: &[V] = &[
    V {
        target: Target::T(Ty::Parameter),
        enter: Some(1),
        leave: None,
        weak: &[Ty::ParameterList, Ty::Operation],
        children: &[],
    },
    V {
        target: Target::T(Ty::Operation),
        enter: Some(2),
        leave: Some(3),
        weak: &[],
        children: &[V {
            target: Target::T(Ty::Parameter),
            enter: Some(4),
            leave: None,
            weak: &[Ty::ParameterList],
            children: &[],
        }],
    },
];

impl Rule for ParametersUnique {
    fn visitors(&self) -> &'static [V] {
        const VS: &[V] = &[V {
            target: Target::T(Ty::PathItem),
            enter: Some(0),
            leave: None,
            weak: &[],
            children: PATH_ITEM_PARAMS,
        }];
        VS
    }

    fn visit(&mut self, id: u8, p: &Node, cx: &mut Cx<'_>) {
        let part = |k: &str| p.get(k).map_or_else(|| "undefined".to_owned(), |v| js_string(&v));
        match id {
            0 => self.path.clear(),
            1 => {
                let pid = param_id(p);
                if !self.path.insert(pid)
                    && let Some(at) = cx.parent_location(Ty::PathItem).cloned()
                {
                    let msg = format!(
                        "Paths must have unique `name` + `in` parameters.\nRepeats of `in:{}` + `name:{}`.",
                        part("in"),
                        part("name")
                    );
                    cx.report_at(&msg, &at.child("parameters").key_child(cx.key));
                }
            }
            2 => self.operation.clear(),
            4 => {
                let pid = param_id(p);
                if !self.operation.insert(pid)
                    && let Some(at) = cx.parent_location(Ty::Operation).cloned()
                {
                    let msg = format!(
                        "Operations must have unique `name` + `in` parameters. Repeats of `in:{}` + `name:{}`.",
                        part("in"),
                        part("name")
                    );
                    cx.report_at(&msg, &at.child("parameters").key_child(cx.key));
                }
            }
            _ => {}
        }
    }
}

struct OperationOperationId;

impl Rule for OperationOperationId {
    fn visitors(&self) -> &'static [V] {
        const OPERATION: &[V] = &[V::on(Target::T(Ty::Operation), 0)];
        const PATH_ITEM: &[V] =
            &[V { target: Target::T(Ty::PathItem), enter: None, leave: None, weak: &[], children: OPERATION }];
        const VS: &[V] = &[V { target: Target::T(Ty::Root), enter: None, leave: None, weak: &[], children: PATH_ITEM }];
        VS
    }

    fn visit(&mut self, _: u8, op: &Node, cx: &mut Cx<'_>) {
        defined_and_non_empty("operationId", op, cx);
    }
}

struct Usage {
    used: bool,
    name: String,
    location: Loc,
}

#[derive(Default)]
struct UnusedComponents {
    components: Vec<(String, Usage)>,
    index: FxHashMap<String, usize>,
}

impl UnusedComponents {
    fn set(&mut self, abs: String, usage: Usage) {
        if let Some(&i) = self.index.get(&abs) {
            self.components[i].1 = usage;
        } else {
            self.index.insert(abs.clone(), self.components.len());
            self.components.push((abs, usage));
        }
    }
}

impl Rule for UnusedComponents {
    fn visitors(&self) -> &'static [V] {
        const fn named(parent: Ty, child: &'static [V]) -> V {
            V { target: Target::T(parent), enter: None, leave: None, weak: &[], children: child }
        }
        const VS: &[V] = &[
            V { target: Target::T(Ty::Root), enter: None, leave: Some(0), weak: &[], children: &[] },
            named(Ty::NamedSchemas, &[V::on(Target::T(Ty::Schema), 1)]),
            named(Ty::NamedResponses, &[V::on(Target::T(Ty::Response), 2)]),
            named(Ty::NamedParameters, &[V::on(Target::T(Ty::Parameter), 2)]),
            named(Ty::NamedExamples, &[V::on(Target::T(Ty::Example), 2)]),
            named(Ty::NamedRequestBodies, &[V::on(Target::T(Ty::RequestBody), 2)]),
            named(Ty::NamedHeaders, &[V::on(Target::T(Ty::Header), 2)]),
            V::on(Target::Ref, 3),
        ];
        VS
    }

    fn visit(&mut self, id: u8, node: &Node, cx: &mut Cx<'_>) {
        match id {
            0 => {
                let unused: Vec<(String, Loc)> = self
                    .components
                    .iter()
                    .filter(|(_, u)| !u.used)
                    .map(|(_, u)| (u.name.clone(), u.location.clone()))
                    .collect();
                for (name, at) in unused {
                    cx.report_at(&format!("Component: \"{name}\" is never used."), &at);
                }
            }
            1 if node.get("allOf").is_some_and(|v| v.truthy()) => {}
            _ => {
                let abs = cx.absolute(&cx.location);
                let used = self.index.get(&abs).is_some_and(|&i| self.components[i].1.used);
                self.set(abs, Usage { used, name: cx.key.to_string(), location: cx.location.clone() });
            }
        }
    }

    fn visit_ref(&mut self, _: u8, node: &Node, cx: &mut Cx<'_>, _: Option<&(Node, Loc)>) {
        if !matches!(cx.ty, Ty::Schema | Ty::Header | Ty::Parameter | Ty::Response | Ty::Example | Ty::RequestBody) {
            return;
        }
        let Some(at) = cx.resolve_location(node) else { return };
        let abs = cx.absolute(&at);
        self.set(abs, Usage { used: true, name: cx.key.to_string(), location: cx.location.clone() });
    }
}

struct PathNotIncludeQuery;

impl Rule for PathNotIncludeQuery {
    fn visitors(&self) -> &'static [V] {
        const PATH_ITEM: &[V] = &[V::on(Target::T(Ty::PathItem), 0)];
        const VS: &[V] =
            &[V { target: Target::T(Ty::Paths), enter: None, leave: None, weak: &[], children: PATH_ITEM }];
        VS
    }

    fn visit(&mut self, _: u8, _: &Node, cx: &mut Cx<'_>) {
        if cx.key.to_string().contains('?') {
            cx.report("Don't put query string items in the path, they belong in parameters with `in: query`.");
        }
    }
}

#[derive(Default)]
struct PathParamsDefined {
    template: Vec<String>,
    path: FxHashSet<String>,
    current: String,
    operation: FxHashSet<String>,
}

/// `/\{([a-zA-Z0-9_.-]+)\}+/g`, deduplicated in order of appearance.
fn template_params(path: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let b = path.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'{' {
            let start = i + 1;
            let mut j = start;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || matches!(b[j], b'_' | b'.' | b'-')) {
                j += 1;
            }
            if j > start && j < b.len() && b[j] == b'}' {
                let name = path[start..j].to_owned();
                if !out.contains(&name) {
                    out.push(name);
                }
                while j < b.len() && b[j] == b'}' {
                    j += 1;
                }
                i = j;
                continue;
            }
        }
        i += 1;
    }
    out
}

impl Rule for PathParamsDefined {
    fn visitors(&self) -> &'static [V] {
        const VS: &[V] = &[V {
            target: Target::T(Ty::PathItem),
            enter: Some(0),
            leave: None,
            weak: &[],
            children: PATH_ITEM_PARAMS,
        }];
        VS
    }

    fn visit(&mut self, id: u8, node: &Node, cx: &mut Cx<'_>) {
        match id {
            0 => {
                self.path.clear();
                self.current = cx.key.to_string();
                self.template = template_params(&self.current);
            }
            1 | 4 => {
                if str_at(node, "in").as_deref() != Some("path") {
                    return;
                }
                let Some(name) = node.get("name").filter(Node::truthy) else { return };
                let name = js_string(&name);
                if id == 1 {
                    self.path.insert(name.clone());
                } else {
                    self.operation.insert(name.clone());
                }
                if !self.template.contains(&name) {
                    let at = cx.location.child("name");
                    let msg = format!("Path parameter `{name}` is not used in the path `{}`.", self.current);
                    cx.report_at(&msg, &at);
                }
            }
            2 => self.operation.clear(),
            3 => {
                for p in self.template.clone() {
                    if !self.operation.contains(&p) && !self.path.contains(&p) {
                        let at = cx.location.child("parameters");
                        let msg = format!(
                            "The operation does not define the path parameter `{{{p}}}` expected by path `{}`.",
                            self.current
                        );
                        cx.report_at(&msg, &at);
                    }
                }
            }
            _ => {}
        }
    }
}

struct Scheme {
    defined: bool,
    from: Vec<Loc>,
}

struct SecurityDefined {
    schemes: Vec<(String, Scheme)>,
    without: Vec<Loc>,
    each_has: bool,
    path: String,
    exceptions: Vec<(String, Option<Vec<String>>)>,
}

impl SecurityDefined {
    fn new(exceptions: Option<&Value>) -> Self {
        let exceptions = exceptions
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .map(|e| {
                        let path = e.get("path").map(Value::js_string).unwrap_or_default();
                        let methods = e
                            .get("methods")
                            .and_then(Value::as_array)
                            .map(|m| m.iter().map(Value::js_string).collect());
                        (path, methods)
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self { schemes: Vec::new(), without: Vec::new(), each_has: true, path: String::new(), exceptions }
    }
}

impl Rule for SecurityDefined {
    fn visitors(&self) -> &'static [V] {
        const OPERATION: &[V] = &[V::on(Target::T(Ty::Operation), 4)];
        const VS: &[V] = &[
            V { target: Target::T(Ty::Root), enter: None, leave: Some(0), weak: &[], children: &[] },
            V { target: Target::T(Ty::PathItem), enter: Some(3), leave: None, weak: &[], children: OPERATION },
            V::on(Target::T(Ty::SecurityRequirement), 2),
            V::on(Target::T(Ty::SecurityScheme), 1),
        ];
        VS
    }

    fn visit(&mut self, id: u8, node: &Node, cx: &mut Cx<'_>) {
        match id {
            0 => {
                for (name, scheme) in &self.schemes {
                    if scheme.defined {
                        continue;
                    }
                    for at in &scheme.from {
                        cx.report_at(&format!("There is no `{name}` security scheme defined."), at);
                    }
                }
                if node.get("security").is_some_and(|s| s.truthy()) || self.each_has {
                    return;
                }
                for at in &self.without {
                    cx.report_at("Every operation should have security defined on it or on the root level.", at);
                }
            }
            1 => {
                let key = cx.key.to_string();
                let scheme = Scheme { defined: true, from: Vec::new() };
                match self.schemes.iter_mut().find(|(k, _)| *k == key) {
                    Some((_, slot)) => *slot = scheme,
                    None => self.schemes.push((key, scheme)),
                }
            }
            2 => {
                for name in keys(node) {
                    let at = cx.location.child(&name);
                    match self.schemes.iter_mut().find(|(k, _)| *k == name) {
                        Some((_, s)) => s.from.push(at),
                        None => self.schemes.push((name, Scheme { defined: false, from: vec![at] })),
                    }
                }
            }
            3 => self.path = cx.key.to_string(),
            _ => {
                let method = cx.key.to_string();
                let exception = self.exceptions.iter().any(|(path, methods)| {
                    *path == self.path && methods.as_ref().is_none_or(|m| m.iter().any(|x| x.to_lowercase() == method))
                });
                if !node.get("security").is_some_and(|s| s.truthy()) && !exception {
                    self.each_has = false;
                    self.without.push(cx.location.clone());
                }
            }
        }
    }
}

struct UnresolvedRefs;

impl Rule for UnresolvedRefs {
    fn visitors(&self) -> &'static [V] {
        const VS: &[V] = &[
            V::on(Target::T(Ty::DiscriminatorMapping), 0),
            V { target: Target::Ref, enter: None, leave: Some(1), weak: &[], children: &[] },
        ];
        VS
    }

    fn visit(&mut self, _: u8, mapping: &Node, cx: &mut Cx<'_>) {
        for name in keys(mapping) {
            let value = mapping.get(&name).unwrap_or(Node::Prim(Value::Null));
            let probe = Node::new_obj([("$ref".to_owned(), value)]);
            if cx.resolve(&probe).is_some() {
                return;
            }
            let at = cx.location.child(&name);
            cx.report_at("Can't resolve $ref", &at);
        }
    }

    fn visit_ref(&mut self, _: u8, _: &Node, cx: &mut Cx<'_>, resolved: Option<&(Node, Loc)>) {
        if resolved.is_none() {
            cx.report("Can't resolve $ref");
        }
    }
}

struct ComponentsMapName;

impl Rule for ComponentsMapName {
    fn visitors(&self) -> &'static [V] {
        const fn named(parent: Ty, child: &'static [V]) -> V {
            V { target: Target::T(parent), enter: None, leave: None, weak: &[], children: child }
        }
        const VS: &[V] = &[
            named(Ty::NamedSchemas, &[V::on(Target::T(Ty::Schema), 0)]),
            named(Ty::NamedResponses, &[V::on(Target::T(Ty::Response), 2)]),
            named(Ty::NamedParameters, &[V::on(Target::T(Ty::Parameter), 1)]),
            named(Ty::NamedExamples, &[V::on(Target::T(Ty::Example), 3)]),
            named(Ty::NamedRequestBodies, &[V::on(Target::T(Ty::RequestBody), 4)]),
            named(Ty::NamedHeaders, &[V::on(Target::T(Ty::Header), 5)]),
            named(Ty::NamedSecuritySchemes, &[V::on(Target::T(Ty::SecurityScheme), 6)]),
            named(Ty::NamedLinks, &[V::on(Target::T(Ty::Link), 7)]),
            named(Ty::NamedCallbacks, &[V::on(Target::T(Ty::Callback), 8)]),
        ];
        VS
    }

    fn visit(&mut self, id: u8, _: &Node, cx: &mut Cx<'_>) {
        let kind = [
            ComponentKind::Schemas,
            ComponentKind::Parameters,
            ComponentKind::Responses,
            ComponentKind::Examples,
            ComponentKind::RequestBodies,
            ComponentKind::Headers,
            ComponentKind::SecuritySchemes,
            ComponentKind::Links,
            ComponentKind::Callbacks,
        ][usize::from(id)];
        let key = cx.key.to_string();
        let valid = !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
        if !valid {
            let msg = format!(
                "The map key in {kind} \"{key}\" does not match the regular expression \"^[a-zA-Z0-9\\.\\-_]+$\""
            );
            cx.report(&msg);
        }
    }
}

//! Redocly's `OpenAPI` 3.0 / 3.1 type tree (`types/oas3.js`, `types/oas3_1.js`), reduced to what
//! the walk needs: which keys are visited, in which order, and as which type.

use super::Node;
use spectype_document::oas::ComponentKind;

/// A named node type. `Scalar` is Redocly's `{ name: 'scalar' }` for unnamed but resolvable
/// specs; `Unknown` only exists while resolving.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Ty {
    Root,
    Tag,
    TagList,
    TagGroups,
    TagGroup,
    ExternalDocs,
    Server,
    ServerList,
    ServerVariable,
    ServerVariablesMap,
    SecurityRequirement,
    SecurityRequirementList,
    Info,
    Contact,
    License,
    Logo,
    Paths,
    WebhooksMap,
    PathItem,
    Parameter,
    ParameterList,
    Operation,
    Callback,
    CallbacksMap,
    RequestBody,
    MediaTypesMap,
    MediaType,
    Example,
    ExamplesMap,
    Encoding,
    EncodingMap,
    Header,
    HeadersMap,
    Responses,
    Response,
    Link,
    LinksMap,
    Schema,
    SchemaList,
    SchemaMap,
    Xml,
    SchemaProperties,
    PatternProperties,
    DependentRequired,
    DiscriminatorMapping,
    Discriminator,
    Components,
    NamedSchemas,
    NamedResponses,
    NamedParameters,
    NamedExamples,
    NamedRequestBodies,
    NamedHeaders,
    NamedSecuritySchemes,
    NamedLinks,
    NamedCallbacks,
    NamedPathItems,
    ImplicitFlow,
    PasswordFlow,
    ClientCredentials,
    AuthorizationCode,
    OAuth2Flows,
    SecurityScheme,
    XCodeSample,
    XCodeSampleList,
    XUsePkce,
    SpecExtension,
    Scalar,
    Unknown,
}

/// A property spec as written in the type tree.
#[derive(Debug, Clone, Copy)]
pub enum P {
    T(Ty),
    /// An unnamed spec such as `{ type: 'string' }`.
    S,
    /// `null`: never visited.
    Null,
    /// 3.0 Schema `items`: a list of schemas for arrays.
    Items30,
    /// `{ type: 'boolean' }` for booleans, the named type otherwise.
    BoolOr(Ty),
    /// `Paths`: keys starting with `/`.
    PathKey,
    /// `Responses`: status-code-like keys.
    ResponseKey,
    /// `DiscriminatorMapping`: `directResolveAs: 'Schema'` for ref-like strings.
    Mapping,
}

/// A property spec after applying it to a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pt {
    Ty(Ty),
    /// Resolve the string value as `{ $ref: value }` of type Schema.
    Direct,
    Null,
}

pub struct TypeDef {
    pub props: &'static [(&'static str, P)],
    pub additional: Option<P>,
    pub items: Option<Ty>,
    pub ext: bool,
}

impl TypeDef {
    /// `type.properties[key] ?? type.additionalProperties`, applied to `value`, with Redocly's
    /// `SpecExtension` fallback for `x-` keys. `None` is `undefined`.
    ///
    /// `type.properties` is a plain JS object, so `toString` and `constructor` find the
    /// inherited functions (called as `fn(value)`) and `__proto__` finds `Object.prototype`.
    /// All of these end up as unnamed specs, so the key is visited as a scalar, except when
    /// `Object(value)` has a `name`. The other `Object.prototype` methods throw when called
    /// unbound, which crashes upstream; they fall through to `additionalProperties` here.
    pub fn prop(&self, key: &str, value: &Node) -> Option<Pt> {
        let raw = match self.props.iter().find(|(k, _)| same_key(k, key)) {
            Some((_, p)) => Some(*p),
            None => match key {
                "constructor" if value.has_name() => return Some(Pt::Null),
                "toString" | "__proto__" | "constructor" => return Some(Pt::Ty(Ty::Scalar)),
                _ => self.additional,
            },
        };
        let pt = raw.and_then(|p| apply(p, value, key));
        if pt.is_none() && self.ext && key.starts_with("x-") { Some(Pt::Ty(Ty::SpecExtension)) } else { pt }
    }
}

pub fn same_key(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.as_bytes().first() == b.as_bytes().first() && a == b
}

pub fn apply(p: P, value: &Node, key: &str) -> Option<Pt> {
    Some(match p {
        P::T(t) => Pt::Ty(t),
        P::S => Pt::Ty(Ty::Scalar),
        P::Null => Pt::Null,
        P::Items30 => Pt::Ty(if matches!(value, Node::Arr(_)) { Ty::SchemaList } else { Ty::Schema }),
        P::BoolOr(t) => Pt::Ty(if value.is_bool() { Ty::Scalar } else { t }),
        P::PathKey => return key.starts_with('/').then_some(Pt::Ty(Ty::PathItem)),
        P::ResponseKey => {
            let b = key.as_bytes();
            let code = b.len() == 3
                && b[0].is_ascii_digit()
                && b[1..].iter().all(|c| c.is_ascii_digit() || *c == b'X' || *c == b'x');
            return code.then_some(Pt::Ty(Ty::Response));
        }
        P::Mapping => match value.str() {
            Some(s) if is_mapping_ref(s) => Pt::Direct,
            _ => Pt::Ty(Ty::Scalar),
        },
    })
}

fn is_mapping_ref(s: &str) -> bool {
    s.starts_with('#')
        || s.starts_with("https://")
        || s.starts_with("http://")
        || s.starts_with("./")
        || s.starts_with("../")
        || s.contains('/')
}

const fn def(props: &'static [(&'static str, P)], ext: bool) -> TypeDef {
    TypeDef { props, additional: None, items: None, ext }
}

const fn map_of(t: P) -> TypeDef {
    TypeDef { props: &[], additional: Some(t), items: None, ext: false }
}

const fn list_of(t: Ty) -> TypeDef {
    TypeDef { props: &[], additional: None, items: Some(t), ext: false }
}

use P::{BoolOr, Null, S, T};

const ROOT_30: &[(&str, P)] = &[
    ("openapi", Null),
    ("info", T(Ty::Info)),
    ("servers", T(Ty::ServerList)),
    ("security", T(Ty::SecurityRequirementList)),
    ("tags", T(Ty::TagList)),
    ("externalDocs", T(Ty::ExternalDocs)),
    ("paths", T(Ty::Paths)),
    ("components", T(Ty::Components)),
    ("x-webhooks", T(Ty::WebhooksMap)),
    ("x-tagGroups", T(Ty::TagGroups)),
    ("x-ignoredHeaderParameters", S),
];
const ROOT_31: &[(&str, P)] = &[
    ("openapi", Null),
    ("info", T(Ty::Info)),
    ("servers", T(Ty::ServerList)),
    ("security", T(Ty::SecurityRequirementList)),
    ("tags", T(Ty::TagList)),
    ("externalDocs", T(Ty::ExternalDocs)),
    ("paths", T(Ty::Paths)),
    ("webhooks", T(Ty::WebhooksMap)),
    ("components", T(Ty::Components)),
    ("jsonSchemaDialect", S),
];
const INFO_30: &[(&str, P)] = &[
    ("title", S),
    ("version", S),
    ("description", S),
    ("termsOfService", S),
    ("contact", T(Ty::Contact)),
    ("license", T(Ty::License)),
    ("x-logo", T(Ty::Logo)),
];
const INFO_31: &[(&str, P)] = &[
    ("title", S),
    ("version", S),
    ("description", S),
    ("termsOfService", S),
    ("summary", S),
    ("contact", T(Ty::Contact)),
    ("license", T(Ty::License)),
    ("x-logo", T(Ty::Logo)),
];
const COMPONENTS_30: &[(&str, P)] = &[
    ("parameters", T(Ty::NamedParameters)),
    ("schemas", T(Ty::NamedSchemas)),
    ("responses", T(Ty::NamedResponses)),
    ("examples", T(Ty::NamedExamples)),
    ("requestBodies", T(Ty::NamedRequestBodies)),
    ("headers", T(Ty::NamedHeaders)),
    ("securitySchemes", T(Ty::NamedSecuritySchemes)),
    ("links", T(Ty::NamedLinks)),
    ("callbacks", T(Ty::NamedCallbacks)),
];
const COMPONENTS_31: &[(&str, P)] = &[
    ("parameters", T(Ty::NamedParameters)),
    ("schemas", T(Ty::NamedSchemas)),
    ("responses", T(Ty::NamedResponses)),
    ("examples", T(Ty::NamedExamples)),
    ("requestBodies", T(Ty::NamedRequestBodies)),
    ("headers", T(Ty::NamedHeaders)),
    ("securitySchemes", T(Ty::NamedSecuritySchemes)),
    ("links", T(Ty::NamedLinks)),
    ("callbacks", T(Ty::NamedCallbacks)),
    ("pathItems", T(Ty::NamedPathItems)),
];
const PARAMETER: &[(&str, P)] = &[
    ("name", S),
    ("in", S),
    ("description", S),
    ("required", S),
    ("deprecated", S),
    ("allowEmptyValue", S),
    ("style", S),
    ("explode", S),
    ("allowReserved", S),
    ("schema", T(Ty::Schema)),
    ("example", S),
    ("examples", T(Ty::ExamplesMap)),
    ("content", T(Ty::MediaTypesMap)),
];
const HEADER: &[(&str, P)] = &[
    ("description", S),
    ("required", S),
    ("deprecated", S),
    ("allowEmptyValue", S),
    ("style", S),
    ("explode", S),
    ("allowReserved", S),
    ("schema", T(Ty::Schema)),
    ("example", S),
    ("examples", T(Ty::ExamplesMap)),
    ("content", T(Ty::MediaTypesMap)),
];
const OPERATION: &[(&str, P)] = &[
    ("tags", S),
    ("summary", S),
    ("description", S),
    ("externalDocs", T(Ty::ExternalDocs)),
    ("operationId", S),
    ("parameters", T(Ty::ParameterList)),
    ("security", T(Ty::SecurityRequirementList)),
    ("servers", T(Ty::ServerList)),
    ("requestBody", T(Ty::RequestBody)),
    ("responses", T(Ty::Responses)),
    ("deprecated", S),
    ("callbacks", T(Ty::CallbacksMap)),
    ("x-codeSamples", T(Ty::XCodeSampleList)),
    ("x-code-samples", T(Ty::XCodeSampleList)),
    ("x-hideTryItPanel", S),
];
const PATH_ITEM: &[(&str, P)] = &[
    ("$ref", S),
    ("servers", T(Ty::ServerList)),
    ("parameters", T(Ty::ParameterList)),
    ("summary", S),
    ("description", S),
    ("get", T(Ty::Operation)),
    ("put", T(Ty::Operation)),
    ("post", T(Ty::Operation)),
    ("delete", T(Ty::Operation)),
    ("options", T(Ty::Operation)),
    ("head", T(Ty::Operation)),
    ("patch", T(Ty::Operation)),
    ("trace", T(Ty::Operation)),
];
const SCHEMA_30: &[(&str, P)] = &[
    ("externalDocs", T(Ty::ExternalDocs)),
    ("discriminator", T(Ty::Discriminator)),
    ("title", S),
    ("multipleOf", S),
    ("maximum", S),
    ("minimum", S),
    ("exclusiveMaximum", S),
    ("exclusiveMinimum", S),
    ("maxLength", S),
    ("minLength", S),
    ("pattern", S),
    ("maxItems", S),
    ("minItems", S),
    ("uniqueItems", S),
    ("maxProperties", S),
    ("minProperties", S),
    ("required", S),
    ("enum", S),
    ("type", S),
    ("allOf", T(Ty::SchemaList)),
    ("anyOf", T(Ty::SchemaList)),
    ("oneOf", T(Ty::SchemaList)),
    ("not", T(Ty::Schema)),
    ("properties", T(Ty::SchemaProperties)),
    ("items", P::Items30),
    ("additionalProperties", BoolOr(Ty::Schema)),
    ("description", S),
    ("format", S),
    ("default", Null),
    ("nullable", S),
    ("readOnly", S),
    ("writeOnly", S),
    ("xml", T(Ty::Xml)),
    ("example", S),
    ("deprecated", S),
    ("x-tags", S),
    ("x-additionalPropertiesName", S),
    ("x-explicitMappingOnly", S),
];
const SCHEMA_31: &[(&str, P)] = &[
    ("$id", S),
    ("$anchor", S),
    ("id", S),
    ("$schema", S),
    ("definitions", T(Ty::NamedSchemas)),
    ("$defs", T(Ty::NamedSchemas)),
    ("$vocabulary", S),
    ("externalDocs", T(Ty::ExternalDocs)),
    ("discriminator", T(Ty::Discriminator)),
    ("title", S),
    ("multipleOf", S),
    ("maximum", S),
    ("minimum", S),
    ("exclusiveMaximum", S),
    ("exclusiveMinimum", S),
    ("maxLength", S),
    ("minLength", S),
    ("pattern", S),
    ("maxItems", S),
    ("minItems", S),
    ("uniqueItems", S),
    ("maxProperties", S),
    ("minProperties", S),
    ("required", S),
    ("enum", S),
    ("type", S),
    ("allOf", T(Ty::SchemaList)),
    ("anyOf", T(Ty::SchemaList)),
    ("oneOf", T(Ty::SchemaList)),
    ("not", T(Ty::Schema)),
    ("if", T(Ty::Schema)),
    ("then", T(Ty::Schema)),
    ("else", T(Ty::Schema)),
    ("dependentSchemas", T(Ty::SchemaMap)),
    ("dependentRequired", T(Ty::DependentRequired)),
    ("prefixItems", T(Ty::SchemaList)),
    ("contains", T(Ty::Schema)),
    ("minContains", S),
    ("maxContains", S),
    ("patternProperties", T(Ty::PatternProperties)),
    ("propertyNames", T(Ty::Schema)),
    ("unevaluatedItems", BoolOr(Ty::Schema)),
    ("unevaluatedProperties", BoolOr(Ty::Schema)),
    ("summary", S),
    ("properties", T(Ty::SchemaProperties)),
    ("items", BoolOr(Ty::Schema)),
    ("additionalProperties", BoolOr(Ty::Schema)),
    ("description", S),
    ("format", S),
    ("contentEncoding", S),
    ("contentMediaType", S),
    ("contentSchema", T(Ty::Schema)),
    ("default", Null),
    ("readOnly", S),
    ("writeOnly", S),
    ("xml", T(Ty::Xml)),
    ("examples", S),
    ("example", S),
    ("deprecated", S),
    ("const", Null),
    ("$comment", S),
    ("x-tags", S),
    ("$dynamicAnchor", S),
    ("$dynamicRef", S),
];
const SECURITY_SCHEME_30: &[(&str, P)] = &[
    ("type", S),
    ("description", S),
    ("name", S),
    ("in", S),
    ("scheme", S),
    ("bearerFormat", S),
    ("flows", T(Ty::OAuth2Flows)),
    ("openIdConnectUrl", S),
    ("x-defaultClientId", S),
];
const SECURITY_SCHEME_31: &[(&str, P)] = &[
    ("type", S),
    ("description", S),
    ("name", S),
    ("in", S),
    ("scheme", S),
    ("bearerFormat", S),
    ("flows", T(Ty::OAuth2Flows)),
    ("openIdConnectUrl", S),
];

/// The type definition of `ty` for a 3.0 (`v31 == false`) or 3.1 document.
pub fn type_def(ty: Ty, v31: bool) -> TypeDef {
    let pick = |a, b| if v31 { b } else { a };
    match ty {
        Ty::Root => def(pick(ROOT_30, ROOT_31), true),
        Ty::Tag => def(
            &[
                ("name", S),
                ("description", S),
                ("externalDocs", T(Ty::ExternalDocs)),
                ("x-traitTag", S),
                ("x-displayName", S),
            ],
            true,
        ),
        Ty::TagList => list_of(Ty::Tag),
        Ty::TagGroups => list_of(Ty::TagGroup),
        Ty::TagGroup => def(&[("name", S), ("tags", S)], true),
        Ty::ExternalDocs => def(&[("description", S), ("url", S)], true),
        Ty::Server => def(&[("url", S), ("description", S), ("variables", T(Ty::ServerVariablesMap))], true),
        Ty::ServerList => list_of(Ty::Server),
        Ty::ServerVariable => def(&[("enum", S), ("default", S), ("description", S)], true),
        Ty::ServerVariablesMap => map_of(T(Ty::ServerVariable)),
        Ty::SecurityRequirement | Ty::DependentRequired | Ty::SpecExtension => map_of(S),
        Ty::SecurityRequirementList => list_of(Ty::SecurityRequirement),
        Ty::Info => def(pick(INFO_30, INFO_31), true),
        Ty::Contact => def(&[("name", S), ("url", S), ("email", S)], true),
        Ty::License => def(pick(&[("name", S), ("url", S)], &[("name", S), ("url", S), ("identifier", S)]), true),
        Ty::Logo => def(&[("url", S), ("altText", S), ("backgroundColor", S), ("href", S)], false),
        Ty::Paths => map_of(P::PathKey),
        Ty::WebhooksMap | Ty::Callback | Ty::NamedPathItems => map_of(T(Ty::PathItem)),
        Ty::PathItem => def(PATH_ITEM, true),
        Ty::Parameter => def(PARAMETER, true),
        Ty::ParameterList => list_of(Ty::Parameter),
        Ty::Operation => def(OPERATION, true),
        Ty::CallbacksMap | Ty::NamedCallbacks => map_of(T(Ty::Callback)),
        Ty::RequestBody => def(&[("description", S), ("required", S), ("content", T(Ty::MediaTypesMap))], true),
        Ty::MediaTypesMap => map_of(T(Ty::MediaType)),
        Ty::MediaType => def(
            &[
                ("schema", T(Ty::Schema)),
                ("example", S),
                ("examples", T(Ty::ExamplesMap)),
                ("encoding", T(Ty::EncodingMap)),
            ],
            true,
        ),
        Ty::Example => def(&[("value", S), ("summary", S), ("description", S), ("externalValue", S)], true),
        Ty::ExamplesMap | Ty::NamedExamples => map_of(T(Ty::Example)),
        Ty::Encoding => def(
            &[("contentType", S), ("headers", T(Ty::HeadersMap)), ("style", S), ("explode", S), ("allowReserved", S)],
            true,
        ),
        Ty::EncodingMap => map_of(T(Ty::Encoding)),
        Ty::Header => def(HEADER, true),
        Ty::HeadersMap | Ty::NamedHeaders => map_of(T(Ty::Header)),
        Ty::Responses => TypeDef {
            props: &[("default", T(Ty::Response))],
            additional: Some(P::ResponseKey),
            items: None,
            ext: false,
        },
        Ty::Response => def(
            &[
                ("description", S),
                ("headers", T(Ty::HeadersMap)),
                ("content", T(Ty::MediaTypesMap)),
                ("links", T(Ty::LinksMap)),
                ("x-summary", S),
            ],
            true,
        ),
        Ty::Link => def(
            &[
                ("operationRef", S),
                ("operationId", S),
                ("parameters", Null),
                ("requestBody", Null),
                ("description", S),
                ("server", T(Ty::Server)),
            ],
            true,
        ),
        Ty::LinksMap | Ty::NamedLinks => map_of(T(Ty::Link)),
        Ty::Schema => def(pick(SCHEMA_30, SCHEMA_31), true),
        Ty::SchemaList => list_of(Ty::Schema),
        Ty::SchemaMap | Ty::NamedSchemas => map_of(T(Ty::Schema)),
        Ty::Xml => def(&[("name", S), ("namespace", S), ("prefix", S), ("attribute", S), ("wrapped", S)], true),
        Ty::SchemaProperties | Ty::PatternProperties => map_of(if v31 { BoolOr(Ty::Schema) } else { T(Ty::Schema) }),
        Ty::DiscriminatorMapping => map_of(P::Mapping),
        Ty::Discriminator => def(&[("propertyName", S), ("mapping", T(Ty::DiscriminatorMapping))], true),
        Ty::Components => def(pick(COMPONENTS_30, COMPONENTS_31), true),
        Ty::NamedResponses => map_of(T(Ty::Response)),
        Ty::NamedParameters => map_of(T(Ty::Parameter)),
        Ty::NamedRequestBodies => map_of(T(Ty::RequestBody)),
        Ty::NamedSecuritySchemes => map_of(T(Ty::SecurityScheme)),
        Ty::ImplicitFlow => def(&[("refreshUrl", S), ("scopes", S), ("authorizationUrl", S)], true),
        Ty::PasswordFlow | Ty::ClientCredentials => def(&[("refreshUrl", S), ("scopes", S), ("tokenUrl", S)], true),
        Ty::AuthorizationCode => def(
            &[
                ("refreshUrl", S),
                ("authorizationUrl", S),
                ("scopes", S),
                ("tokenUrl", S),
                ("x-usePkce", BoolOr(Ty::XUsePkce)),
            ],
            true,
        ),
        Ty::OAuth2Flows => def(
            &[
                ("implicit", T(Ty::ImplicitFlow)),
                ("password", T(Ty::PasswordFlow)),
                ("clientCredentials", T(Ty::ClientCredentials)),
                ("authorizationCode", T(Ty::AuthorizationCode)),
            ],
            true,
        ),
        Ty::SecurityScheme => def(pick(SECURITY_SCHEME_30, SECURITY_SCHEME_31), true),
        Ty::XCodeSample => def(&[("lang", S), ("label", S), ("source", S)], false),
        Ty::XCodeSampleList => list_of(Ty::XCodeSample),
        Ty::XUsePkce => def(&[("disableManualConfiguration", S), ("hideClientSecretInput", S)], false),
        Ty::Scalar | Ty::Unknown => def(&[], false),
    }
}

/// `mapTypeToComponent` for OAS3. `SecurityScheme` is missing on purpose: Redocly spells it
/// `SecuritySchema`, so security schemes are always inlined.
pub fn component_of(ty: Ty) -> Option<ComponentKind> {
    Some(match ty {
        Ty::Schema => ComponentKind::Schemas,
        Ty::Parameter => ComponentKind::Parameters,
        Ty::Response => ComponentKind::Responses,
        Ty::Example => ComponentKind::Examples,
        Ty::RequestBody => ComponentKind::RequestBodies,
        Ty::Header => ComponentKind::Headers,
        Ty::Link => ComponentKind::Links,
        Ty::Callback => ComponentKind::Callbacks,
        _ => return None,
    })
}

//! The spectype npm package's engine: the CLI and `spectype_core` as a Node-API
//! addon, with a JS object as the Node API's [`Host`] so it can load files and run
//! openapi-typescript's hooks in-process.

use napi::bindgen_prelude::{
    BufferSlice, Either, FnArgs, FromNapiValue, Function, JsObjectValue, JsValuesTupleIntoVec, Object, Unknown,
};
use napi::{JsValue, Result, ValueType};
use napi_derive::napi;
use rustc_hash::FxHashMap;

use spectype_core::lint::Config as LintConfig;
use spectype_core::ts::{PropName, Ty, TypeKind, print_type};
use spectype_core::value::Value;
use spectype_core::{
    Error, Format, HookContext, Host, HostError, LoadedDocument, Options, PropertyHookInput, TransformResult,
    generate_with_host, load_file, parse_document,
};

#[cfg(not(target_family = "wasm"))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// Runs the CLI in `cwd` with `args` (without the program name). Returns whether it succeeded.
///
/// WebAssembly does not inherit the process's working directory and has no network, so JS
/// passes `cwd` and a `host` whose synchronous `fetch(url, headers)` (`headers` as JSON
/// `[name, value]` pairs) returns `{ text, mimeType }` or throws. The native addon fetches by itself.
#[napi]
#[must_use]
pub fn cli(args: Vec<String>, cwd: String, host: Object) -> bool {
    let _ = std::env::set_current_dir(cwd);
    #[cfg(target_family = "wasm")]
    {
        let fetch: Option<Function<FnArgs<(String, String)>, Unknown>> = method(&host, "fetch").ok().flatten();
        spectype::run_cli_with(
            "spectype",
            args,
            Box::new(move |url, headers| match &fetch {
                Some(fetch) => js_fetch(fetch, url, headers),
                None => Err(HostError::new(format!("Failed to load {url}: host.fetch must be a function"))),
            }),
        )
    }
    #[cfg(not(target_family = "wasm"))]
    {
        let _ = host;
        spectype::run_cli("spectype", args)
    }
}

#[cfg(target_family = "wasm")]
fn js_fetch(
    fetch: &Function<FnArgs<(String, String)>, Unknown>,
    url: &str,
    headers: &[(String, String)],
) -> std::result::Result<LoadedDocument, HostError> {
    let headers = serde_json::to_string(headers).unwrap_or_default();
    let loaded = || -> Result<LoadedDocument> {
        let doc = object(fetch.call((url.to_owned(), headers.clone()).into())?)?
            .ok_or_else(|| napi::Error::from_reason("no document"))?;
        let text: Option<String> = doc.get("text")?;
        Ok(LoadedDocument { text: text.unwrap_or_default(), mime_type: doc.get("mimeType")? })
    };
    loaded().map_err(|e| HostError::new(e.reason))
}

/// The headers Redocly sends with a request for `url`, as `[name, value]` pairs. `headers` is
/// `resolve.http.headers` as JSON; an entry's `envVariable` must already be replaced by its
/// `value`.
#[napi]
#[must_use]
pub fn request_headers(url: String, headers: String) -> Vec<Vec<String>> {
    let mut json = headers;
    json.insert_str(0, r#"{"http":{"headers":"#);
    json.push_str("}}");
    let resolve = parse_document(&json, Format::Json).ok();
    let list = spectype_core::http::headers_of(resolve.as_ref());
    spectype_core::http::request_headers(&url.into_boxed_str(), &list)
        .into_iter()
        .map(|(name, value)| vec![name, value])
        .collect()
}

/// What [`generate`] returns: the generated file and its warnings, or why it failed.
#[napi(object)]
pub struct Generated {
    /// The generated file, header included.
    pub code: Option<String>,
    /// Warnings, in the order openapi-typescript prints them.
    pub warnings: Vec<String>,
    /// Set when generation failed.
    pub error: Option<GenerateError>,
}

/// Why [`generate`] failed.
#[napi(object)]
pub struct GenerateError {
    /// The error message.
    pub message: String,
    /// Every unresolved `$ref`, for an unresolved-ref failure.
    pub pointers: Option<Vec<String>>,
    /// Every lint problem, for a lint failure.
    pub problems: Option<Vec<Problem>>,
}

/// One lint problem.
#[napi(object)]
pub struct Problem {
    /// The problem as Redocly prints it.
    pub message: String,
    /// Whether it is an error rather than a warning.
    pub error: bool,
}

/// Generates TypeScript from `text`, a string or a `Buffer` of UTF-8 decoded as `buffer.toString()`.
///
/// `format` is `json`, `yaml` or `auto` (JSON when the text starts with `{`). `location` is the
/// document's absolute path or URL (for text without one, the working directory plus `/`, as
/// openapi-typescript does); relative `$ref`s resolve against it. `options` is a JSON
/// object with openapi-typescript's option names plus `lint` (`{ oas3_0, oas3_1 }` rule maps) and
/// `fix` (`"all"` or fix names). `host` must have `load(url)` for `http(s)` documents, returning
/// `{ text, mimeType }` or `null`; it may have `document(json)` and the hooks `transform`,
/// `postTransform` and `transformProperty`, which are only called when present.
///
/// Deep nesting must hit the parser's depth limit rather than the stack's: natively the
/// document is generated on a thread with a big stack and host calls come back to this thread;
/// in WebAssembly the stack size is set at link time.
#[napi]
pub fn generate(
    text: Either<String, BufferSlice>,
    format: String,
    location: String,
    options: String,
    host: Object,
) -> Result<Generated> {
    let js = JsHost::new(&host)?;
    let text = match text {
        Either::A(text) => text,
        Either::B(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
    };
    let hooks = Hooks {
        transform: js.transform.is_some(),
        post_transform: js.post_transform.is_some(),
        transform_property: js.transform_property.is_some(),
    };
    on_big_stack(&js, hooks, Input { text, format, location, options })
}

struct Input {
    text: String,
    format: String,
    location: String,
    options: String,
}

#[cfg(target_family = "wasm")]
#[expect(clippy::unnecessary_wraps, reason = "matches the native version")]
fn on_big_stack(js: &JsHost, hooks: Hooks, input: Input) -> Result<Generated> {
    Ok(run(input, &mut ProxyHost::new(|request| js.answer(request), hooks)))
}

#[cfg(not(target_family = "wasm"))]
fn on_big_stack(js: &JsHost, hooks: Hooks, input: Input) -> Result<Generated> {
    use std::sync::mpsc::channel;

    enum Message {
        Call(Request),
        Done(Generated),
    }

    let (calls, requests) = channel();
    let (answer, answers) = channel();
    let worker = std::thread::Builder::new().stack_size(256 << 20).spawn(move || {
        let done = calls.clone();
        let ask = move |request| {
            calls.send(Message::Call(request)).map_err(|_| "internal error".to_owned())?;
            answers.recv().map_err(|_| "internal error".to_owned())?
        };
        let _ = done.send(Message::Done(run(input, &mut ProxyHost::new(ask, hooks))));
    });
    let worker = worker.map_err(|e| napi::Error::from_reason(format!("could not start worker thread: {e}")))?;
    let result = loop {
        match requests.recv() {
            Ok(Message::Call(request)) => {
                if answer.send(js.answer(request)).is_err() {
                    break failed("internal error".into());
                }
            }
            Ok(Message::Done(result)) => break result,
            Err(_) => break failed("internal error".into()),
        }
    };
    let _ = worker.join();
    Ok(result)
}

fn run<Ask: FnMut(Request) -> std::result::Result<Answer, String>>(
    input: Input,
    host: &mut ProxyHost<Ask>,
) -> Generated {
    let Input { text, format, location, options } = input;
    let options = match parse_options(&options) {
        Ok(o) => o,
        Err(e) => return failed(e),
    };
    let format = match format.as_str() {
        "json" => Format::Json,
        "yaml" => Format::Yaml,
        _ => Format::Auto,
    };
    let document = parse_document(&text, format);
    drop(text);
    let output = document.and_then(|d| generate_with_host(d, Some(&location), &options, host));
    match output {
        Ok(output) => Generated { code: Some(output.code), warnings: output.warnings, error: None },
        Err(e) => Generated { code: None, warnings: Vec::new(), error: Some(core_error(&e)) },
    }
}

fn failed(message: String) -> Generated {
    Generated {
        code: None,
        warnings: Vec::new(),
        error: Some(GenerateError { message, pointers: None, problems: None }),
    }
}

fn core_error(error: &Error) -> GenerateError {
    let mut out = GenerateError { message: error.to_string(), pointers: None, problems: None };
    match error {
        Error::UnresolvedRefs { pointer, earlier } => {
            out.pointers = Some(earlier.iter().chain([pointer]).map(ToString::to_string).collect());
        }
        Error::Lint(problems) => {
            out.problems = Some(problems.iter().map(|p| Problem { message: p.to_string(), error: p.error }).collect());
        }
        _ => {}
    }
    out
}

fn parse_options(json: &str) -> std::result::Result<Options, String> {
    let serde_json::Value::Object(map) = serde_json::from_str(json).map_err(|e| e.to_string())? else {
        return Err("options must be an object".into());
    };
    let mut o = Options::default();
    for (key, value) in &map {
        let flag = match key.as_str() {
            "additionalProperties" => &mut o.additional_properties,
            "alphabetize" => &mut o.alphabetize,
            "arrayLength" => &mut o.array_length,
            "defaultNonNullable" => &mut o.default_non_nullable,
            "emptyObjectsUnknown" => &mut o.empty_objects_unknown,
            "enum" => &mut o.enum_,
            "enumValues" => &mut o.enum_values,
            "conditionalEnums" => &mut o.conditional_enums,
            "dedupeEnums" => &mut o.dedupe_enums,
            "excludeDeprecated" => &mut o.exclude_deprecated,
            "exportType" => &mut o.export_type,
            "immutable" => &mut o.immutable,
            "rootTypes" => &mut o.root_types,
            "rootTypesNoSchemaPrefix" => &mut o.root_types_no_schema_prefix,
            "rootTypesKeepCasing" => &mut o.root_types_keep_casing,
            "pathParamsAsTypes" => &mut o.path_params_as_types,
            "propertiesRequiredByDefault" => &mut o.properties_required_by_default,
            "makePathsEnum" => &mut o.make_paths_enum,
            "generatePathParams" => &mut o.generate_path_params,
            "readWriteMarkers" => &mut o.read_write_markers,
            "inject" => {
                o.inject = value.as_str().map(str::to_owned);
                continue;
            }
            "fix" => {
                match value {
                    serde_json::Value::String(name) => o.fix.enable(name).map_err(|e| e.to_string())?,
                    serde_json::Value::Array(names) => {
                        for name in names {
                            o.fix.enable(name.as_str().unwrap_or_default()).map_err(|e| e.to_string())?;
                        }
                    }
                    _ => return Err("option \"fix\" must be \"all\" or an array of fix names".into()),
                }
                continue;
            }
            "lint" => {
                o.lint = lint_config(value);
                continue;
            }
            _ => continue,
        };
        *flag = js_truthy(value);
    }
    Ok(o)
}

/// openapi-typescript reads every boolean option as `options.x ?? default`, then tests it
/// for truthiness.
fn js_truthy(v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::Null => false,
        serde_json::Value::Bool(b) => *b,
        serde_json::Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.0),
        serde_json::Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

fn lint_config(value: &serde_json::Value) -> LintConfig {
    let rules =
        |key: &str| match parse_document(&value.get(key).map(ToString::to_string).unwrap_or_default(), Format::Json) {
            Ok(Value::Object(map)) => map,
            _ => spectype_core::value::Map::new(),
        };
    LintConfig { oas3_0: rules("oas3_0"), oas3_1: rules("oas3_1"), ..LintConfig::default() }
}

/// A schema as JS receives it: its index in the document the JS side parsed, or its JSON when it
/// is not part of it.
type Schema = Either<u32, String>;

/// A host call the worker thread asks the JS thread to make.
enum Request {
    Load(String),
    Document { document: String, discriminators: String },
    Transform { schema: Schema, path: String, component: Option<Schema> },
    PostTransform { text: String, path: String, component: Option<Schema> },
    TransformProperty { input: PropertyInput, schema: Schema, path: String, component: Option<Schema> },
}

/// What a host call returned; `None` for `null` or `undefined`.
enum Answer {
    None,
    Loaded(LoadedDocument),
    Type { text: String, kind: Option<String>, question_token: bool },
    Text(String),
}

/// The property `transformProperty` receives, before JS turns it into a TypeScript node.
#[napi(object)]
pub struct PropertyInput {
    /// `identifier`, `string` or `numeric`.
    pub name_kind: String,
    /// The property name.
    pub name: String,
    /// Whether the property is `readonly`.
    pub readonly: bool,
    /// Whether the property is optional.
    pub optional: bool,
    /// The property's type source.
    pub r#type: String,
    /// The `JSDoc` comment to attach after the hook.
    pub jsdoc: Option<String>,
}

#[derive(Clone, Copy)]
struct Hooks {
    transform: bool,
    post_transform: bool,
    transform_property: bool,
}

/// The core's [`Host`]: local files are read here, everything else goes to JS through `ask`.
/// Schema objects cross over by index into the document the JS side parsed once (see
/// [`ProxyHost::begin`]), so hooks see shared objects as upstream's do.
struct ProxyHost<Ask> {
    ask: Ask,
    hooks: Hooks,
    /// Address of every object and array in the bundled document → its pre-order index.
    index: Option<FxHashMap<usize, u32>>,
}

impl<Ask: FnMut(Request) -> std::result::Result<Answer, String>> ProxyHost<Ask> {
    const fn new(ask: Ask, hooks: Hooks) -> Self {
        Self { ask, hooks, index: None }
    }

    fn ask(&mut self, request: Request) -> std::result::Result<Answer, HostError> {
        (self.ask)(request).map_err(HostError::new)
    }

    /// Hands the bundled document to JS before the first hook runs, numbering its objects and
    /// arrays in the order `JSON.parse` output walks with `Object.keys`.
    fn begin(&mut self, ctx: &HookContext) -> std::result::Result<(), HostError> {
        if self.index.is_some() {
            return Ok(());
        }
        let mut index = FxHashMap::default();
        number(ctx.document, &mut index);
        self.index = Some(index);
        let document = ctx.document.json_stringify(0);
        self.ask(Request::Document { document, discriminators: ctx.discriminators.to_json() }).map(drop)
    }

    fn value(&self, v: &Value) -> Schema {
        match self.index.as_ref().and_then(|i| i.get(&std::ptr::from_ref(v).addr())) {
            Some(i) => Either::A(*i),
            None => Either::B(v.json_stringify(0)),
        }
    }

    fn context(&mut self, ctx: &HookContext) -> std::result::Result<(String, Option<Schema>), HostError> {
        self.begin(ctx)?;
        Ok((ctx.path.as_str().to_owned(), ctx.schema.map(|s| self.value(s))))
    }
}

fn number(v: &Value, index: &mut FxHashMap<usize, u32>) {
    let children: Box<dyn Iterator<Item = &Value>> = match v {
        Value::Object(map) => Box::new(map.values()),
        Value::Array(items) => Box::new(items.iter()),
        _ => return,
    };
    index.insert(std::ptr::from_ref(v).addr(), u32::try_from(index.len()).unwrap_or(u32::MAX));
    for child in children {
        number(child, index);
    }
}

/// A type a hook returned: `{ text, kind }` with `kind` one of the [`TypeKind`] names.
fn returned_type(text: String, kind: Option<&str>) -> Ty {
    let kind = match kind {
        Some("keyword") => TypeKind::Keyword,
        Some("union") => TypeKind::Union,
        Some("intersection") => TypeKind::Intersection,
        Some("function") => TypeKind::Function,
        Some("typeOperator") => TypeKind::TypeOperator,
        Some("query") => TypeKind::Query,
        Some("array") => TypeKind::Array,
        Some("tuple") => TypeKind::Tuple,
        _ => TypeKind::Other,
    };
    Ty::source(text, kind)
}

impl<Ask: FnMut(Request) -> std::result::Result<Answer, String>> Host for ProxyHost<Ask> {
    fn load(&mut self, absolute_ref: &str) -> std::result::Result<Option<LoadedDocument>, HostError> {
        if !absolute_ref.starts_with("http://") && !absolute_ref.starts_with("https://") {
            return load_file(absolute_ref);
        }
        match self.ask(Request::Load(absolute_ref.to_owned()))? {
            Answer::Loaded(doc) => Ok(Some(doc)),
            _ => Ok(None),
        }
    }

    fn transform(
        &mut self,
        schema: &Value,
        ctx: &HookContext,
    ) -> std::result::Result<Option<TransformResult>, HostError> {
        if !self.hooks.transform {
            return Ok(None);
        }
        let (path, component) = self.context(ctx)?;
        let schema = self.value(schema);
        match self.ask(Request::Transform { schema, path, component })? {
            Answer::Type { text, kind, question_token } => {
                Ok(Some(TransformResult { ty: returned_type(text, kind.as_deref()), question_token }))
            }
            _ => Ok(None),
        }
    }

    fn post_transform(&mut self, ty: &Ty, ctx: &HookContext) -> std::result::Result<Option<Ty>, HostError> {
        if !self.hooks.post_transform {
            return Ok(None);
        }
        let (path, component) = self.context(ctx)?;
        match self.ask(Request::PostTransform { text: print_type(ty), path, component })? {
            Answer::Type { text, kind, .. } => Ok(Some(returned_type(text, kind.as_deref()))),
            _ => Ok(None),
        }
    }

    fn transform_property(
        &mut self,
        property: &PropertyHookInput,
        schema: &Value,
        ctx: &HookContext,
    ) -> std::result::Result<Option<String>, HostError> {
        if !self.hooks.transform_property {
            return Ok(None);
        }
        let (path, component) = self.context(ctx)?;
        let (name_kind, name) = match property.name {
            PropName::Ident(s) => ("identifier", s),
            PropName::Str(s) => ("string", s),
            PropName::Num(s) => ("numeric", s),
        };
        let input = PropertyInput {
            name_kind: name_kind.to_owned(),
            name: name.clone(),
            readonly: property.readonly,
            optional: property.optional,
            r#type: property.type_source(),
            jsdoc: property.jsdoc.map(str::to_owned),
        };
        let schema = self.value(schema);
        match self.ask(Request::TransformProperty { input, schema, path, component })? {
            Answer::Text(text) => Ok(Some(text)),
            _ => Ok(None),
        }
    }
}

/// The JS host object's methods, called on the JS thread.
struct JsHost<'env> {
    load: Function<'env, String, Unknown<'env>>,
    document: Hook<'env, (String, String)>,
    transform: Hook<'env, (Schema, String, Option<Schema>)>,
    post_transform: Hook<'env, (String, String, Option<Schema>)>,
    transform_property: Hook<'env, (PropertyInput, Schema, String, Option<Schema>)>,
}

type Hook<'env, Args> = Option<Function<'env, FnArgs<Args>, Unknown<'env>>>;

fn method<'env, A, R>(host: &Object<'env>, name: &str) -> Result<Option<Function<'env, A, R>>>
where
    A: JsValuesTupleIntoVec,
    R: FromNapiValue,
{
    let value: Unknown = host.get_named_property(name)?;
    if value.get_type()? != ValueType::Function {
        return Ok(None);
    }
    host.get_named_property(name).map(Some)
}

impl<'env> JsHost<'env> {
    fn new(host: &Object<'env>) -> Result<Self> {
        Ok(Self {
            load: method(host, "load")?.ok_or_else(|| napi::Error::from_reason("host.load must be a function"))?,
            document: method(host, "document")?,
            transform: method(host, "transform")?,
            post_transform: method(host, "postTransform")?,
            transform_property: method(host, "transformProperty")?,
        })
    }

    fn answer(&self, request: Request) -> std::result::Result<Answer, String> {
        self.call(request).map_err(|e| e.reason)
    }

    fn call(&self, request: Request) -> Result<Answer> {
        match request {
            Request::Load(absolute_ref) => {
                let doc = self.load.call(absolute_ref)?;
                let Some(doc) = object(doc)? else { return Ok(Answer::None) };
                let text: Option<String> = doc.get("text")?;
                let mime_type: Option<String> = doc.get("mimeType")?;
                Ok(Answer::Loaded(LoadedDocument { text: text.unwrap_or_default(), mime_type }))
            }
            Request::Document { document, discriminators } => {
                let f = self
                    .document
                    .as_ref()
                    .ok_or_else(|| napi::Error::from_reason("host.document must be a function"))?;
                f.call((document, discriminators).into()).map(|_| Answer::None)
            }
            Request::Transform { schema, path, component } => {
                let Some(f) = &self.transform else { return Ok(Answer::None) };
                type_answer(f.call((schema, path, component).into())?)
            }
            Request::PostTransform { text, path, component } => {
                let Some(f) = &self.post_transform else { return Ok(Answer::None) };
                type_answer(f.call((text, path, component).into())?)
            }
            Request::TransformProperty { input, schema, path, component } => {
                let Some(f) = &self.transform_property else { return Ok(Answer::None) };
                let result = f.call((input, schema, path, component).into())?;
                if result.get_type()? != ValueType::String {
                    return Ok(Answer::None);
                }
                Ok(Answer::Text(result.coerce_to_string()?.into_utf8()?.into_owned()?))
            }
        }
    }
}

fn object(value: Unknown<'_>) -> Result<Option<Object<'_>>> {
    match value.get_type()? {
        ValueType::Undefined | ValueType::Null => Ok(None),
        _ => value.coerce_to_object().map(Some),
    }
}

fn type_answer(result: Unknown<'_>) -> Result<Answer> {
    let Some(result) = object(result)? else { return Ok(Answer::None) };
    let text: Option<String> = result.get("text")?;
    let kind: Option<String> = result.get("kind")?;
    let question_token: Option<Unknown> = result.get("questionToken")?;
    let question_token = match question_token {
        Some(v) => v.coerce_to_bool()?,
        None => false,
    };
    Ok(Answer::Type { text: text.unwrap_or_default(), kind, question_token })
}

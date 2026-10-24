//! The Node API hooks, checked against what openapi-typescript prints with the same callbacks
//! (`fixtures/hooks/upstream.mjs` produced the expected files).

use spectype_core::ts::{PropName, Ty, TypeKind, print_type};
use spectype_core::{
    Error, Format, HookContext, Host, HostError, Options, PropertyHookInput, TransformResult, Value,
    generate_with_host, parse_document,
};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/hooks/");

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("{FIXTURES}{name}")).unwrap()
}

#[derive(Default)]
struct TestHost {
    calls: Vec<String>,
}

const JSON_LITERAL: &str = "{\n    /**\n    \n     * two\n     */\n    a: string;\n    b?: number;\n}";

impl Host for TestHost {
    fn transform(&mut self, schema: &Value, ctx: &HookContext) -> Result<Option<TransformResult>, HostError> {
        self.calls.push(format!("transform {} {}", ctx.path, if ctx.schema.is_some() { "schema" } else { "-" }));
        Ok(match schema.get("format").and_then(Value::as_str) {
            Some("date-time") => Some(Ty::Ref("Date".into(), Vec::new()).into()),
            Some("date") => {
                Some(TransformResult { ty: Ty::source("Date | null", TypeKind::Union), question_token: true })
            }
            Some("json") => Some(Ty::source(JSON_LITERAL, TypeKind::Other).into()),
            _ => None,
        })
    }

    fn post_transform(&mut self, ty: &Ty, ctx: &HookContext) -> Result<Option<Ty>, HostError> {
        Ok((ctx.path == "#/components/schemas/Wrapped")
            .then(|| Ty::source(format!("{} | Wrapped", print_type(ty)), TypeKind::Union)))
    }

    fn transform_property(
        &mut self,
        property: &PropertyHookInput,
        _schema: &Value,
        ctx: &HookContext,
    ) -> Result<Option<String>, HostError> {
        Ok(matches!(property.name, PropName::Ident(n) if n == "id")
            .then(|| format!("// {}\n{}", ctx.path, property.source())))
    }
}

#[test]
fn hooks_match_upstream() {
    let doc = parse_document(&fixture("doc.json"), Format::Json).unwrap();
    let mut host = TestHost::default();
    let out = generate_with_host(doc, None, &Options::default(), &mut host).unwrap();
    let code = out.code.strip_prefix(spectype_core::COMMENT_HEADER).unwrap();
    assert_eq!(code, fixture("expected.ts"));
    assert_eq!(host.calls.join("\n") + "\n", fixture("transform-calls.txt"));
}

struct FailingHost;

impl Host for FailingHost {
    fn post_transform(&mut self, _: &Ty, _: &HookContext) -> Result<Option<Ty>, HostError> {
        Err(HostError::with_source("boom", std::io::Error::other("cause")))
    }
}

#[test]
fn hook_errors_abort() {
    use std::error::Error as _;
    let doc = parse_document(&fixture("doc.json"), Format::Json).unwrap();
    let err = generate_with_host(doc, None, &Options::default(), &mut FailingHost).unwrap_err();
    assert!(matches!(err, Error::Host(_)));
    assert_eq!(err.to_string(), "boom");
    let host_error = err.source().unwrap();
    assert_eq!(host_error.to_string(), "boom");
    assert_eq!(host_error.source().unwrap().to_string(), "cause");
}

#[test]
fn inject_goes_before_paths() {
    let doc = parse_document(r#"{"openapi":"3.1.0","info":{"title":"t","version":"1"}}"#, Format::Json).unwrap();
    let options = Options { inject: Some("\ntype Foo = string;\ntype Bar = number;\n".into()), ..Options::default() };
    let code = spectype_core::generate(doc, &options).unwrap().code;
    let body = code.strip_prefix(spectype_core::COMMENT_HEADER).unwrap();
    assert!(body.starts_with("type Foo = string;\ntype Bar = number;\nexport type paths = Record<string, never>;\n"));
}

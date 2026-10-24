//! What generation needs from its environment: loading referenced documents, and the
//! openapi-typescript Node API hooks (`transform`, `postTransform`, `transformProperty`).

use spectype_document::{HostError, JsonPointer, Value};
pub use spectype_redocly::{LoadedDocument, load_file};

use crate::discriminators::Discriminators;
use crate::ts::{Member, MemberKind, PropName, Ty, print_member, print_type};

/// What a `transform` hook returns: openapi-typescript's bare `ts.TypeNode` is
/// `question_token: false`, its `{ schema, questionToken }` object sets the flag.
#[derive(Debug, Clone, PartialEq)]
pub struct TransformResult {
    /// The type to use instead of the one the core would build.
    pub ty: Ty,
    /// In a schema: `ty | undefined`. On a property or component member: makes it optional.
    pub question_token: bool,
}

impl From<Ty> for TransformResult {
    fn from(ty: Ty) -> Self {
        Self { ty, question_token: false }
    }
}

/// openapi-typescript's `TransformNodeOptions` (minus `ctx`, whose options the host already has).
#[derive(Debug, Clone, Copy)]
pub struct HookContext<'a> {
    /// `options.path`: the JSON Pointer openapi-typescript passes, e.g. `#/components/schemas/Pet`.
    /// For the `transform` call on a property this is the pointer of the object owning it, as
    /// upstream passes its own `options` there.
    pub path: &'a JsonPointer,
    /// `options.schema`: the component being transformed (`#/components/<kind>/<name>`), the
    /// root `$defs` object, or `None` under `paths` and `webhooks`.
    pub schema: Option<&'a Value>,
    /// The bundled document, for `ctx.resolve`.
    pub document: &'a Value,
    /// `ctx.discriminators`.
    pub discriminators: &'a Discriminators,
}

/// The property signature a `transformProperty` hook receives.
#[derive(Debug, Clone, Copy)]
pub struct PropertyHookInput<'a> {
    /// The property name, as it will be printed.
    pub name: &'a PropName,
    /// Whether the property is `readonly`.
    pub readonly: bool,
    /// Whether the property is optional (`?`).
    pub optional: bool,
    /// The property's type.
    pub ty: &'a Ty,
    /// The `JSDoc` comment openapi-typescript attaches after the hook: the text between `/*` and
    /// `*/`. A hook that returns a member must print it as that member's last leading comment
    /// (`addSyntheticLeadingComment(node, MultiLineCommentTrivia, jsdoc, true)`), since the core
    /// does not touch returned text.
    pub jsdoc: Option<&'a str>,
}

impl PropertyHookInput<'_> {
    /// The property's type, printed at indent 0.
    #[must_use]
    pub fn type_source(&self) -> String {
        print_type(self.ty)
    }

    /// The member as the core would print it at indent 0, `JSDoc` included: what returning it
    /// unchanged produces.
    #[must_use]
    pub fn source(&self) -> String {
        print_member(&Member {
            comment: self.jsdoc.map(str::to_owned),
            kind: MemberKind::Property {
                readonly: self.readonly,
                name: self.name.clone(),
                optional: self.optional,
                ty: self.ty.clone(),
            },
        })
    }
}

/// What generation needs from its environment. Every method has a default, so a host only
/// implements what it changes. A hook's `Err` aborts generation with [`Error::Host`].
///
/// [`Error::Host`]: spectype_document::Error::Host
pub trait Host {
    /// Loads a document by absolute reference: a file path or an `http(s)` URL. `Ok(None)` means
    /// it does not exist. Either failure leaves the `$ref` unresolved, as with Redocly.
    fn load(&mut self, absolute_ref: &str) -> Result<Option<LoadedDocument>, HostError> {
        if absolute_ref.starts_with("http://") || absolute_ref.starts_with("https://") {
            return Err(HostError::new(format!("Failed to load {absolute_ref}: this host cannot fetch URLs")));
        }
        load_file(absolute_ref)
    }

    /// openapi-typescript's `transform(schemaObject, options)`. Called for every schema with a
    /// truthy `type` before its type is built, for every object property (with the property's
    /// schema, possibly a `$ref` or a boolean), and for every component of every kind.
    fn transform(&mut self, schema: &Value, ctx: &HookContext) -> Result<Option<TransformResult>, HostError> {
        let _ = (schema, ctx);
        Ok(None)
    }

    /// openapi-typescript's `postTransform(type, options)`, run on the type built for every schema
    /// object. `Some` replaces it. Use [`print_type`] for its source.
    fn post_transform(&mut self, ty: &Ty, ctx: &HookContext) -> Result<Option<Ty>, HostError> {
        let _ = (ty, ctx);
        Ok(None)
    }

    /// openapi-typescript's `transformProperty(property, schemaObject, options)`, run on each
    /// object property and `$defs` entry. `Some` replaces the member with this TypeScript source
    /// (leading comments and `;` included), printed at indent 0. See
    /// [`PropertyHookInput::jsdoc`].
    fn transform_property(
        &mut self,
        property: &PropertyHookInput,
        schema: &Value,
        ctx: &HookContext,
    ) -> Result<Option<String>, HostError> {
        let _ = (property, schema, ctx);
        Ok(None)
    }
}

/// The default [`Host`]: files from disk, no remote URLs, no hooks.
#[derive(Debug, Default, Clone, Copy)]
pub struct FsHost;

impl Host for FsHost {}

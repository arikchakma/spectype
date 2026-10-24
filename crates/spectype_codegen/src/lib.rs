//! Stage 3 of spectype: turning a bundled document into TypeScript. The transforms port
//! openapi-typescript's `transform/*`, build a small TS AST ([`ts`]) and print it the way the
//! TypeScript printer does. [`Host`] carries the Node API hooks they call.

mod discriminators;
pub mod host;
mod names;
mod transform;
pub mod ts;

pub use discriminators::{Discriminators, scan as scan_discriminators};
pub use host::{FsHost, HookContext, Host, LoadedDocument, PropertyHookInput, TransformResult, load_file};
use spectype_redocly::lint;
pub use transform::transform_document;

/// Generation options. Field names follow openapi-typescript's Node API.
#[derive(Debug, Clone)]
pub struct Options {
    pub additional_properties: bool,
    pub alphabetize: bool,
    pub array_length: bool,
    pub default_non_nullable: bool,
    pub empty_objects_unknown: bool,
    pub enum_: bool,
    pub enum_values: bool,
    pub conditional_enums: bool,
    pub dedupe_enums: bool,
    pub exclude_deprecated: bool,
    pub export_type: bool,
    pub immutable: bool,
    pub root_types: bool,
    pub root_types_no_schema_prefix: bool,
    pub root_types_keep_casing: bool,
    pub path_params_as_types: bool,
    pub properties_required_by_default: bool,
    pub make_paths_enum: bool,
    pub generate_path_params: bool,
    pub read_write_markers: bool,
    /// Redocly lint rules to run before bundling (from `redocly.yaml`); none by default.
    pub lint: lint::Config,
    /// TypeScript inserted after the `--read-write-markers` helpers and before `paths`.
    /// openapi-typescript reprints it through the TS printer; it is inserted here as given
    /// (trimmed), so pass printer-formatted text for identical output.
    pub inject: Option<String>,
    /// Opt-in fixes for openapi-typescript bugs; all off keeps output byte-identical.
    pub fix: Fixes,
}

/// A fix for an openapi-typescript bug. See `COMPAT.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Fix {
    /// `--array-length`: tuples of length `minItems..=maxItems` instead of `0..=max-min`.
    ArrayLength,
    /// `--enum-values`: type arrays by their values instead of a `FlattenedDeepRequired` lookup
    /// that fails to compile on many real specs.
    EnumValuesType,
    /// `--root-types`: unique, valid alias names that do not shadow names the file uses.
    RootTypesNames,
    /// `--enum`: unique, valid member and enum names; `null` allowed in enums.
    EnumNames,
    /// `operations` keys and their `paths` references agree for operationIds with `%xx`.
    OperationIdRefs,
    /// `--default-non-nullable` exempts request inputs by location, not by pointer substring.
    DefaultNonNullableScope,
    /// A `const` tuple used as `items` keeps its outer array.
    ConstItems,
    /// An `allOf` member that is an object with no properties no longer makes its siblings
    /// `never`, and its `required` applies to them.
    AllOfEmptyObjects,
    /// A schema that extends a `oneOf` discriminator base with `allOf`, without being one of its
    /// variants, keeps the union instead of becoming a new variant.
    DiscriminatorOneOfBase,
    /// A parent's `required` applies to an `allOf` `$ref` even when a discriminator mapping
    /// patched the target.
    AllOfRequired,
    /// A schema with its own discriminator keeps it when it extends another discriminator base.
    NestedDiscriminators,
    /// A `oneOf` variant without a mapping entry keeps the `enum` or `const` it declares for the
    /// discriminator property instead of getting its schema name.
    DiscriminatorDeclaredValues,
    /// `anyOf` next to `properties` or `allOf` intersects with them, as `oneOf` does.
    AnyOfProperties,
    /// A schema's `$defs` is an optional property, and `$ref`s into it go through `NonNullable`.
    DefsOptional,
    /// A `prefixItems` tuple allows the extra items that `items` describes, unless `items: false`,
    /// `unevaluatedItems: false` or `maxItems` closes it.
    PrefixItems,
}

impl Fix {
    /// Every fix, in the order `--help` lists them.
    pub const ALL: [Self; 15] = [
        Self::ArrayLength,
        Self::EnumValuesType,
        Self::RootTypesNames,
        Self::EnumNames,
        Self::OperationIdRefs,
        Self::DefaultNonNullableScope,
        Self::ConstItems,
        Self::AllOfEmptyObjects,
        Self::DiscriminatorOneOfBase,
        Self::AllOfRequired,
        Self::NestedDiscriminators,
        Self::DiscriminatorDeclaredValues,
        Self::AnyOfProperties,
        Self::DefsOptional,
        Self::PrefixItems,
    ];

    /// The name `--fix` takes.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ArrayLength => "array-length",
            Self::EnumValuesType => "enum-values-type",
            Self::RootTypesNames => "root-types-names",
            Self::EnumNames => "enum-names",
            Self::OperationIdRefs => "operation-id-refs",
            Self::DefaultNonNullableScope => "default-non-nullable-scope",
            Self::ConstItems => "const-items",
            Self::AllOfEmptyObjects => "allof-empty-objects",
            Self::DiscriminatorOneOfBase => "discriminator-oneof-base",
            Self::AllOfRequired => "allof-required",
            Self::NestedDiscriminators => "nested-discriminators",
            Self::DiscriminatorDeclaredValues => "discriminator-declared-values",
            Self::AnyOfProperties => "anyof-properties",
            Self::DefsOptional => "defs-optional",
            Self::PrefixItems => "prefix-items",
        }
    }

    const fn bit(self) -> u16 {
        1 << self as u8
    }
}

impl std::str::FromStr for Fix {
    type Err = UnknownFix;

    fn from_str(name: &str) -> std::result::Result<Self, UnknownFix> {
        Self::ALL.into_iter().find(|f| f.name() == name).ok_or_else(|| UnknownFix(name.to_owned()))
    }
}

/// A set of [`Fix`]es, empty by default so output stays byte-identical.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Fixes(u16);

/// A name [`Fixes::enable`] does not know, carrying that name.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown fix \"{0}\"; expected all or one of: {names}", names = Fix::ALL.map(Fix::name).join(", "))]
pub struct UnknownFix(pub String);

impl Fixes {
    /// Every fix turned on.
    #[must_use]
    pub const fn all() -> Self {
        Self((1 << Fix::ALL.len()) - 1)
    }

    /// Whether `fix` is turned on.
    #[must_use]
    pub const fn contains(self, fix: Fix) -> bool {
        self.0 & fix.bit() != 0
    }

    /// Turns on `fix`.
    pub const fn insert(&mut self, fix: Fix) {
        self.0 |= fix.bit();
    }

    /// Turns on a fix by name (`all` turns on every fix).
    pub fn enable(&mut self, name: &str) -> std::result::Result<(), UnknownFix> {
        if name == "all" {
            *self = Self::all();
        } else {
            self.insert(name.parse()?);
        }
        Ok(())
    }
}

impl FromIterator<Fix> for Fixes {
    fn from_iter<I: IntoIterator<Item = Fix>>(iter: I) -> Self {
        let mut fixes = Self::default();
        for fix in iter {
            fixes.insert(fix);
        }
        fixes
    }
}

impl Default for Options {
    fn default() -> Self {
        Self {
            additional_properties: false,
            alphabetize: false,
            array_length: false,
            default_non_nullable: true,
            empty_objects_unknown: false,
            enum_: false,
            enum_values: false,
            conditional_enums: false,
            dedupe_enums: false,
            exclude_deprecated: false,
            export_type: false,
            immutable: false,
            root_types: false,
            root_types_no_schema_prefix: false,
            root_types_keep_casing: false,
            path_params_as_types: false,
            properties_required_by_default: false,
            make_paths_enum: false,
            generate_path_params: false,
            read_write_markers: false,
            lint: lint::Config::default(),
            inject: None,
            fix: Fixes::default(),
        }
    }
}

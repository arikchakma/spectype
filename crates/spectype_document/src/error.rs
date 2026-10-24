use std::fmt;

use crate::pointer::JsonPointer;

/// A generation failure, carrying the JSON Pointer of the offending node when there is one.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The document is not valid JSON or YAML.
    #[error("{0}")]
    Parse(String),
    /// A node openapi-typescript rejects while transforming.
    #[error("{message} (at {pointer})")]
    Invalid {
        /// Where the node is.
        pointer: JsonPointer,
        /// What is wrong with it.
        message: String,
    },
    /// The document (or what bundling made of it) cannot be generated from.
    #[error("{0}")]
    Unsupported(String),
    /// `pointer` is the error openapi-typescript throws; `earlier` are the ones it logs first.
    #[error("Can't resolve $ref at {pointer}")]
    UnresolvedRefs {
        /// The last `$ref` that does not resolve.
        pointer: JsonPointer,
        /// The ones before it, in walk order.
        earlier: Vec<JsonPointer>,
    },
    /// Lint problems, in report order; openapi-typescript throws the last error among them.
    #[error("{}", last_error(.0))]
    Lint(Vec<Problem>),
    /// A host hook failed.
    #[error("{0}")]
    Host(#[from] HostError),
}

impl Error {
    /// An [`Error::Invalid`] at `pointer`.
    pub fn at(pointer: impl Into<JsonPointer>, message: impl Into<String>) -> Self {
        Self::Invalid { pointer: pointer.into(), message: message.into() }
    }
}

fn last_error(problems: &[Problem]) -> String {
    problems.iter().rev().find(|p| p.error).map(ToString::to_string).unwrap_or_default()
}

/// `Result` with [`Error`] as the default error.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// A failure in the environment generation runs in: a document loader or a Node API hook.
///
/// It displays as its message alone; the underlying error, if any, is its
/// [`source`](std::error::Error::source).
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct HostError {
    message: String,
    #[source]
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl HostError {
    /// A failure described by `message` alone.
    pub fn new(message: impl Into<String>) -> Self {
        Self { message: message.into(), source: None }
    }

    /// A failure described by `message` and caused by `source`.
    pub fn with_source(
        message: impl Into<String>,
        source: impl Into<Box<dyn std::error::Error + Send + Sync>>,
    ) -> Self {
        Self { message: message.into(), source: Some(source.into()) }
    }
}

impl From<String> for HostError {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

impl From<&str> for HostError {
    fn from(message: &str) -> Self {
        Self::new(message)
    }
}

/// A lint finding, printed by openapi-typescript as `<message> at <pointer>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// The rule id, `bundler` for the bundler's own problems.
    pub rule: String,
    /// `error` severity; everything else prints as a warning.
    pub error: bool,
    /// The message, without the location.
    pub message: String,
    /// Where the problem is, when it has a location.
    pub pointer: Option<JsonPointer>,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.pointer {
            Some(p) => write!(f, "{} at {p}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

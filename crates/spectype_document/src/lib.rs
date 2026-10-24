//! Stage 1 of spectype: reading `OpenAPI` documents. JSON/YAML parsing with the semantics
//! openapi-typescript gets from `JSON.parse` and js-yaml, the [`Value`] model with JavaScript
//! object rules, JSON Pointer utilities, the `OpenAPI` vocabulary enums ([`oas`]), and the
//! [`Error`] every stage returns.

mod error;
pub mod oas;
pub mod parse;
pub mod pointer;
pub mod value;

pub use error::{Error, HostError, Problem, Result};
pub use parse::{Format, parse_document};
pub use pointer::JsonPointer;
pub use value::Value;

//! Stage 2 of spectype: what openapi-typescript gets from Redocly. The lint engine
//! (`redocly.yaml` rules) and the bundler that resolves `$ref`s into one document share the
//! OAS3 type tree and resolver, and lint runs between resolving and bundling, so they live in
//! one crate.

mod bundle;
pub mod http;
pub mod lint;
mod load;

pub use bundle::bundle;
pub use load::{Load, LoadedDocument, load_file};

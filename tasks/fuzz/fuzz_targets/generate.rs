#![no_main]

use libfuzzer_sys::fuzz_target;
use spectype_core::{Format, Options, generate, parse_document};

// Wraps arbitrary JSON into the places a schema, path item or component can appear, so the
// resolver and transforms see adversarial shapes rather than mostly failing the version check.
fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else { return };
    let Ok(fragment) = parse_document(text, Format::Json) else { return };
    let fragment = serde_json_like(&fragment);
    let docs = [
        format!(r##"{{"openapi":"3.1.0","info":{{"title":"f","version":"1"}},"components":{{"schemas":{{"A":{fragment},"B":{{"$ref":"#/components/schemas/A"}}}}}}}}"##),
        format!(r##"{{"openapi":"3.0.3","info":{{"title":"f","version":"1"}},"paths":{{"/x/{{id}}":{fragment}}}}}"##),
        format!(r##"{{"openapi":"3.1.0","info":{{"title":"f","version":"1"}},"components":{fragment}}}"##),
        text.to_owned(),
    ];
    let mut all = Options::default();
    all.enum_values = true;
    all.root_types = true;
    all.root_types_no_schema_prefix = true;
    let mut other = Options::default();
    other.enum_ = true;
    other.dedupe_enums = true;
    other.alphabetize = true;
    other.exclude_deprecated = true;
    other.immutable = true;
    other.array_length = true;
    other.path_params_as_types = true;
    other.read_write_markers = true;
    other.make_paths_enum = true;
    other.generate_path_params = true;
    for doc in &docs {
        let Ok(value) = parse_document(doc, Format::Json) else { continue };
        for options in [&Options::default(), &all, &other] {
            let _ = generate(value.clone(), options);
        }
    }
});

fn serde_json_like(v: &spectype_core::Value) -> String {
    v.json_stringify(0)
}

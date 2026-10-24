#![no_main]

use libfuzzer_sys::fuzz_target;
use spectype_core::{Format, parse_document};

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        for format in [Format::Json, Format::Yaml, Format::Auto] {
            let _ = parse_document(text, format);
        }
    }
});

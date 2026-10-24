#![expect(clippy::print_stdout, reason = "prints timings")]

use std::time::Instant;

use spectype_document::{Format, parse_document};

fn main() {
    for path in std::env::args().skip(1) {
        let text = std::fs::read_to_string(&path).expect("read");
        let best = (0..5)
            .map(|_| {
                let t = Instant::now();
                let v = parse_document(&text, Format::Yaml).expect("parse");
                let e = t.elapsed();
                drop(v);
                e
            })
            .min()
            .unwrap_or_default();
        println!("{path}: {:.1} ms", best.as_secs_f64() * 1000.0);
    }
}

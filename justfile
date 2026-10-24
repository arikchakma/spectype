_default:
    @just --list -u

# Everything CI checks on the Rust side; run before pushing.
ready:
    just fmt
    cargo check --workspace --all-targets --all-features --locked
    cargo test --workspace
    just lint
    just doc
    cargo coverage
    just allocs
    git diff --exit-code

fmt:
    cargo fmt --all

lint:
    cargo lint

doc:
    RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --document-private-items

typos:
    typos

deny:
    cargo deny check

shear:
    cargo shear

msrv:
    cargo +1.88 check --workspace --all-targets --locked

coverage:
    cargo coverage

allocs:
    cargo allocs

bench *args:
    cargo build --release -p spectype
    cd tasks/benchmark && ([ -d node_modules ] || npm ci) && node run.mjs {{args}}

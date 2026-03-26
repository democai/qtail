default:
    just --list

build:
    cargo build

release:
    cargo build --release

test:
    cargo test

lint:
    cargo clippy -- -D warnings

fmt:
    cargo fmt --check

check: fmt lint test

cpu-regression:
    cargo build
    cargo test --test cpu_regression -- --ignored --nocapture

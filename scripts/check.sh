#!/bin/sh
# Contrôle exigé avant chaque commit : format, lints sans avertissement, tests.
set -eu
cd "$(dirname "$0")/.."
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

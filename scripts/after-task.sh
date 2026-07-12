#!/usr/bin/env sh
set -eu

cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
sh ./scripts/security-post-action.sh

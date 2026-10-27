#!/usr/bin/env bash
# Checks that production code cannot change a Map except through a Command (ADR 0004).
#
# `Map::apply`, `execute` and `revert` exist only with the `test-support` feature, which the
# tests and benches turn on through dev-dependencies. A workspace-wide `cargo clippy
# --all-targets` builds those with the feature on, and Cargo then unifies it into every crate in
# that build, so it cannot catch a stray call in app or render code. Building only the
# libraries and binaries leaves the feature off.
set -euo pipefail

cd "$(dirname "$0")/.."
cargo check --workspace --lib --bins --locked
echo "Production code reaches a Map only through Commands."

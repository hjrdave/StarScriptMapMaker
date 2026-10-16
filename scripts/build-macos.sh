#!/usr/bin/env bash
# Builds the unsigned macOS arm64 .dmg and .app locally, with the same cargo-packager
# step and version as .github/workflows/release.yml. Not a release: no tag, no upload.
#
#   scripts/build-macos.sh
set -euo pipefail

# Keep in step with CARGO_PACKAGER_VERSION in .github/workflows/release.yml.
packager_version=0.11.8

if [[ $(uname -s) != Darwin ]]; then
  echo "error: build-macos.sh only runs on macOS (found $(uname -s))." >&2
  exit 1
fi
if [[ $(uname -m) != arm64 ]]; then
  echo "error: only Apple Silicon (arm64) is supported (found $(uname -m))." >&2
  exit 1
fi
command -v cargo >/dev/null || { echo "error: cargo not found; install Rust from https://rustup.rs" >&2; exit 1; }

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

installed=$(cargo packager --version 2>/dev/null || true)
if [[ $installed != *"$packager_version"* ]]; then
  cargo install cargo-packager --version "$packager_version" --locked
fi

(cd apps/starscript && cargo packager --release --formats dmg)

packages=$root/target/packages
echo
echo "Built in $packages:"
found=0
for item in "$packages"/*.dmg "$packages"/*.app; do
  [[ -e $item ]] && { echo "  $item"; found=1; }
done
if [[ $found == 0 ]]; then
  echo "error: no .dmg or .app found in $packages" >&2
  exit 1
fi
echo
echo "The build is unsigned; see docs/release/macos-local-build.md for first launch."

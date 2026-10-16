#!/usr/bin/env bash
# Fails if a default build carries the frame-stage spans (issue #309): `tracing` must be reached
# only through the `profiling` feature of ssmap-render and the app, which no default build turns on.
set -euo pipefail

for package in app ssmap-render; do
  direct=$(cargo tree -p "$package" -e normal --depth 1 --prefix none --format '{p}' \
    | awk '{print $1}' | grep -x tracing || true)
  if [[ -n "$direct" ]]; then
    echo "error: a default build of $package depends on tracing directly." >&2
    exit 1
  fi
  if cargo tree -p "$package" -e features --prefix none \
    | grep -Eq '^(app|ssmap-render) feature "profiling"'; then
    echo "error: a default build of $package enables the profiling feature." >&2
    exit 1
  fi
done

# The span marker is a string in the binary only when the spans are compiled in.
if [[ "${1:-}" == "--binary" ]]; then
  cargo build --release --locked -p app
  if grep -aq ssmap_stage target/release/starscript-map-maker; then
    echo "error: the release binary contains frame-stage spans." >&2
    exit 1
  fi
fi
echo "Default builds carry no frame-stage spans."

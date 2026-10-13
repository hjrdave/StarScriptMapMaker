#!/usr/bin/env bash
# Fails if starscript-core depends on UI or GPU crates, directly or
# transitively. The core crate must stay headless (ADR 0001).
set -euo pipefail

forbidden='^(egui.*|eframe|epaint.*|emath|ecolor|wgpu.*|winit|naga)$'

deps=$(cargo tree -p starscript-core -e normal,build --prefix none --format '{p}' \
    | awk '{print $1}' | sort -u)

if bad=$(grep -E "$forbidden" <<<"$deps"); then
    echo "starscript-core must not depend on UI or GPU crates, but depends on:" >&2
    echo "$bad" >&2
    exit 1
fi
echo "starscript-core has no UI or GPU dependencies."

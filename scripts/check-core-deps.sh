#!/usr/bin/env bash
# Fails if crates/core depends, directly or transitively, on egui or wgpu (ADR 0001).
set -euo pipefail

forbidden='^(egui|eframe|egui-[a-z-]+|egui_[a-z_]+|wgpu|wgpu-[a-z-]+)$'

offenders=$(cargo tree -p ssmap-core -e normal,build --prefix none --format '{p}' \
  | awk '{print $1}' | sort -u | grep -E "$forbidden" || true)

if [[ -n "$offenders" ]]; then
  echo "error: ssmap-core must not depend on egui or wgpu, but it pulls in:" >&2
  echo "$offenders" | sed 's/^/  /' >&2
  exit 1
fi
echo "ssmap-core has no egui or wgpu dependency."

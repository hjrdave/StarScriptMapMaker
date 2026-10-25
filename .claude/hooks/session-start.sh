#!/bin/bash
# Sets up a Claude Code cloud session so it can run fmt, clippy and the tests (#347).
# Local sessions use the developer's own setup, so this does nothing there.
set -euo pipefail

if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

cd "$CLAUDE_PROJECT_DIR"

# The pinned toolchain from rust-toolchain.toml, with rustfmt and clippy. Same step as CI.
rustup show active-toolchain || rustup toolchain install

cargo fetch --locked

# A full debug test build of the workspace is about 25 GB, more than the session's disk
# allowance. Without incremental data and full debug info it is about 8 GB.
if [ -n "${CLAUDE_ENV_FILE:-}" ]; then
  echo 'export CARGO_INCREMENTAL=0' >> "$CLAUDE_ENV_FILE"
  echo 'export CARGO_PROFILE_DEV_DEBUG=line-tables-only' >> "$CLAUDE_ENV_FILE"
fi

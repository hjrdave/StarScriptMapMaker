#!/bin/bash
# Sets up a Claude Code cloud session so it can run fmt, clippy and the tests (#347).
# Local sessions use the developer's own setup, so this does nothing there.
set -euo pipefail

if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

cd "$CLAUDE_PROJECT_DIR"

# The pinned toolchain from rust-toolchain.toml, with rustfmt, clippy and rust-analyzer (the LSP
# plugin in .claude/settings.json runs it). `rustup show` adds a component the toolchain is
# missing. Same step as CI.
rustup show active-toolchain || rustup toolchain install

cargo fetch --locked

# A full debug test build of the workspace is about 25 GB, more than the session's disk
# allowance. Without incremental data and full debug info it is about 8 GB.
if [ -n "${CLAUDE_ENV_FILE:-}" ]; then
  echo 'export CARGO_INCREMENTAL=0' >> "$CLAUDE_ENV_FILE"
  echo 'export CARGO_PROFILE_DEV_DEBUG=line-tables-only' >> "$CLAUDE_ENV_FILE"
fi

# Lavapipe, a software Vulkan driver, so the render tests run and compare golden images (#642).
# Optional, so it comes last and every way it can fail leaves the session working: the tests then
# skip. apt's output goes to a log because a SessionStart hook's stdout is added to the context.
lavapipe_log="${TMPDIR:-/tmp}/ssmap-lavapipe-install.log"

# Runs a command as root without ever asking for a password.
as_root() {
  if [ "$(id -u)" = "0" ]; then
    "$@"
  else
    sudo -n "$@"
  fi
}

# Runs apt-get, stopped after $1 seconds. `timeout` is inside `as_root` so that it can signal
# apt-get, which runs as root.
apt_get() {
  local seconds="$1"
  shift
  as_root timeout -k 10 "$seconds" env DEBIAN_FRONTEND=noninteractive \
    apt-get -q -o DPkg::Lock::Timeout=30 -o Acquire::Retries=1 -o Acquire::http::Timeout=20 \
    -o Acquire::https::Timeout=20 "$@" < /dev/null >> "$lavapipe_log" 2>&1
}

lavapipe_installed() {
  [ "$(dpkg-query -W -f='${Status}' mesa-vulkan-drivers 2>/dev/null)" = "install ok installed" ]
}

install_lavapipe() {
  if lavapipe_installed; then
    return 0
  fi
  local tool
  for tool in apt-get timeout; do
    if ! command -v "$tool" > /dev/null; then
      echo "lavapipe not installed (no $tool): render tests skip."
      return 0
    fi
  done
  if ! as_root true < /dev/null > /dev/null 2>&1; then
    echo "lavapipe not installed (no root): render tests skip."
    return 0
  fi
  # One unreachable repository fails the update even when the one we need was refreshed, so the
  # install is tried either way.
  apt_get 60 update || true
  if apt_get 180 install -y --no-install-recommends mesa-vulkan-drivers && lavapipe_installed; then
    echo "lavapipe installed: render tests compare golden images."
  else
    echo "lavapipe not installed (apt-get failed, see $lavapipe_log): render tests skip."
  fi
}

install_lavapipe || true

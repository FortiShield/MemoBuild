#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "Usage: $0 /path/to/memobuild" >&2
  exit 2
}
[[ $# -eq 1 ]] || usage
ROOT="$(cd "$1" && pwd)"
cd "$ROOT"

command -v cargo >/dev/null || { echo "cargo is required" >&2; exit 127; }

echo "==> Cargo metadata"
cargo metadata --format-version 1 --no-deps >/dev/null

echo "==> Rust formatting"
cargo fmt --all -- --check

echo "==> Workspace check"
cargo check --workspace --all-targets

echo "==> Workspace tests"
cargo test --workspace

if [[ "${SKIP_CLIPPY:-0}" != "1" ]]; then
  echo "==> Clippy"
  cargo clippy --workspace --all-targets -- -D warnings
else
  echo "==> Clippy skipped (SKIP_CLIPPY=1)"
fi

if [[ -x "./scripts/check-dependency-boundaries.py" ]]; then
  echo "==> Dependency boundary policy"
  ./scripts/check-dependency-boundaries.py
fi

echo "All configured Rust workspace checks passed."

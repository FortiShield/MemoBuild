#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "Usage: $0 /path/to/memobuild [output-report.txt]" >&2
  exit 2
}
[[ $# -ge 1 && $# -le 2 ]] || usage
ROOT="$(cd "$1" && pwd)"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
OUT="${2:-${TMPDIR:-/tmp}/memobuild-rust-audit-${STAMP}.txt}"

{
  echo "MemoBuild Rust layout audit"
  echo "UTC timestamp: ${STAMP}"
  echo "Repository: ${ROOT}"
  echo
  echo "=== Git status ==="
  git -C "$ROOT" status --short 2>&1 || true
  echo
  echo "=== Root manifest ==="
  if [[ -f "$ROOT/Cargo.toml" ]]; then
    sed -n '1,260p' "$ROOT/Cargo.toml"
  else
    echo "No root Cargo.toml found"
  fi
  echo
  echo "=== Rust source files ==="
  find "$ROOT/src" "$ROOT/tests" "$ROOT/benches" "$ROOT/examples" \
    -type f \( -name '*.rs' -o -name 'Cargo.toml' \) -print 2>/dev/null | sort || true
  echo
  echo "=== Module declarations (best-effort grep) ==="
  grep -RInE '^[[:space:]]*(pub[[:space:]]+)?(mod|use)[[:space:]]+' \
    "$ROOT/src" --include='*.rs' 2>/dev/null || true
  echo
  echo "=== Cargo metadata (may fail before workspace is valid) ==="
  cargo metadata --manifest-path "$ROOT/Cargo.toml" --no-deps --format-version 1 2>&1 || true
  echo
  echo "=== Cargo targets ==="
  cargo metadata --manifest-path "$ROOT/Cargo.toml" --no-deps --format-version 1 2>/dev/null \
    | python3 -c 'import json,sys; d=json.load(sys.stdin); [print(p.get("name"), p.get("manifest_path"), [(t.get("name"),t.get("kind")) for t in p.get("targets",[])]) for p in d.get("packages",[])]' \
    2>/dev/null || true
  echo
  echo "=== Build and CI references to source paths ==="
  grep -RInE 'src/(main|lib|cache|executor|cluster_server)|Dockerfile\.(farm|sample|server)|dist/index\.js' \
    "$ROOT/.github" "$ROOT/Dockerfile"* "$ROOT/docker-compose"* "$ROOT/package.json" \
    "$ROOT/scripts" "$ROOT/docs" 2>/dev/null || true
} > "$OUT"

echo "Audit report written to: $OUT"
echo "Review the report before changing manifests or moving source files."

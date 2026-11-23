#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
BIN=${1:-"$ROOT/target/debug/lume-bootstrap"}

if [[ ! -x "$BIN" ]]; then
  echo "lume bootstrap not found or not executable: $BIN" >&2
  exit 1
fi

cd "$ROOT"

check_ok() {
  "$BIN" check "$1" > /dev/null
  echo "check: $1"
}

run_expected() {
  local expected=$1
  shift
  local actual
  actual=$("$BIN" "$@")
  if [[ "$actual" != "$expected" ]]; then
    printf 'unexpected output for %q' "$1" >&2
    printf '\nexpected:\n%s\nactual:\n%s\n' "$expected" "$actual" >&2
    return 1
  fi
  echo "run: $*"
}

check_ok examples/demo_basic/src/main.lume
run_expected 42 run examples/demo_basic/src/main.lume

check_ok examples/demo_buffer.lume
run_expected $'255\n16909060\nlume\n16909315' run examples/demo_buffer.lume

check_ok examples/demo_buffer_jit.lume
run_expected 50727435 run examples/demo_buffer_jit.lume
run_expected 50727435 jit examples/demo_buffer_jit.lume
"$BIN" llvm-ir examples/demo_buffer_jit.lume > /dev/null

"$BIN" run examples/demo_math_logging.lume > /dev/null
"$BIN" jit examples/demo_basic/src/main.lume > /dev/null
"$BIN" jit examples/demo_llvm_jit.lume > /dev/null
"$BIN" llvm-ir examples/demo_llvm_jit.lume > /dev/null

check_ok examples/demo_c_interop/manifest.toml
"$BIN" header examples/demo_c_interop/manifest.toml > /dev/null
run_expected 8256 run examples/demo_c_interop/manifest.toml
run_expected 8256 jit examples/demo_c_interop/manifest.toml
"$BIN" llvm-ir examples/demo_c_interop/manifest.toml > /dev/null

manifest_docs=$("$BIN" docs examples/demo_c_interop/manifest.toml)
if [[ "$manifest_docs" != *"c_matmul"* || "$manifest_docs" != *"src/calc.c"* ]]; then
  echo "manifest documentation omitted module or external symbol details" >&2
  exit 1
fi

echo "All Lume example checks passed."

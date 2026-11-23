#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=${LUME_BIN:-"$ROOT/target/debug/lume-bootstrap"}
CC=${CC:-clang}
PORT=${LUME_ECHO_PORT:-18080}

if [[ ! -x "$BIN" ]]; then
  echo "lume bootstrap not found: $BIN" >&2
  exit 1
fi

WORK=$(mktemp -d)
CLIENT="$WORK/test_echo"
case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*)
    CLIENT="$WORK/test_echo.exe"
    "$CC" "$ROOT/examples/server_echo/test_echo.c" -o "$CLIENT" -lws2_32
    ;;
  *)
    "$CC" "$ROOT/examples/server_echo/test_echo.c" -o "$CLIENT"
    ;;
esac

cd "$ROOT"
"$BIN" check examples/server_echo/manifest.toml > /dev/null
"$BIN" header examples/server_echo/manifest.toml > /dev/null

LOG="$WORK/server.log"
"$BIN" run examples/server_echo/manifest.toml > "$LOG" 2>&1 &
server_pid=$!
cleanup() {
  kill "$server_pid" 2>/dev/null || true
  rm -rf "$WORK"
}
trap cleanup EXIT

"$CLIENT" "$PORT"
wait "$server_pid"

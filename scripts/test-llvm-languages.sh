#!/usr/bin/env bash
# 用统一的 LLVM 后端编译 C / Rust / Fortran 测试代码，验证：
#   1. 三种语言都能产出 LLVM bitcode；
#   2. 导出符号符合 C ABI 命名；
#   3. 合并后的 bitcode 能被 llvm-link 链接并由 lli 执行。
# Fortran 编译器不可用时只跳过该项，不把整条测试判失败。
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
SRC="$ROOT/tests/llvm-languages"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

find_tool() {
  local candidate
  for candidate in "$@"; do
    if command -v "$candidate" > /dev/null 2>&1; then
      command -v "$candidate"
      return 0
    fi
  done
  return 1
}

CLANG=$(find_tool clang clang-23 clang-22 clang-20)
LLVM_AS=$(find_tool llvm-as llvm-as-23 llvm-as-22 llvm-as-20)
LLVM_NM=$(find_tool llvm-nm llvm-nm-23 llvm-nm-22 llvm-nm-20)
LLVM_LINK=$(find_tool llvm-link llvm-link-23 llvm-link-22 llvm-link-20)
LLI=$(find_tool lli lli-23 lli-22 lli-20) || LLI=""
RUSTC=$(find_tool rustc) || { echo "rustc not found" >&2; exit 1; }
FLANG=$(find_tool flang flang-new flang-23 flang-22) || FLANG=""
RUST_HOST=$("$RUSTC" -vV | sed -n 's/^host: //p')

echo "llvm-languages: clang=$CLANG"
echo "llvm-languages: rustc=$RUSTC"
if [[ -n "$FLANG" ]]; then
  echo "llvm-languages: flang=$FLANG"
else
  echo "llvm-languages: Fortran SKIP (no flang/flang-new found)"
fi

check_symbol() {
  local bitcode=$1
  local symbol=$2
  if ! "$LLVM_NM" --defined-only "$bitcode" | grep -q "$symbol"; then
    echo "llvm-languages: missing exported symbol $symbol in $bitcode" >&2
    "$LLVM_NM" --defined-only "$bitcode" >&2 || true
    exit 1
  fi
  echo "llvm-languages: symbol $symbol OK"
}

# C -> LLVM bitcode
"$CLANG" -target "$RUST_HOST" -emit-llvm -c "$SRC/add.c" -o "$WORK/add-c.bc"
check_symbol "$WORK/add-c.bc" lume_test_add_c

# Rust -> LLVM IR -> LLVM bitcode（统一交给 llvm-as，不依赖 rustc 的 bc 输出开关）
"$RUSTC" --edition=2021 --crate-type=lib --emit=llvm-ir \
  -C panic=abort -C overflow-checks=off -C opt-level=1 \
  -o "$WORK/add-rust.ll" "$SRC/add.rs"
"$LLVM_AS" "$WORK/add-rust.ll" -o "$WORK/add-rust.bc"
check_symbol "$WORK/add-rust.bc" lume_test_add_rust

LINK_INPUTS=("$WORK/add-c.bc" "$WORK/add-rust.bc")

# Fortran -> LLVM bitcode（用 bind(C) 保证符号不带下划线）
if [[ -n "$FLANG" ]]; then
  "$FLANG" -emit-llvm -c "$SRC/add.f90" -o "$WORK/add-fortran.bc"
  check_symbol "$WORK/add-fortran.bc" lume_test_add_fortran
  LINK_INPUTS+=("$WORK/add-fortran.bc")
fi

# 一个 C main 同时调用所有语言的实现，并链成单一 bitcode。
{
  printf '%s\n' '#include <stdint.h>' '#include <stdio.h>'
  printf '%s\n' 'int32_t lume_test_add_c(int32_t, int32_t);'
  printf '%s\n' 'int32_t lume_test_add_rust(int32_t, int32_t);'
  if [[ -n "$FLANG" ]]; then
    printf '%s\n' 'int32_t lume_test_add_fortran(int32_t, int32_t);'
  fi
  printf '%s\n' 'int main(void) {'
  printf '%s\n' '  if (lume_test_add_c(2, 3) != 5) return 1;'
  printf '%s\n' '  if (lume_test_add_rust(2, 3) != 5) return 2;'
  if [[ -n "$FLANG" ]]; then
    printf '%s\n' '  if (lume_test_add_fortran(2, 3) != 5) return 3;'
  fi
  printf '%s\n' '  printf("LLVM_LANG_OK\n");'
  printf '%s\n' '  return 0;' '}'
} > "$WORK/main.c"

"$CLANG" -target "$RUST_HOST" -emit-llvm -c "$WORK/main.c" -o "$WORK/main.bc"
"$LLVM_LINK" "${LINK_INPUTS[@]}" "$WORK/main.bc" -o "$WORK/combined.bc"
echo "llvm-languages: llvm-link OK (${#LINK_INPUTS[@]} language modules)"

if [[ -n "$LLI" ]]; then
  output=$("$LLI" "$WORK/combined.bc")
  if [[ "$output" != "LLVM_LANG_OK" ]]; then
    echo "llvm-languages: unexpected lli output: $output" >&2
    exit 1
  fi
  echo "llvm-languages: lli $output"
else
  echo "llvm-languages: lli SKIP (not installed)"
fi

echo "All LLVM backend language checks passed."

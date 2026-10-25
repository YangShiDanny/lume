#!/usr/bin/env bash
#
# Lume 测试驱动。
#
#   scripts/run_tests.sh              跑全部用例
#   UPDATE=1 scripts/run_tests.sh     重写 golden（确认输出变化是有意的之后）
#   LUME_BIN=/path/to/lume ...        换用别的编译器二进制
#   LUME_SKIP_BUILD=1 ...             跳过 cargo build（CI 里已经构建过时用）
#
# 两类用例：
#   1) 正向：main.lume + tests_*.lume。必须退出码 0，输出和 tests/golden/<stem>.out 一致。
#   2) 负向：tests/fail/*.lume。第一行 `// expect: <子串>` 声明期望的报错，
#      必须退出码非 0 且输出里含该子串。
#
# 为什么要有 golden：这些套件以前只把「期望值」印在 say 里，靠人眼看，
# 退出码永远是 0 —— 值算错了也照样绿。现在值错了就 diff 不过。
#
# 归一化：只抹掉真正的随机量（各类耗时、并行 worker 的输出顺序），
# 其余逐字节比对。别把归一化写宽了，否则等于没测。

set -uo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
GOLDEN="$ROOT/tests/golden"
BIN="${LUME_BIN:-$ROOT/target/debug/lume}"

pass=0
fail=0
failed_list=()

say_ok()  { printf '  \033[32mok\033[0m   %s\n' "$1"; }
say_bad() { printf '  \033[31mFAIL\033[0m %s\n' "$1"; }

# ── 构建 ──
if [ -z "${LUME_SKIP_BUILD:-}" ]; then
    printf '==> cargo build\n'
    if ! (cd "$ROOT" && cargo build --quiet); then
        printf '构建失败，后面的测试没有意义\n' >&2
        exit 1
    fi
fi
if [ ! -x "$BIN" ]; then
    printf 'error: 找不到编译器二进制 %s\n' "$BIN" >&2
    exit 1
fi

# 生成的 .c 和可执行文件都落在 CWD（cli.rs 的 output_c_path 只用 stem），
# 所以统一在临时目录里跑，别污染仓库根。
WORK=$(mktemp -d "${TMPDIR:-/tmp}/lume-tests.XXXXXX")
cleanup() { rm -rf "$WORK"; }
trap cleanup EXIT

# 耗时和并发顺序是唯一该被抹掉的东西：
#   "耗时 64 ns" / "首次 15104 ns" / "elapsed=24402 ns" → <Ns>
#   worker[640] → worker[*]（并行块顺序不保证，连续相同行由 uniq 收掉）
normalize() {
    sed -E 's/[0-9]+ ns/<Ns>/g; s/worker\[[0-9]+\]/worker[*]/g' | uniq
}

cd "$WORK" || exit 1

# ── 正向用例 ──
# check_positive <用例名> <入口文件>
check_positive() {
    local stem="$1" src="$2"
    local raw="$WORK/$stem.raw" norm="$WORK/$stem.norm"

    "$BIN" run "$src" > "$raw" 2>&1
    local rc=$?
    normalize < "$raw" > "$norm"

    if [ "$rc" -ne 0 ]; then
        say_bad "$stem（退出码 $rc）"
        tail -20 "$raw" | sed 's/^/       /'
        failed_list+=("$stem"); fail=$((fail + 1)); return
    fi

    if [ -n "${UPDATE:-}" ]; then
        cp "$norm" "$GOLDEN/$stem.out"
        say_ok "$stem（golden 已更新）"
        pass=$((pass + 1)); return
    fi

    if [ ! -f "$GOLDEN/$stem.out" ]; then
        say_bad "$stem（缺 tests/golden/$stem.out，先跑 UPDATE=1 scripts/run_tests.sh 生成基线）"
        failed_list+=("$stem"); fail=$((fail + 1)); return
    fi

    if diff -q "$GOLDEN/$stem.out" "$norm" > /dev/null; then
        say_ok "$stem"
        pass=$((pass + 1))
    else
        say_bad "$stem（输出和 golden 不一致）"
        diff -u "$GOLDEN/$stem.out" "$norm" | head -30 | sed 's/^/       /'
        failed_list+=("$stem"); fail=$((fail + 1))
    fi
}

printf '==> 正向用例（golden 比对）\n'
shopt -s nullglob
positive=("$ROOT/main.lume")
for f in "$ROOT"/tests_*.lume; do
    positive+=("$f")
done
shopt -u nullglob

mkdir -p "$GOLDEN"

for src in "${positive[@]}"; do
    check_positive "$(basename "$src" .lume)" "$src"
done

# ── 多文件用例（模块系统） ──
# 入口是 tests/multi/main.lume，同目录的 mathx.lume / textx.lume
# 和子目录 util/deep.lume 靠「自动扫描」并入，源码里没有任何 import。
if [ -d "$ROOT/tests/multi" ]; then
    printf '==> 多文件用例（模块系统）\n'
    check_positive "multi" "$ROOT/tests/multi/main.lume"
fi

# ── 负向用例 ──
printf '==> 负向用例（必须报错）\n'
shopt -s nullglob
negatives=("$ROOT"/tests/fail/*.lume)
shopt -u nullglob

for src in "${negatives[@]}"; do
    stem="fail/$(basename "$src" .lume)"
    expect=$(head -1 "$src" | sed -E 's|^// *expect: *||')
    if [ -z "$expect" ] || [ "$expect" = "$(head -1 "$src")" ]; then
        say_bad "$stem（第一行没有 // expect: <子串>）"
        failed_list+=("$stem")
        fail=$((fail + 1))
        continue
    fi

    raw="$WORK/neg_$(basename "$src" .lume).raw"
    "$BIN" run "$src" > "$raw" 2>&1
    rc=$?

    if [ "$rc" -eq 0 ]; then
        say_bad "$stem（本该失败，却成功退出）"
        failed_list+=("$stem")
        fail=$((fail + 1))
    elif ! grep -qF -- "$expect" "$raw"; then
        say_bad "$stem（退出码 $rc，但输出里没有「$expect」）"
        tail -10 "$raw" | sed 's/^/       /'
        failed_list+=("$stem")
        fail=$((fail + 1))
    else
        say_ok "$stem"
        pass=$((pass + 1))
    fi
done

# ── 负向：多文件项目 ──
# 目录形式：tests/fail_multi/<用例名>/main.lume 是入口，
# 期望的报错子串写在它的第一行（和单文件用例同格式）。
printf '==> 负向用例（多文件）\n'
shopt -s nullglob
neg_multi=("$ROOT"/tests/fail_multi/*/main.lume)
shopt -u nullglob

for src in "${neg_multi[@]}"; do
    name=$(basename "$(dirname "$src")")
    stem="fail_multi/$name"
    expect=$(head -1 "$src" | sed -E 's|^// *expect: *||')
    if [ -z "$expect" ] || [ "$expect" = "$(head -1 "$src")" ]; then
        say_bad "$stem（第一行没有 // expect: <子串>）"
        failed_list+=("$stem")
        fail=$((fail + 1))
        continue
    fi

    raw="$WORK/neg_multi_$name.raw"
    "$BIN" run "$src" > "$raw" 2>&1
    rc=$?

    if [ "$rc" -eq 0 ]; then
        say_bad "$stem（本该失败，却成功退出）"
        failed_list+=("$stem")
        fail=$((fail + 1))
    elif ! grep -qF -- "$expect" "$raw"; then
        say_bad "$stem（退出码 $rc，但输出里没有「$expect」）"
        tail -10 "$raw" | sed 's/^/       /'
        failed_list+=("$stem")
        fail=$((fail + 1))
    else
        say_ok "$stem"
        pass=$((pass + 1))
    fi
done

# ── 汇总 ──
printf '\n通过 %d，失败 %d\n' "$pass" "$fail"
if [ "$fail" -ne 0 ]; then
    printf '失败用例：%s\n' "${failed_list[*]}"
    exit 1
fi

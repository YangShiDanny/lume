#!/usr/bin/env bash
# 把仓库 docs/ 下的 Markdown 单向同步到 GitHub Wiki 检出目录。
# 用法: sync-wiki.sh <docs目录> <wiki检出目录>
set -euo pipefail

DOCS_DIR=${1:?用法: sync-wiki.sh <docs目录> <wiki检出目录>}
WIKI_DIR=${2:?用法: sync-wiki.sh <docs目录> <wiki检出目录>}

if [ ! -d "$DOCS_DIR" ]; then
  echo "docs 目录不存在: $DOCS_DIR" >&2
  exit 1
fi
if [ ! -d "$WIKI_DIR/.git" ]; then
  echo "Wiki 目录不是 git 仓库: $WIKI_DIR" >&2
  exit 1
fi

START='<!-- lume-docs-sync:start -->'
END='<!-- lume-docs-sync:end -->'

shopt -s nullglob
files=("$DOCS_DIR"/*.md)
shopt -u nullglob

if [ ${#files[@]} -eq 0 ]; then
  echo "docs/ 下没有 Markdown 文件，跳过同步。"
  exit 0
fi

# Wiki 页面都在根目录，把仓库内的相对链接改写掉。
for src in "${files[@]}"; do
  name=${src##*/}
  sed \
    -e 's#](docs/#](#g' \
    -e 's#](/docs/#](#g' \
    -e 's#](\./#](#g' \
    "$src" > "$WIKI_DIR/$name"
done

# _Sidebar.md 只托管标记块内的内容，块外的自定义内容原样保留。
sidebar="$WIKI_DIR/_Sidebar.md"
block="$WIKI_DIR/.lume-sidebar-block"
{
  printf '%s\n' "$START"
  printf '## Lume 文档\n\n'
  for src in "${files[@]}"; do
    name=${src##*/}
    page=${name%.md}
    title=$(sed -n 's/^#[[:space:]]\+//p' "$src" | sed -n '1p')
    if [ -z "$title" ]; then
      title=$page
    fi
    printf -- '- [[%s|%s]]\n' "$page" "$title"
  done
  printf '%s\n' "$END"
} > "$block"

if [ -f "$sidebar" ] && grep -qF "$START" "$sidebar" && grep -qF "$END" "$sidebar"; then
  awk -v s="$START" -v e="$END" -v b="$block" '
    $0 == s { inside = 1; while ((getline line < b) > 0) print line; close(b); next }
    $0 == e { inside = 0; next }
    !inside { print }
  ' "$sidebar" > "$sidebar.tmp"
else
  if [ -f "$sidebar" ]; then
    cat "$sidebar" > "$sidebar.tmp"
    printf '\n' >> "$sidebar.tmp"
  else
    : > "$sidebar.tmp"
  fi
  cat "$block" >> "$sidebar.tmp"
fi

mv "$sidebar.tmp" "$sidebar"
rm -f "$block"

echo "已同步 ${#files[@]} 个文档到 $WIKI_DIR"

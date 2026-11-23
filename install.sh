#!/usr/bin/env bash
# Lume 安装脚本（Linux / macOS / BSD / Git Bash）。
# 从 GitHub Releases 下载对应平台的预编译包，解压到本地并创建 lume 命令链接。
set -euo pipefail

REPO="${LUME_REPO:-YangShiDanny/lume}"
VERSION="${LUME_VERSION:-latest}"
PREFIX="${LUME_PREFIX:-$HOME/.local}"

case "$(uname -s)" in
  Linux)  PLATFORM="linux-x64" ;;
  Darwin) PLATFORM="macos-arm64" ;;
  MINGW*|MSYS*|CYGWIN*) echo "Windows 请使用 install.ps1" >&2; exit 1 ;;
  *) echo "不支持的操作系统: $(uname -s)" >&2; exit 1 ;;
esac

ARCH="$(uname -m)"
if [[ "$PLATFORM" == "linux-x64" && "$ARCH" != "x86_64" && "$ARCH" != "amd64" ]]; then
  echo "警告: Linux 预编译包仅提供 x64，当前架构 $ARCH 需从源码构建。" >&2
fi
if [[ "$PLATFORM" == "macos-arm64" && "$ARCH" != "arm64" ]]; then
  echo "警告: macOS 预编译包仅提供 arm64，当前架构 $ARCH 需从源码构建。" >&2
fi

if [[ "$VERSION" == "latest" ]]; then
  URL="https://github.com/$REPO/releases/latest/download/lume-$PLATFORM.tar.gz"
else
  URL="https://github.com/$REPO/releases/download/$VERSION/lume-$PLATFORM.tar.gz"
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "[lume] 下载 $URL"
curl -fL --retry 3 -o "$TMP/lume.tar.gz" "$URL"

echo "[lume] 解压到 $PREFIX"
mkdir -p "$PREFIX/lib/lume"
tar -xzf "$TMP/lume.tar.gz" -C "$TMP"
rm -rf "$PREFIX/lib/lume"
mv "$TMP/lume-$PLATFORM" "$PREFIX/lib/lume"
chmod +x "$PREFIX/lib/lume/bin/"* "$PREFIX/lib/lume/lume" 2>/dev/null || true

mkdir -p "$PREFIX/bin"
ln -sf "$PREFIX/lib/lume/lume" "$PREFIX/bin/lume"

echo
echo "[lume] 安装完成。若 $PREFIX/bin 不在 PATH，请加入："
echo "  export PATH=\"$PREFIX/bin:\$PATH\""
echo "[lume] 运行 'lume --help' 或 'lume' 进入 REPL。"

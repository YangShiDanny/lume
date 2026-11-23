#!/usr/bin/env bash
# 在 macOS runner 上安装 LLVM 23（Apple Silicon 用 Homebrew bottle；Intel 回退源码构建）。
set -euo pipefail

# CI 使用 macos-14 (arm64)，Homebrew 提供 llvm@23 bottle。
# 若在 Intel Mac 上本地运行，Homebrew 无 bottle 会自动走源码构建。
brew install llvm@23
PREFIX="$(brew --prefix llvm@23)"

"$PREFIX/bin/llvm-config" --version
echo "LLVM_SYS_231_PREFIX=$PREFIX" >> "$GITHUB_ENV"
echo "$PREFIX/bin" >> "$GITHUB_PATH"

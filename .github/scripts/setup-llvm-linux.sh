#!/usr/bin/env bash
# 在 Ubuntu runner 上通过 apt.llvm.org 安装 LLVM 23。
set -euo pipefail

curl --retry 5 --retry-delay 2 --retry-all-errors \
  -fsSL https://apt.llvm.org/llvm.sh -o /tmp/llvm.sh
chmod +x /tmp/llvm.sh
sudo /tmp/llvm.sh 23
sudo apt-get install -y llvm-23-dev clang-23

llvm-config-23 --version
echo "LLVM_SYS_231_PREFIX=/usr/lib/llvm-23" >> "$GITHUB_ENV"
echo "/usr/lib/llvm-23/bin" >> "$GITHUB_PATH"

#!/usr/bin/env bash
# 在 Ubuntu runner 上通过 apt.llvm.org 安装 LLVM 23。
set -euo pipefail

curl --retry 5 --retry-delay 2 --retry-all-errors \
  -fsSL https://apt.llvm.org/llvm.sh -o /tmp/llvm.sh
chmod +x /tmp/llvm.sh
sudo /tmp/llvm.sh 23
# llvm-sys 会按 llvm-config --libs 链接全部组件，apt 把 Polly 拆成了独立包，
# 只装 llvm-23-dev 会报 "could not find native static library `Polly`"。
sudo apt-get install -y llvm-23-dev clang-23 libpolly-23-dev

llvm-config-23 --version
echo "LLVM_SYS_231_PREFIX=/usr/lib/llvm-23" >> "$GITHUB_ENV"
echo "/usr/lib/llvm-23/bin" >> "$GITHUB_PATH"

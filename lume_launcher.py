#!/usr/bin/env python3
"""Lume 跨平台启动器。

职责与平台无关：定位仓库根目录、解析 --help、找 Python/venv、调用 REPL，
或在必要时先构建 lume-bootstrap 再转发参数。

Windows 的 lume.cmd / lume.ps1 和 Unix 的 lume 脚本都只是薄包装，最终都进入
这里，保证三套平台的命令行为一致。

不修改 Rust 源码；构建通过 cargo 完成。
"""
from __future__ import annotations

import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
EXE_SUFFIX = ".exe" if os.name == "nt" else ""
BOOTSTRAP = ROOT / "target" / "release" / f"lume-bootstrap{EXE_SUFFIX}"
CARGO = os.environ.get("CARGO", "cargo")

HELP = """Usage:
  lume                         进入 REPL（逐行执行）
  lume -i | --repl             同上
  lume run <file.lume>         解释执行
  lume check <file.lume>       语法检查
  lume jit <file.lume>         LLVM JIT 执行
  lume llvm-ir <file.lume>     输出 LLVM IR
  lume manifest <file.toml>    构建混合 C/Lume 项目
  lume --lsp                   启动语言服务器
  lume --help                  显示本帮助

环境变量：
  LUME_REPL_PYTHON   REPL 使用的 Python 解释器路径
  LUME_BOOTSTRAP     覆盖 lume-bootstrap 可执行文件路径
  LLVM_SYS_231_PREFIX  LLVM 23 安装前缀（构建时需要）
"""


def fail(message: str, code: int = 1) -> "NoReturn":  # type: ignore[valid-type]
    print(f"[lume] {message}", file=sys.stderr)
    raise SystemExit(code)


def find_bootstrap() -> Path:
    override = os.environ.get("LUME_BOOTSTRAP")
    if override:
        path = Path(override).expanduser()
        if not path.is_file():
            fail(f"LUME_BOOTSTRAP 指向的文件不存在: {path}", 127)
        return path
    if BOOTSTRAP.is_file():
        return BOOTSTRAP
    fail(f"未找到 {BOOTSTRAP}，请先运行: lume --build", 127)


def build_bootstrap() -> None:
    if not shutil.which(CARGO):
        fail(f"未找到 cargo（{CARGO}）。请先安装 Rust stable 工具链。", 127)
    cmd = [
        CARGO,
        "build",
        "--release",
        "--manifest-path",
        str(ROOT / "Cargo.toml"),
        "--bin",
        "lume-bootstrap",
    ]
    print(f"[lume] 构建 lume-bootstrap: {' '.join(cmd)}", file=sys.stderr)
    result = subprocess.run(cmd, cwd=ROOT)
    if result.returncode != 0:
        fail("lume-bootstrap 构建失败", result.returncode)


def candidate_pythons() -> list[Path]:
    candidates: list[Path] = []
    override = os.environ.get("LUME_REPL_PYTHON")
    if override:
        candidates.append(Path(override).expanduser())

    venv = ROOT / ".venv"
    if os.name == "nt":
        candidates.extend(
            [
                venv / "Scripts" / "python.exe",
                Path.home() / ".local" / "share" / "lume-repl-venv" / "Scripts" / "python.exe",
            ]
        )
    else:
        candidates.extend(
            [
                venv / "bin" / "python",
                Path.home() / ".local" / "share" / "lume-repl-venv" / "bin" / "python",
            ]
        )

    for name in ("python3", "python"):
        found = shutil.which(name)
        if found:
            candidates.append(Path(found))
    return candidates


def find_python() -> Path:
    for candidate in candidate_pythons():
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return candidate
        # shutil.which 返回的路径可能已在上面命中；这里再兜一次 PATH 解析。
        found = shutil.which(str(candidate))
        if found:
            return Path(found)
    fail(
        "未找到 Python 3。请安装 Python 3.10+，或设置 LUME_REPL_PYTHON 指向解释器。",
        127,
    )


def run_repl() -> "NoReturn":  # type: ignore[valid-type]
    python = find_python()
    env = os.environ.copy()
    env.pop("PYTHONPATH", None)
    env.pop("LD_LIBRARY_PATH", None)
    result = subprocess.run([str(python), str(ROOT / "lume_repl.py")], cwd=ROOT, env=env)
    raise SystemExit(result.returncode)


def main(argv: list[str]) -> int:
    args = argv[1:]

    if not args or args[0] in {"-i", "--repl"}:
        if len(args) > 1:
            fail("REPL 不接受额外参数；用 'lume --help' 查看用法。", 2)
        run_repl()

    if args[0] in {"-h", "--help"}:
        sys.stdout.write(HELP)
        return 0

    if args[0] in {"--build", "build"}:
        build_bootstrap()
        return 0

    if args[0] in {"-v", "--version"}:
        sys.stdout.write("lume 0.1.0 (bootstrap runtime)\n")
        return 0

    bootstrap = find_bootstrap()
    # 文件模式保留调用者的相对路径语义，不切换 cwd。
    result = subprocess.run([str(bootstrap), *args], cwd=os.getcwd())
    return result.returncode


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))

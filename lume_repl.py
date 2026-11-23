#!/usr/bin/env python3
"""Lume REPL：用 Python 驱动现有 lume-bootstrap，提供语法高亮和逐行执行。

不修改 Rust 源码；每次把会话状态拼成一个临时 .lume 文件交给编译器执行。
"""
from __future__ import annotations

import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent
BOOTSTRAP = ROOT / "target" / "release" / "lume-bootstrap"

KEYWORDS = {"fn", "ret", "if", "else", "end", "while", "import", "is", "and", "or", "not"}
BUILTINS = {
    "print", "len", "sin", "cos", "tan", "sqrt", "pow", "abs", "min", "max",
    "trace", "debug", "info", "warn", "error",
    "log_trace", "log_debug", "log_info", "log_warn", "log_error",
}

DECL_RE = re.compile(r"^\s*([A-Za-z_]\w*)\s*(?::\s*[A-Za-z_]\w*)?\s*=\s*(.+)$")
WALRUS_RE = re.compile(r"^\s*([A-Za-z_]\w*)\s*:=\s*(.+)$")
CALL_RE = re.compile(r"^\s*([A-Za-z_]\w*)\s*\(")
SIDE_EFFECT_RE = re.compile(
    r"^\s*(print|log_[a-z_]+|trace|debug|info|warn|error)\s*\("
)

if sys.stdin.isatty():
    from prompt_toolkit import PromptSession
    from prompt_toolkit.history import FileHistory
    from prompt_toolkit.lexers import PygmentsLexer
    from prompt_toolkit.styles import Style
    from pygments.lexer import RegexLexer
    from pygments.token import Comment, Keyword, Name, Number, Operator, Punctuation, String, Text

    class LumeLexer(RegexLexer):
        name = "Lume"
        tokens = {
            "root": [
                (r"#.*$", Comment.Single),
                (r'"(?:\\.|[^"\\])*"', String),
                (r"\b(fn|ret|if|else|end|while|import|is|and|or|not)\b", Keyword),
                (r"\b[A-Za-z_]\w*(?=\s*\()", Name.Builtin),
                (r"\b\d+\.\d+\b", Number.Float),
                (r"\b\d+\b", Number.Integer),
                (r"==|!=|<=|>=|:=|->|[+\-*/%<>=]", Operator),
                (r"[A-Za-z_]\w*", Name),
                (r"\s+", Text),
                (r".", Punctuation),
            ]
        }

    STYLE = Style.from_dict(
        {
            "": "#e8e8ec",
            "pygments.keyword": "#c792ea bold",
            "pygments.name.builtin": "#82aaff",
            "pygments.number": "#f78c6c",
            "pygments.string": "#c3e88d",
            "pygments.comment": "#546e7a italic",
            "pygments.operator": "#89ddff",
        }
    )
else:
    PromptSession = None  # type: ignore[assignment]
    LumeLexer = None  # type: ignore[assignment]
    STYLE = None  # type: ignore[assignment]


class Session:
    def __init__(self) -> None:
        self.imports: list[str] = []
        self.functions: list[str] = []
        self.statements: list[str] = []
        self.last_output = ""

    def reset(self) -> None:
        self.imports.clear()
        self.functions.clear()
        self.statements.clear()
        self.last_output = ""

    def add_function(self, chunk: str) -> None:
        match = re.match(r"\s*fn\s+([A-Za-z_]\w*)", chunk)
        name = match.group(1) if match else None
        if name:
            self.functions = [f for f in self.functions if not re.match(rf"\s*fn\s+{re.escape(name)}\b", f)]
        self.functions.append(chunk.rstrip())

    def source(self, current: str, expression: bool) -> str:
        parts: list[str] = []
        parts.extend(self.imports)
        if self.functions:
            parts.extend(self.functions)
            parts.append("")
        parts.append("fn boot()")
        for line in self.statements:
            parts.append("    " + line)
        for line in current.splitlines():
            parts.append("    " + line)
        if expression:
            parts.append("    ret " + current.strip())
        parts.append("end")
        return "\n".join(parts) + "\n"

    def run(self, current: str, expression: bool) -> tuple[str, str]:
        source = self.source(current, expression)
        fd, path = tempfile.mkstemp(prefix="lume_repl_", suffix=".lume", dir=ROOT)
        try:
            with os.fdopen(fd, "w", encoding="utf-8") as handle:
                handle.write(source)
            result = subprocess.run(
                [str(BOOTSTRAP), "run", path],
                cwd=ROOT,
                text=True,
                capture_output=True,
            )
        finally:
            try:
                os.unlink(path)
            except OSError:
                pass
        out = result.stdout
        if self.last_output and out.startswith(self.last_output):
            out = out[len(self.last_output) :]
        self.last_output = result.stdout if result.returncode == 0 else ""
        return out, result.stderr


def block_delta(line: str) -> int:
    text = line.strip()
    if not text or text.startswith("#"):
        return 0
    if text == "end":
        return -1
    opens = (
        text.startswith("fn ")
        or text.startswith("if ")
        or text.startswith("while ")
        or "= if " in text
        or "= while " in text
        or text.endswith("= if")
        or text.endswith("= while")
    )
    return 1 if opens else 0


def looks_like_expression(line: str) -> bool:
    text = line.strip()
    if not text or text.startswith("#"):
        return False
    if text.startswith(("fn ", "if ", "while ", "else", "end", "ret ", "import ")):
        return False
    if DECL_RE.match(text) or WALRUS_RE.match(text):
        return False
    call = CALL_RE.match(text)
    if call and call.group(1) in BUILTINS | {"print"}:
        return False
    return True


def print_help() -> None:
    print("命令：")
    print("  :help     显示帮助")
    print("  :vars     查看当前变量/语句")
    print("  :funcs    查看当前函数")
    print("  :source   查看生成的 Lume 源码")
    print("  :clear    清空会话")
    print("  :quit     退出（也可输入 quit() / exit()）")
    print("示例： 1 + 2   /   x:int = 40 + 2   /   x   /   print(x)")


def make_prompt() -> object:
    if PromptSession is None:
        return None
    return PromptSession(
        history=FileHistory(str(Path.home() / ".lume_repl_history")),
        lexer=PygmentsLexer(LumeLexer),
        style=STYLE,
    )


def read_line(prompt_session: object | None, prompt_text: str) -> str | None:
    if prompt_session is not None:
        try:
            return prompt_session.prompt(prompt_text)
        except (EOFError, KeyboardInterrupt):
            return None
    try:
        return input(prompt_text)
    except (EOFError, KeyboardInterrupt):
        return None


def main() -> None:
    if not BOOTSTRAP.exists():
        print("请先构建一次：lume <file.lume>", file=sys.stderr)
        raise SystemExit(1)

    print("Lume REPL —— 输入一行执行一行，:help 查看命令。")
    session = Session()
    prompt_session = make_prompt()
    pending: list[str] = []
    depth = 0

    while True:
        line = read_line(prompt_session, "... " if depth > 0 else ">>> ")
        if line is None:
            print()
            break
        text = line.strip()
        if depth == 0:
            if text in {":quit", "quit()", "exit()"}:
                break
            if text == ":help":
                print_help()
                continue
            if text == ":vars":
                for stmt in session.statements:
                    print("  " + stmt)
                continue
            if text == ":funcs":
                for func in session.functions:
                    print("  " + func.splitlines()[0])
                continue
            if text == ":source":
                print(session.source("", False))
                continue
            if text == ":clear":
                session.reset()
                continue
            if text.startswith(":"):
                print(f"未知命令: {text}，输入 :help 查看帮助。")
                continue
        if not text:
            continue

        pending.append(line)
        depth += block_delta(line)
        if depth > 0:
            continue

        chunk = "\n".join(pending)
        pending.clear()
        depth = 0

        if chunk.lstrip().startswith("fn "):
            session.add_function(chunk)
            continue
        if chunk.lstrip().startswith("import "):
            session.imports.extend(chunk.splitlines())
            continue

        expression = len(chunk.splitlines()) == 1 and looks_like_expression(chunk)
        out, err = session.run(chunk, expression)
        if not expression and not SIDE_EFFECT_RE.match(chunk.strip()):
            # print/log 只在本次执行，不写进上下文，避免下次重复输出。
            session.statements.append(chunk.rstrip())
        if not expression and out.endswith("0\n"):
            # 函数正常返回时 lume-bootstrap 会补打一个 0，REPL 里不显示。
            out = out[:-2]
        if out:
            print(out, end="" if out.endswith("\n") else "\n")
        if err:
            print("\033[31m" + err.rstrip() + "\033[0m")


if __name__ == "__main__":
    main()

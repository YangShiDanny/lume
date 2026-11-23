import { useCallback, useEffect, useMemo, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { CheckIcon, CopyIcon, DownloadIcon } from './Icons';

type SelectionRect = { left: number; top: number; width: number; height: number };
type MenuPos = { x: number; y: number };

const KEYWORDS = new Set(['fn', 'ret', 'if', 'else', 'while', 'end', 'and', 'or', 'not', 'is', 'true', 'false']);
const TYPES = new Set(['int', 'i16', 'float', 'bool', 'string', 'pointer']);
const BUILTINS = new Set([
  'print',
  'len',
  'pow',
  'atan2',
  'hypot',
  'fmod',
  'remainder',
  'copysign',
  'log_at',
  'sin',
  'cos',
  'tan',
  'asin',
  'acos',
  'atan',
  'sinh',
  'cosh',
  'tanh',
  'exp',
  'exp2',
  'expm1',
  'ln',
  'log',
  'log2',
  'log10',
  'log1p',
  'sqrt',
  'cbrt',
  'floor',
  'ceil',
  'round',
  'trunc',
  'abs',
  'trace',
  'debug',
  'info',
  'warn',
  'error',
]);

const TOKEN_RE = /("(?:\\.|[^"\\])*"|#.*$|\b\d+(?:\.\d+)?\b|\b[A-Za-z_][A-Za-z0-9_]*\b|[+\-*/%<>=:,()[\]])/g;

function tokenClass(token: string): string {
  if (token.startsWith('#')) return 'comment';
  if (token.startsWith('"')) return 'string';
  if (/^\d/.test(token)) return 'number';
  if (KEYWORDS.has(token)) return 'keyword';
  if (TYPES.has(token)) return 'type';
  if (BUILTINS.has(token)) return 'builtin';
  if (/^[+\-*/%<>=:,()[\]]$/.test(token)) return 'punct';
  return 'ident';
}

function highlight(line: string): ReactNode[] {
  const nodes: ReactNode[] = [];
  let last = 0;
  for (const match of line.matchAll(TOKEN_RE)) {
    const index = match.index ?? 0;
    if (index > last) nodes.push(line.slice(last, index));
    const token = match[0];
    nodes.push(
      <span className={`tok tok-${tokenClass(token)}`} key={`${index}-${token}`}>
        {token}
      </span>,
    );
    last = index + token.length;
  }
  if (last < line.length) nodes.push(line.slice(last));
  return nodes;
}

type Props = {
  code: string;
  blockCount: number;
};

export function CodePanel({ code, blockCount }: Props) {
  const [copied, setCopied] = useState(false);
  const [selRects, setSelRects] = useState<SelectionRect[]>([]);
  const [selText, setSelText] = useState('');
  const [menuPos, setMenuPos] = useState<MenuPos | null>(null);
  const lines = useMemo(() => code.replace(/\n$/, '').split('\n'), [code]);

  const clearSelection = useCallback(() => {
    setSelRects([]);
    setSelText('');
    setMenuPos(null);
  }, []);

  // 仿 LumeOS 的 LumeSel：原生高亮涂透明，用逐行圆角框绘制选区。
  useEffect(() => {
    const sync = () => {
      const selection = window.getSelection();
      if (!selection || selection.isCollapsed || !selection.rangeCount) {
        clearSelection();
        return;
      }
      const range = selection.getRangeAt(0);
      const node =
        range.commonAncestorContainer instanceof Element
          ? range.commonAncestorContainer
          : range.commonAncestorContainer.parentElement;
      if (!node?.closest('.code-scroll')) {
        clearSelection();
        return;
      }
      const rects = Array.from(range.getClientRects())
        .filter((rect) => rect.width > 0 && rect.height > 0)
        .map((rect) => ({
          left: rect.left,
          top: rect.top,
          width: rect.width,
          height: rect.height,
        }));
      if (!rects.length) {
        clearSelection();
        return;
      }
      setSelRects(rects);
      setSelText(selection.toString());
      const last = rects[rects.length - 1];
      setMenuPos({
        x: Math.min(window.innerWidth - 170, Math.max(8, last.left + last.width / 2 - 78)),
        y: Math.max(42, last.top - 38),
      });
    };
    document.addEventListener('selectionchange', sync);
    window.addEventListener('scroll', sync, true);
    window.addEventListener('resize', sync);
    return () => {
      document.removeEventListener('selectionchange', sync);
      window.removeEventListener('scroll', sync, true);
      window.removeEventListener('resize', sync);
    };
  }, [clearSelection, code]);

  const copySelection = async () => {
    try {
      await navigator.clipboard.writeText(selText);
    } catch {
      // 剪贴板不可用时忽略
    }
    window.getSelection()?.removeAllRanges();
    clearSelection();
  };

  const selectAll = () => {
    const host = document.querySelector('.code-scroll');
    const selection = window.getSelection();
    if (!host || !selection) return;
    const range = document.createRange();
    range.selectNodeContents(host);
    selection.removeAllRanges();
    selection.addRange(range);
  };

  const clearSelectionNow = () => {
    window.getSelection()?.removeAllRanges();
    clearSelection();
  };

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(code);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1400);
    } catch {
      setCopied(false);
    }
  };

  const download = () => {
    const blob = new Blob([code], { type: 'text/plain;charset=utf-8' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = 'workspace.lume';
    a.click();
    URL.revokeObjectURL(url);
  };

  return (
    <>
    <section className="code-panel" aria-label="Lume 源码输出">
      <header className="code-head">
        <span className="code-head-title">SOURCE / .LUME</span>
        <div className="code-actions">
          <button className="icon-btn" onClick={copy} aria-label="复制 Lume 源码" title="复制源码">
            {copied ? <CheckIcon /> : <CopyIcon />}
          </button>
          <button className="icon-btn" onClick={download} aria-label="下载 workspace.lume" title="下载 .lume">
            <DownloadIcon />
          </button>
        </div>
      </header>

      <div className="code-scroll" tabIndex={0}>
        <div className="code-lines">
          {lines.map((line, index) => (
            <div className="code-line" key={index}>
              <span className="line-no">{String(index + 1).padStart(2, '0')}</span>
              <span className="line-code">{line ? highlight(line) : ' '}</span>
            </div>
          ))}
        </div>
      </div>

      <footer className="code-foot">
        <span>{lines.length} LINES</span>
        <span>{blockCount} BLOCKS</span>
        <span className="code-foot-state">
          <i /> COMPILED
        </span>
      </footer>
    </section>

    {selRects.length > 0 &&
      createPortal(
        <div className="lume-sel-layer" aria-hidden="true">
          {selRects.map((rect, index) => (
            <div
              className={`lume-sel-box ${index === 0 ? 'is-top' : ''} ${
                index === selRects.length - 1 ? 'is-bottom' : ''
              }`}
              key={index}
              style={{ left: rect.left, top: rect.top, width: rect.width, height: rect.height }}
            />
          ))}
        </div>,
        document.body,
      )}

    {menuPos &&
      createPortal(
        <div className="lume-sel-menu" style={{ left: menuPos.x, top: menuPos.y }} role="menu">
          <button onClick={copySelection}>复制</button>
          <button onClick={selectAll}>全选</button>
          <button className="is-danger" onClick={clearSelectionNow}>清除</button>
        </div>,
        document.body,
      )}
    </>
  );
}

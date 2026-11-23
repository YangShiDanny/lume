import { CloseIcon } from './Icons';

const ROWS = [
  { k: '函数', v: 'fn add(a:int, b:int) -> int', note: 'ret 直接结束函数，无需 end' },
  { k: '声明', v: 'total:int = 0', note: '声明必须带初始值' },
  { k: '赋值', v: 'total = 10  /  total := 10', note: '后者是表达式，有返回值' },
  { k: '条件', v: 'if cond ... else ... end', note: 'if 可作为块表达式赋值' },
  { k: '循环', v: 'while i <= n ... end', note: '迭代保护上限 100000' },
  { k: '运算', v: '+ - * / %  == != < <= > >=', note: 'and / or / not / is' },
  { k: '字面量', v: '10  2.0  "文本"  true  [1,2]', note: '# 为单行注释' },
  { k: '内置', v: 'print(len(values))', note: 'sin/cos/sqrt/pow/log...' },
];

export function SyntaxSheet({ onClose }: { onClose: () => void }) {
  return (
    <div className="sheet-backdrop" onClick={onClose}>
      <section className="sheet" onClick={(event) => event.stopPropagation()} aria-label="Lume 语法速览">
        <header className="sheet-head">
          <div>
            <div className="eyebrow">REFERENCE / 00</div>
            <h2>Lume 语法速览</h2>
          </div>
          <button className="icon-btn" onClick={onClose} aria-label="关闭语法速览">
            <CloseIcon />
          </button>
        </header>
        <div className="sheet-body">
          {ROWS.map((row) => (
            <div className="sheet-row" key={row.k}>
              <span className="sheet-key">{row.k}</span>
              <code>{row.v}</code>
              <span className="sheet-note">{row.note}</span>
            </div>
          ))}
        </div>
        <footer className="sheet-foot">依据 README.md 与 docs/lume_spec.md · 当前仅生成源码，不执行</footer>
      </section>
    </div>
  );
}

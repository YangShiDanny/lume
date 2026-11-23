import { useEffect, useState } from 'react';

const STEPS = [
  {
    sel: '.menubar',
    title: '菜单栏',
    text: '主题、语法速览和窗口状态都在这里。当前版本只生成 Lume 源码，不接运行时。',
  },
  {
    sel: '.blockly-host',
    title: '积木工作区',
    text: '从左侧工具箱拖出积木。空白处按住左键或中键可平移，Ctrl + 滚轮缩放，右下角也有缩放控件。',
  },
  {
    sel: '.code-panel',
    title: 'Lume 源码',
    text: '积木变化会实时编译成缩进式 .lume 代码，可复制或下载。拖动中间分隔条可调整宽度。',
  },
  {
    sel: '.dock',
    title: 'Dock',
    text: '快速打开工作台、切换源码面板、查看语法速览、重置示例或切换明暗主题。',
  },
];

export function Tour({ onDone }: { onDone: () => void }) {
  const [step, setStep] = useState(0);

  useEffect(() => {
    const target = document.querySelector(STEPS[step].sel);
    target?.classList.add('tour-hi');
    return () => target?.classList.remove('tour-hi');
  }, [step]);

  const next = () => {
    if (step + 1 >= STEPS.length) onDone();
    else setStep(step + 1);
  };

  return (
    <div className="tour" aria-label="首次使用引导">
      <div className="tour-box">
        <div className="tour-title">{STEPS[step].title}</div>
        <div className="tour-text">{STEPS[step].text}</div>
        <div className="tour-dots" aria-hidden="true">
          {STEPS.map((item, index) => (
            <i className={index === step ? 'on' : ''} key={item.sel} />
          ))}
        </div>
        <div className="tour-actions">
          <button onClick={onDone}>跳过</button>
          <button className="primary" onClick={next}>
            {step + 1 === STEPS.length ? '完成' : '下一步'}
          </button>
        </div>
      </div>
    </div>
  );
}

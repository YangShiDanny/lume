import { forwardRef, useEffect, useImperativeHandle, useRef } from 'react';
import * as Blockly from 'blockly';
import {
  defineLumeBlocks,
  defineLumeThemes,
  generateLume,
  loadExample,
  lumeDarkTheme,
  lumeLightTheme,
  toolbox,
} from '../lume/blocks';

export type WorkspaceApi = {
  reset: () => void;
  zoomIn: () => void;
  zoomOut: () => void;
  fit: () => void;
};

type Props = {
  theme: 'dark' | 'light';
  onChange: (code: string, blockCount: number) => void;
};

export const BlocklyWorkspace = forwardRef<WorkspaceApi, Props>(function BlocklyWorkspace(
  { theme, onChange },
  ref,
) {
  const hostRef = useRef<HTMLDivElement>(null);
  const workspaceRef = useRef<Blockly.WorkspaceSvg | null>(null);
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;

  useImperativeHandle(ref, () => ({
    reset: () => {
      const ws = workspaceRef.current;
      if (!ws) return;
      loadExample(ws);
      ws.zoomCenter(0);
    },
    zoomIn: () => workspaceRef.current?.zoomCenter(1),
    zoomOut: () => workspaceRef.current?.zoomCenter(-1),
    fit: () => workspaceRef.current?.zoomCenter(0),
  }));

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    defineLumeThemes();
    defineLumeBlocks();

    const workspace = Blockly.inject(host, {
      toolbox,
      theme: theme === 'light' ? lumeLightTheme : lumeDarkTheme,
      renderer: 'zelos',
      scrollbars: true,
      trashcan: true,
      sounds: false,
      grid: {
        spacing: 20,
        length: 3,
        colour: theme === 'light' ? '#d4d4d8' : '#26262d',
        snap: false,
      },
      zoom: {
        controls: true,
        wheel: true,
        startScale: 0.92,
        maxScale: 2.4,
        minScale: 0.35,
        scaleSpeed: 1.16,
      },
      // 平移由下面的背景拖拽实现；滚轮滚动交给 scrollbars。
      move: {
        drag: false,
        wheel: true,
        scrollbars: true,
      },
    }) as Blockly.WorkspaceSvg;

    workspaceRef.current = workspace;
    loadExample(workspace);

    // 在空白画布上按住左键/中键拖动即可平移工作区。
    const svg = workspace.getCanvas().ownerSVGElement;
    let pan: { id: number; x: number; y: number; sx: number; sy: number } | null = null;
    const onPanDown = (e: PointerEvent) => {
      if (e.button !== 0 && e.button !== 1) return;
      const target = e.target as Element | null;
      const isBackground =
        target === svg ||
        target?.classList?.contains('blocklyMainBackground') ||
        target?.classList?.contains('blocklySvg');
      if (!isBackground) return;
      e.preventDefault();
      e.stopPropagation();
      pan = {
        id: e.pointerId,
        x: e.clientX,
        y: e.clientY,
        sx: workspace.scrollX,
        sy: workspace.scrollY,
      };
      svg?.setPointerCapture(e.pointerId);
    };
    const onPanMove = (e: PointerEvent) => {
      if (!pan || pan.id !== e.pointerId) return;
      const scale = workspace.scale || 1;
      workspace.scroll(pan.sx + (e.clientX - pan.x) / scale, pan.sy + (e.clientY - pan.y) / scale, false);
    };
    const onPanEnd = (e: PointerEvent) => {
      if (!pan || pan.id !== e.pointerId) return;
      pan = null;
      if (svg?.hasPointerCapture(e.pointerId)) svg.releasePointerCapture(e.pointerId);
    };
    svg?.addEventListener('pointerdown', onPanDown, true);
    svg?.addEventListener('pointermove', onPanMove, true);
    svg?.addEventListener('pointerup', onPanEnd, true);
    svg?.addEventListener('pointercancel', onPanEnd, true);

    let timer = 0;
    const emit = () => {
      const code = generateLume(workspace);
      onChangeRef.current(code, workspace.getAllBlocks(false).length);
    };

    const listener = (event: Blockly.Events.Abstract) => {
      if ((event as Blockly.Events.Abstract & { isUiEvent?: boolean }).isUiEvent) return;
      window.clearTimeout(timer);
      timer = window.setTimeout(emit, 40);
    };
    workspace.addChangeListener(listener);
    emit();

    const resize = new ResizeObserver(() => {
      Blockly.svgResize(workspace);
    });
    resize.observe(host);

    return () => {
      window.clearTimeout(timer);
      resize.disconnect();
      svg?.removeEventListener('pointerdown', onPanDown, true);
      svg?.removeEventListener('pointermove', onPanMove, true);
      svg?.removeEventListener('pointerup', onPanEnd, true);
      svg?.removeEventListener('pointercancel', onPanEnd, true);
      workspace.removeChangeListener(listener);
      workspace.dispose();
      workspaceRef.current = null;
    };
    // 主题通过下面的独立 effect 切换，工作区只创建一次。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const ws = workspaceRef.current;
    if (!ws) return;
    ws.setTheme(theme === 'light' ? lumeLightTheme : lumeDarkTheme);
  }, [theme]);

  return <div className="blockly-host" ref={hostRef} aria-label="Lume 积木工作区" />;
});

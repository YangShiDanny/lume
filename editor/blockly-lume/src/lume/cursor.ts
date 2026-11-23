/* LumeOS 风格自绘白点光标。节点由运行时挂到 body 末尾。 */

(function cursor() {
  'use strict';
  if (window.matchMedia('(pointer: coarse)').matches) return;

  function mount(id: string): HTMLDivElement {
    const node = document.createElement('div');
    node.id = id;
    node.setAttribute('aria-hidden', 'true');
    document.body.appendChild(node);
    return node;
  }

  const dot = mount('cursor');
  const ring = mount('cursor-ring');

  let mx = window.innerWidth / 2;
  let my = window.innerHeight / 2;
  let mode = 'idle';
  let barH = 24;
  let dragging = false;

  type CursorEvent = MouseEvent | PointerEvent;

  function syncCursor(ev?: CursorEvent | null) {
    if (!ev || ev.clientX == null) return;
    mx = ev.clientX;
    my = ev.clientY;
    dot.style.left = `${mx}px`;
    dot.style.top = `${my}px`;
    dot.classList.remove('is-hidden', 'is-away');
    ring.classList.remove('is-hidden', 'is-away');
  }
  (window as unknown as { __lumeCursorSync: (ev?: CursorEvent | null) => void }).__lumeCursorSync = syncCursor;

  const TEXT_TAGS = /^(p|h[1-6]|span|li|code|pre|label|small|strong|em|b|i|td|th|dd|dt|blockquote)$/i;

  function hasOwnText(node: Node): boolean {
    for (const child of node.childNodes) {
      if (child.nodeType === 3 && child.nodeValue && child.nodeValue.trim()) return true;
    }
    return false;
  }

  function measureBar(node: EventTarget | null): number {
    if (!(node instanceof Element)) return 0;
    const base = parseFloat(getComputedStyle(node).fontSize) || 0;
    return base ? Math.round(Math.min(Math.max(base * 1.35, 18), 56)) : 0;
  }

  function setBarFor(node: EventTarget | null) {
    const next = measureBar(node) || 26;
    if (next === barH) return;
    barH = next;
    dot.style.setProperty('--bar-h', `${barH}px`);
  }

  function setMode(next: string, node: EventTarget | null) {
    // 拖拽中强制回到白点态，避免按钮/链接的 is-link 把点藏起来。
    if (dragging) next = 'idle';
    if (next === 'text') setBarFor(node);
    if (next === mode) return;
    mode = next;
    dot.classList.toggle('is-text', next === 'text');
    dot.classList.toggle('is-link', next === 'link');
    ring.classList.toggle('is-link', next === 'link');
    ring.classList.toggle('is-text', next === 'text');
  }

  function modeFor(target: EventTarget | null): string {
    if (!(target instanceof Element)) return 'idle';
    if (
      target.closest(
        'a,button,[role="button"],summary,label,.dock-item,.desk-icon,.win-light,.mb-btn,.icon-btn,.splitter,.sheet-row'
      )
    ) return 'link';
    if (target.closest('textarea,select,input,[contenteditable="true"]')) return 'text';
    if (target.closest('.win-titlebar,.mb-clock,.menubar')) return 'idle';
    if (hasOwnText(target)) return 'text';
    if (TEXT_TAGS.test(target.tagName) && target.textContent?.trim()) return 'text';
    return 'idle';
  }

  let lastSeen = { x: mx, y: my, t: 0 };
  function onMove(e: CursorEvent) {
    lastSeen = { x: e.clientX, y: e.clientY, t: performance.now() };
    syncCursor(e);
    setMode(modeFor(e.target), e.target);
  }
  // 捕获阶段监听：Blockly 拖拽 / setPointerCapture 时事件可能不冒泡到 window。
  window.addEventListener('mousemove', onMove, { capture: true, passive: true });
  window.addEventListener('pointermove', onMove, { capture: true, passive: true });
  if ('onpointerrawupdate' in window) {
    window.addEventListener('pointerrawupdate', onMove as EventListener, { capture: true, passive: true });
  }
  window.addEventListener('mouseover', (e) => setMode(modeFor(e.target), e.target), {
    capture: true,
    passive: true,
  });
  window.addEventListener(
    'scroll',
    () => {
      const target = document.elementFromPoint(mx, my);
      if (target) setMode(modeFor(target), target);
    },
    { capture: true, passive: true }
  );

  document.addEventListener('mouseleave', () => {
    dot.classList.add('is-hidden');
    ring.classList.add('is-hidden');
  });
  document.addEventListener('mouseenter', () => {
    dot.classList.remove('is-hidden');
    ring.classList.remove('is-hidden');
  });
  const endDrag = () => {
    if (!dragging) return;
    dragging = false;
    dot.classList.remove('is-dragging');
    ring.classList.remove('is-dragging');
    const target = document.elementFromPoint(mx, my);
    setMode(modeFor(target), target);
  };
  document.addEventListener(
    'pointerdown',
    (e) => {
      syncCursor(e as PointerEvent);
      dragging = true;
      dot.classList.add('is-dragging');
      ring.classList.add('is-dragging');
      setMode('idle', null);
    },
    true
  );
  document.addEventListener('pointerup', endDrag, true);
  document.addEventListener('pointercancel', endDrag, true);

  const isFrame = (node: EventTarget | null) => node instanceof Element && node.tagName === 'IFRAME';
  document.addEventListener(
    'pointerover',
    (e) => {
      if (isFrame(e.target)) {
        dot.classList.add('is-away');
        ring.classList.add('is-away');
      }
    },
    true
  );
  document.addEventListener(
    'pointerout',
    (e) => {
      if (isFrame(e.target)) {
        dot.classList.remove('is-away');
        ring.classList.remove('is-away');
      }
    },
    true
  );

  let ringX = mx;
  let ringY = my;
  let vx = 0;
  let vy = 0;
  let lastT = performance.now();
  const reduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

  function followRing(now: number) {
    const t = now || performance.now();
    const dt = Math.min((t - lastT) / 16.667, 3);
    lastT = t;
    vx = (vx + (mx - ringX) * 0.24) * 0.72;
    vy = (vy + (my - ringY) * 0.24) * 0.72;
    ringX += vx * dt;
    ringY += vy * dt;
    ring.style.left = `${ringX}px`;
    ring.style.top = `${ringY}px`;

    if (!reduced && !dragging) {
      const speed = Math.min(Math.hypot(vx, vy) / 26, 1);
      const stretch = (1 + speed * 0.32).toFixed(3);
      const shrink = (1 - speed * 0.12).toFixed(3);
      const angle = `${((Math.atan2(vy, vx) * 180) / Math.PI).toFixed(1)}deg`;
      const target = mode === 'link' ? ring : dot;
      target.style.setProperty('--stretch', stretch);
      target.style.setProperty('--shrink', shrink);
      target.style.setProperty('--angle', angle);
    }
    requestAnimationFrame(followRing);
  }
  requestAnimationFrame(followRing);

  void lastSeen;
})();

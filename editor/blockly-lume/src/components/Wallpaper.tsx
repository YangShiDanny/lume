import { useEffect, useRef } from 'react';

type Star = { x: number; y: number; r: number; phase: number; speed: number };

export function Wallpaper({ theme }: { theme: 'dark' | 'light' }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const themeRef = useRef(theme);
  themeRef.current = theme;

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    let stars: Star[] = [];
    let frame = 0;
    let raf = 0;
    let width = 0;
    let height = 0;
    const reduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

    const resize = () => {
      const dpr = Math.min(window.devicePixelRatio || 1, 2);
      width = window.innerWidth;
      height = window.innerHeight;
      canvas.width = Math.floor(width * dpr);
      canvas.height = Math.floor(height * dpr);
      canvas.style.width = `${width}px`;
      canvas.style.height = `${height}px`;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

      let seed = 20261122;
      const rand = () => {
        seed = (seed * 1664525 + 1013904223) >>> 0;
        return seed / 4294967296;
      };
      const count = Math.min(320, Math.max(90, Math.floor((width * height) / 6200)));
      stars = Array.from({ length: count }, () => ({
        x: rand() * width,
        y: rand() * height,
        r: 0.35 + rand() * 1.05,
        phase: rand() * Math.PI * 2,
        speed: 0.0004 + rand() * 0.0012,
      }));
      draw();
    };

    const draw = () => {
      const isLight = themeRef.current === 'light';
      ctx.clearRect(0, 0, width, height);
      for (const star of stars) {
        const alpha = reduced
          ? 0.42
          : 0.22 + 0.38 * (0.5 + 0.5 * Math.sin(frame * star.speed + star.phase));
        ctx.beginPath();
        ctx.arc(star.x, star.y, star.r, 0, Math.PI * 2);
        ctx.fillStyle = isLight ? `rgba(60, 60, 85, ${alpha})` : `rgba(255, 255, 255, ${alpha})`;
        ctx.fill();
      }
    };

    const loop = () => {
      frame += 1;
      draw();
      raf = requestAnimationFrame(loop);
    };

    resize();
    window.addEventListener('resize', resize);
    if (!reduced) raf = requestAnimationFrame(loop);

    return () => {
      window.removeEventListener('resize', resize);
      cancelAnimationFrame(raf);
    };
  }, [theme]);

  return <canvas className="wallpaper" ref={canvasRef} aria-hidden="true" />;
}

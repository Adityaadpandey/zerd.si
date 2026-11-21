"use client";
import { useEffect, useRef } from "react";

// The zerd wordmark drawn the way the film draws everything: a grid of hex digits, white with a few blue ones.
// On load the digits flicker and settle into the letters. After that they shimmer a little, and the ones
// near the pointer scramble. The real text is in the page for screen readers (see the h1 in page.tsx).
const CHARS = "01xX>$1011abcdef";
const BLUE = "#2f6bff";

type Cell = { x: number; y: number; ch: string; blue: boolean; born: number };

export default function HexWord({ text = "zerd" }: { text?: string }) {
  const box = useRef<HTMLDivElement>(null);
  const cvs = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const wrap = box.current, canvas = cvs.current;
    if (!wrap || !canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const reduced = matchMedia("(prefers-reduced-motion: reduce)").matches;
    const rnd = () => CHARS[(Math.random() * CHARS.length) | 0];
    let cells: Cell[] = [], cell = 10, ch = 14, W = 0, H = 0, dpr = 1, raf = 0, visible = true, t0 = 0;
    const ptr = { x: -999, y: -999 };

    const build = () => {
      W = wrap.clientWidth; H = wrap.clientHeight;
      if (!W || !H) return;
      dpr = Math.min(window.devicePixelRatio || 1, 2);
      canvas.width = W * dpr; canvas.height = H * dpr;
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      const family = getComputedStyle(canvas).fontFamily;
      const off = document.createElement("canvas");
      off.width = W; off.height = H;
      const o = off.getContext("2d", { willReadFrequently: true })!;
      o.font = `800 100px ${family}`;
      const m = o.measureText(text);
      const size = Math.min((100 * W * 0.96) / m.width, H * 1.05);
      o.font = `800 ${size}px ${family}`;
      o.textAlign = "center"; o.textBaseline = "alphabetic";
      o.fillStyle = "#fff";
      o.fillText(text, W / 2, H * 0.8);
      const data = o.getImageData(0, 0, W, H).data;
      cell = Math.max(5, W / 128); ch = cell * 1.6;
      cells = [];
      for (let y = ch / 2; y < H; y += ch) {
        for (let x = cell / 2; x < W; x += cell) {
          if (data[((y | 0) * W + (x | 0)) * 4 + 3] > 120) {
            cells.push({ x, y, ch: rnd(), blue: Math.random() < 0.14, born: Math.random() * 1100 });
          }
        }
      }
      t0 = performance.now();
    };

    const draw = (now: number) => {
      const t = now - t0;
      ctx.clearRect(0, 0, W, H);
      ctx.font = `700 ${ch * 0.8}px ${getComputedStyle(canvas).fontFamily}`;
      ctx.textAlign = "center"; ctx.textBaseline = "middle";
      const r2 = 110 * 110;
      for (const c of cells) {
        const age = t - c.born;
        if (age < 0) continue;
        const dx = c.x - ptr.x, dy = c.y - ptr.y;
        const near = dx * dx + dy * dy < r2;
        const deciding = age < 420;
        if (deciding || near ? Math.random() < 0.5 : Math.random() < 0.004) c.ch = rnd();
        ctx.fillStyle = deciding ? (Math.random() < 0.5 ? BLUE : "#fff") : near ? "#9fbcff" : c.blue ? BLUE : "#f4f6fb";
        ctx.globalAlpha = deciding ? 0.55 + 0.45 * (age / 420) : 1;
        ctx.fillText(c.ch, c.x, c.y);
      }
      ctx.globalAlpha = 1;
    };

    const loop = (now: number) => {
      draw(now);
      if (visible) raf = requestAnimationFrame(loop);
    };
    const start = () => { cancelAnimationFrame(raf); if (reduced) { draw(t0 + 5000); } else raf = requestAnimationFrame(loop); };

    const move = (e: PointerEvent) => { const r = canvas.getBoundingClientRect(); ptr.x = e.clientX - r.left; ptr.y = e.clientY - r.top; };
    const leave = () => { ptr.x = ptr.y = -999; };
    const ro = new ResizeObserver(() => { build(); start(); });
    const io = new IntersectionObserver(([en]) => { visible = en.isIntersecting; if (visible) start(); });
    ro.observe(wrap); io.observe(wrap);
    window.addEventListener("pointermove", move, { passive: true });
    document.addEventListener("pointerleave", leave);
    // draw once the web font is ready, so the mask uses the real letterforms
    document.fonts?.ready.then(() => { build(); start(); });
    return () => { cancelAnimationFrame(raf); ro.disconnect(); io.disconnect(); window.removeEventListener("pointermove", move); document.removeEventListener("pointerleave", leave); };
  }, [text]);

  return (
    <div className="hexword" ref={box} aria-hidden="true">
      <canvas ref={cvs} />
    </div>
  );
}

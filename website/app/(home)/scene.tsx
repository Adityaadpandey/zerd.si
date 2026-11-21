"use client";
import Link from "next/link";
import { useEffect, useRef } from "react";
import CopyCmd from "./copy-cmd";
import HexWord from "./hex-word";
import Models from "./models";

// The opening is your video, scrubbed with the scroll wheel. public/scrub/000..069.webp are its 70 frames.
// Scroll position (p, 0 to 1) picks the frame, and every statement is its own full screen on the same timeline.
// Frames: 0 black hex hands, 1-15 the painting, 16-21 it dissolves to hex, 22-50 hands reach, 51-62 spark, 63-69 white.
const N = 70;
const KEYS: [number, number][] = [[0, 0], [0.06, 0], [0.1, 1], [0.27, 15], [0.33, 22], [0.6, 50], [0.8, 62], [0.88, 69], [1, 69]];
const WIN = "irm https://www.zerd.si/install.ps1 | iex";
const STEP_A = 0.42, STEP_W = 0.063;
// Where each statement is fully in focus. The page settles on these, so a small scroll moves one whole screen.
const SNAPS = [0, 0.19, 0.35, ...Array.from({ length: 6 }, (_, i) => STEP_A + (i + 0.5) * STEP_W), 0.97];

const frameAt = (p: number) => {
  for (let i = 1; i < KEYS.length; i++) {
    if (p <= KEYS[i][0]) {
      const [p0, f0] = KEYS[i - 1], [p1, f1] = KEYS[i];
      return p1 === p0 ? f1 : f0 + ((p - p0) / (p1 - p0)) * (f1 - f0);
    }
  }
  return N - 1;
};
const win = (p: number, a: number, b: number, f = 0.035) => Math.max(0, Math.min(1, Math.min((p - a) / f, (b - p) / f)));
const ease = (x: number) => x * x * (3 - 2 * x);

export default function Scene({ install, steps, version }: { install: string; steps: [string, string, string?][]; version?: string }) {
  const film = useRef<HTMLElement>(null);
  const cvs = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const root = film.current, canvas = cvs.current;
    if (!root || !canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    if (matchMedia("(prefers-reduced-motion: reduce)").matches) return; // the static layout in CSS takes over
    root.classList.add("armed");

    const frames: (HTMLImageElement | undefined)[] = new Array(N);
    const fx = Array.from(root.querySelectorAll<HTMLElement>("[data-a]"));
    let W = 0, H = 0, dpr = 1, p = 0, target = 0, need = true, raf = 0, on = true, lastFrame = -1;

    const size = () => {
      dpr = Math.min(window.devicePixelRatio || 1, 2);
      W = canvas.clientWidth; H = canvas.clientHeight;
      canvas.width = W * dpr; canvas.height = H * dpr;
      need = true;
    };
    const measure = () => {
      const r = root.getBoundingClientRect();
      target = Math.max(0, Math.min(1, -r.top / (r.height - innerHeight)));
    };

    const nearest = (f: number) => {
      // nearest frame that has loaded, so a slow network shows an older frame instead of a blank
      let im = frames[f];
      for (let d = 1; !im && d < N; d++) im = frames[f - d] ?? frames[f + d];
      return im;
    };
    const paint = () => {
      // The film has 24 frames a second and a scroll step is smaller than one frame, so two neighbouring
      // frames are blended by the fractional part. That is what makes it glide instead of step.
      const fl = frameAt(p), a = Math.floor(fl), t = fl - a;
      const key = fl * 100 + (frames.filter(Boolean).length << 20);
      if (key !== lastFrame || need) {
        lastFrame = key; need = false;
        const im = nearest(a);
        if (im) {
          ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
          ctx.imageSmoothingEnabled = true; ctx.imageSmoothingQuality = "high";
          ctx.globalAlpha = 1; ctx.fillStyle = "#000"; ctx.fillRect(0, 0, W, H);
          const fit = W / im.naturalWidth, cover = Math.max(fit, H / im.naturalHeight);
          const s = cover;
          const w = im.naturalWidth * s, h = im.naturalHeight * s, x = (W - w) / 2, y = (H - h) / 2 - (H > W ? H * 0.06 : 0);
          ctx.drawImage(im, x, y, w, h);
          const nx = a < N - 1 && t > 0.01 ? nearest(a + 1) : undefined;
          if (nx) { ctx.globalAlpha = t; ctx.drawImage(nx, x, y, w, h); ctx.globalAlpha = 1; }
        }
      }
      // every statement fades and blurs in, holds, then blurs out; its second line sharpens as you scroll into it
      for (const el of fx) {
        const A = +el.dataset.a!, B = +el.dataset.b!, f = +(el.dataset.f ?? 0.035);
        const ein = Math.max(0, Math.min(1, (p - A) / f)), eout = Math.max(0, Math.min(1, (B - p) / f));
        const o = Math.min(ein, eout), q = Math.max(0, Math.min(1, (p - A) / (B - A)));
        el.style.setProperty("--o", String(ease(o)));
        el.style.setProperty("--y", `${(1 - ein) * 40 - (1 - eout) * 40}px`);
        el.style.setProperty("--s", String(ease(Math.max(0, Math.min(1, (q - 0.04) / 0.4)))));
        el.style.visibility = o > 0.01 ? "visible" : "hidden";
      }
      const lift = ease(Math.max(0, Math.min(1, (p - 0.78) / 0.07)));
      canvas.style.filter = `brightness(${0.7 + 0.3 * lift - 0.2 * win(p, -1, 0.1)})`;
      canvas.style.opacity = String(1 - ease(Math.max(0, Math.min(1, (p - 0.88) / 0.05))));
    };

    const tick = () => {
      p += (target - p) * 0.1;
      if (Math.abs(target - p) < 0.0004) p = target;
      paint();
      raf = p === target ? 0 : requestAnimationFrame(tick);
    };
    const kick = () => { measure(); if (!raf && on) raf = requestAnimationFrame(tick); };

    const load = (i: number) => new Promise<void>((ok) => {
      const im = new Image();
      im.decoding = "async";
      im.onload = () => { frames[i] = im; ok(); };
      im.onerror = () => ok();
      im.src = `/scrub/${String(i).padStart(3, "0")}.webp`;
    });
    // First the frames you reach first, then the rest, a few at a time.
    (async () => {
      const order = [0, 1, ...Array.from({ length: N - 2 }, (_, k) => k + 2)];
      let at = 0;
      const worker = async () => { while (at < order.length) { await load(order[at++]); need = true; kick(); } };
      await Promise.all([worker(), worker(), worker(), worker()]);
    })();

    // ---- snapping: one nudge of the wheel, a key or a swipe goes to the next whole statement ----
    let busy = false, idle = 0, lastY = scrollY, dirY = 0;
    const span = () => root.offsetHeight - innerHeight;
    const topY = () => root.getBoundingClientRect().top + scrollY;
    const inside = () => { const t = topY(); return scrollY >= t - 2 && scrollY <= t + span() + 2; };
    const stop = () => { const q = (scrollY - topY()) / span(); let k = 0; for (let i = 1; i < SNAPS.length; i++) if (Math.abs(SNAPS[i] - q) < Math.abs(SNAPS[k] - q)) k = i; return k; };
    const go = (k: number) => {
      const from = scrollY, to = topY() + SNAPS[k] * span(), t0 = performance.now(), dur = Math.min(1100, 520 + Math.abs(to - from) * 0.12);
      busy = true;
      const step = (now: number) => {
        const x = Math.min(1, (now - t0) / dur), e = x < 0.5 ? 4 * x * x * x : 1 - Math.pow(-2 * x + 2, 3) / 2;
        window.scrollTo({ top: from + (to - from) * e, behavior: "instant" });
        if (x < 1) requestAnimationFrame(step); else { lastY = scrollY; setTimeout(() => { busy = false; }, 120); }
      };
      requestAnimationFrame(step);
    };
    const move = (dir: number, e?: Event) => {
      if (!inside()) return;
      const k = stop(), q = (scrollY - topY()) / span();
      // leaving the film at either end hands the scroll back to the page
      if ((dir > 0 && k === SNAPS.length - 1 && q >= SNAPS[k] - 0.004) || (dir < 0 && k === 0 && q <= 0.004)) return;
      e?.preventDefault();
      if (busy) return;
      // if the page sits between two stops, a nudge goes to the one that lies in the direction you pushed
      let n = k;
      if (dir > 0) n = SNAPS[k] > q + 0.004 ? k : Math.min(k + 1, SNAPS.length - 1);
      else n = SNAPS[k] < q - 0.004 ? k : Math.max(k - 1, 0);
      go(n);
    };
    const onWheel = (e: WheelEvent) => { if (Math.abs(e.deltaY) < 3 || e.ctrlKey) return; move(Math.sign(e.deltaY), e); };
    const onKey = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLElement && /^(INPUT|TEXTAREA|SELECT|BUTTON|A)$/.test(e.target.tagName) && e.key === " ") return;
      if (["ArrowDown", "PageDown", " "].includes(e.key) && !e.shiftKey) move(1, e);
      else if (["ArrowUp", "PageUp"].includes(e.key) || (e.key === " " && e.shiftKey)) move(-1, e);
    };
    // touch, scrollbar drags and anything else: once it stops moving, settle on the stop in the direction it was going
    const settle = () => {
      if (busy || !inside()) return;
      const q = (scrollY - topY()) / span(), k = stop();
      if (k === SNAPS.length - 1 && q >= SNAPS[k]) return; // past the last stop the page is free to scroll on
      let n = k;
      // a quarter of the way toward the next stop is enough to commit to it; less than that settles back
      if (dirY > 0 && k < SNAPS.length - 1 && q > SNAPS[k] + 0.25 * (SNAPS[k + 1] - SNAPS[k])) n = k + 1;
      else if (dirY > 0 && k < SNAPS.length - 1 && SNAPS[k] < q) n = k;
      else if (dirY < 0 && k > 0 && q < SNAPS[k] - 0.25 * (SNAPS[k] - SNAPS[k - 1])) n = k - 1;
      if (Math.abs(scrollY - (topY() + SNAPS[n] * span())) > 2) go(n);
    };
    const onScroll = () => { dirY = Math.sign(scrollY - lastY) || dirY; lastY = scrollY; if (busy) return; clearTimeout(idle); idle = window.setTimeout(settle, 140); };
    window.addEventListener("wheel", onWheel, { passive: false });
    window.addEventListener("keydown", onKey);
    window.addEventListener("scroll", onScroll, { passive: true });

    const io = new IntersectionObserver(([en]) => { on = en.isIntersecting; if (on) kick(); }, { rootMargin: "100px" });
    io.observe(root);
    const onResize = () => { size(); kick(); };
    size(); measure(); p = target; paint();
    window.addEventListener("scroll", kick, { passive: true });
    window.addEventListener("resize", onResize);
    return () => { window.removeEventListener("wheel", onWheel); window.removeEventListener("keydown", onKey); window.removeEventListener("scroll", onScroll); clearTimeout(idle); cancelAnimationFrame(raf); io.disconnect(); window.removeEventListener("scroll", kick); window.removeEventListener("resize", onResize); root.classList.remove("armed"); };
  }, []);

  return (
    <section className="film" ref={film} aria-label="zerd">
      <span id="how" className="anchor" style={{ top: "50%" }} />
      <div className="reel">
        {/* the first frame is real markup, so something is on screen before any script runs */}
        {/* eslint-disable-next-line @next/next/no-img-element */}
        <img className="poster" src="/scrub/000.webp" alt="" width={1600} height={903} fetchPriority="high" draggable={false} />
        <canvas ref={cvs} aria-hidden="true" />
        <div className="scrim" aria-hidden="true" />

        <div className="beat hero-beat" data-a="-1" data-b="0.1">
          <h1>
            <HexWord text="Zerd." />
            <span className="sr-only">Zerd. An AI that writes code in your terminal, then checks its own work.</span>
          </h1>
          <p className="tag">An AI that writes code in your terminal, then checks its own work.</p>
          <div className="hero-install">
            <CopyCmd cmd={install} winCmd={WIN} />
            <p className="hero-links">
              <Link href="/docs/quickstart">Get started</Link>
              <Link href="/how-it-works">How it works</Link>
            </p>
            <Models />
          </div>
        </div>

        <div className="beat stmt" data-a="0.1" data-b="0.275">
          <p className="t1">AI can write code.</p>
          <p className="t2">But is it right?</p>
        </div>

        <div className="beat stmt" data-a="0.285" data-b="0.41">
          <p className="t1">So we built zerd.</p>
          <p className="t2 small">An AI coding tool that checks its own work before you ever look at it.</p>
        </div>

        {steps.map(([t, d, ev], i) => (
          <div className="beat stmt step" key={t} data-a={STEP_A + i * STEP_W} data-b={STEP_A + (i + 1) * STEP_W} data-f="0.018">
            <h2 className="t1">{t}.</h2>
            <p className="t2">{d}</p>
            {ev && <code>{ev}</code>}
          </div>
        ))}

        <div className="beat close" data-a="0.93" data-b="2" id="start">
          <div className="close-copy">
            <h2>Read a short report, not the whole change.</h2>
            <p>Every run ends with what changed, what was tested, and what zerd couldn’t check. Don’t like it? <code>/undo</code> puts every file back.</p>
            <p className="links">
              <Link href="/docs/quickstart">Get started</Link>
              <Link href="/how-it-works">How it works</Link>
            </p>
          </div>
          <figure className="close-shot">
            <div className="close-win">
              <div className="close-bar" aria-hidden="true"><i /><i /><i /><span>zerd · a real recording</span></div>
              {/* eslint-disable-next-line @next/next/no-img-element */}
              <img src="/shots/close.webp" alt="zerd’s real output after a fix: an explanation of the change, 3 tests passed, an unverified status line, then /verify running the check, which passes. Side panels list the files read, the file written and the commands run." width={1800} height={1227} loading="lazy" draggable={false} />
            </div>
            <figcaption>A real zerd run, after <code>/verify</code>. The panels on the right list what it read, wrote and ran.</figcaption>
          </figure>
        </div>
      </div>
    </section>
  );
}

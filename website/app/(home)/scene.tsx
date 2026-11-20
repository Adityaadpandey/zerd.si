"use client";
import Link from "next/link";
import { useEffect, useRef } from "react";
import CopyCmd from "./copy-cmd";
import HexWord from "./hex-word";
import { STEP_ICONS } from "./icons";
import Models from "./models";

// The opening is a film you scrub with the scroll wheel. public/scrub/000..069.webp are the 70 frames of
// the video; scroll position picks the frame, and the words fade in and out on the same timeline (p, 0 to 1).
// Frames: 0 black hex hands, 1-15 the painting, 16-21 it dissolves to hex, 22-50 hands reach, 51-62 spark, 63-69 white.
const N = 70;
const KEYS: [number, number][] = [[0, 0], [0.06, 0], [0.1, 1], [0.27, 15], [0.33, 22], [0.6, 50], [0.8, 62], [0.88, 69], [1, 69]];
const STEPS_FROM = 0.48, STEPS_TO = 0.8;

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

export default function Scene({ install, steps }: { install: string; steps: [string, string, string?][] }) {
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
    const steps_ = Array.from(root.querySelectorAll<HTMLElement>(".step"));
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
          const s = fit + (cover - fit) * ease(Math.max(0, Math.min(1, (p - 0.55) / 0.25)));
          const w = im.naturalWidth * s, h = im.naturalHeight * s, x = (W - w) / 2, y = (H - h) / 2 - (H > W ? H * 0.06 : 0);
          ctx.drawImage(im, x, y, w, h);
          const nx = a < N - 1 && t > 0.01 ? nearest(a + 1) : undefined;
          if (nx) { ctx.globalAlpha = t; ctx.drawImage(nx, x, y, w, h); ctx.globalAlpha = 1; }
        }
      }
      // words and the picture itself fade on the same clock
      for (const el of fx) {
        const o = win(p, +el.dataset.a!, +el.dataset.b!);
        el.style.setProperty("--o", String(o));
        el.style.setProperty("--y", `${(1 - o) * 26}px`);
        el.style.visibility = o > 0.01 ? "visible" : "hidden";
      }
      const k = Math.floor(((p - STEPS_FROM) / (STEPS_TO - STEPS_FROM)) * steps_.length);
      canvas.style.filter = `brightness(${1 - 0.55 * win(p, -1, 0.1)})`;
      steps_.forEach((el, i) => el.classList.toggle("on", i === Math.max(0, Math.min(steps_.length - 1, k)) && p >= STEPS_FROM - 0.02));
      canvas.style.opacity = String(1 - ease(Math.max(0, Math.min(1, (p - 0.87) / 0.05))));
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

    const io = new IntersectionObserver(([en]) => { on = en.isIntersecting; if (on) kick(); }, { rootMargin: "100px" });
    io.observe(root);
    const onResize = () => { size(); kick(); };
    size(); measure(); p = target; paint();
    window.addEventListener("scroll", kick, { passive: true });
    window.addEventListener("resize", onResize);
    return () => { cancelAnimationFrame(raf); io.disconnect(); window.removeEventListener("scroll", kick); window.removeEventListener("resize", onResize); root.classList.remove("armed"); };
  }, []);

  return (
    <section className="film" ref={film} aria-label="zerd">
      <span id="how" className="anchor" style={{ top: "47%" }} />
      <div className="reel">
        {/* the first frame is real markup, so something is on screen before any script runs */}
        {/* eslint-disable-next-line @next/next/no-img-element */}
        <img className="poster" src="/scrub/000.webp" alt="" width={1276} height={720} fetchPriority="high" draggable={false} />
        <canvas ref={cvs} aria-hidden="true" />
        <div className="scrim" aria-hidden="true" />

        <div className="beat hero-beat" data-a="-1" data-b="0.1">
          <h1>
            <HexWord />
            <span className="sr-only">zerd. Your agent says “done.” zerd shows the proof.</span>
          </h1>
          <p className="tag">The terminal coding agent that shows the proof.</p>
          <p className="cue" aria-hidden="true">Scroll<i /></p>
        </div>

        <div className="beat line" data-a="0.1" data-b="0.285">
          <p className="big">Your agent says “done.”</p>
          <p className="sub">That’s a claim, not a result.</p>
        </div>

        <div className="beat line" data-a="0.325" data-b="0.47">
          <p className="big">zerd shows the proof.</p>
          <p className="sub">It runs tests on what it changed, has a second AI read the real diff, and tells you what it couldn’t verify.</p>
        </div>

        <div className="beat steps" data-a="0.45" data-b="0.84">
          <h2>Every change comes with receipts</h2>
          <ol>
            {steps.map(([t, d, ev], i) => (
              <li className="step" key={t}>
                <span className="ic">{STEP_ICONS[i]}</span>
                <div>
                  <h3>{t}</h3>
                  <div className="more">
                    <p>{d}</p>
                    {ev && <code>{ev}</code>}
                  </div>
                </div>
              </li>
            ))}
          </ol>
        </div>

        <div className="beat close" data-a="0.93" data-b="2" id="start">
          <h2>You read a short receipt, not the whole change.</h2>
          <div className="receipt" role="img" aria-label="Example receipt: 1 file verified, 2 checks ok, /undo reverts">
            <p><b>✓</b> edited<span>1 file</span></p>
            <p><b>✓</b> tests<span>npx tsc --noEmit · ran</span></p>
            <p><b>✓</b> review<span>second model read the diff</span></p>
            <p><b>✓</b> claims<span>every path and line found</span></p>
            <p className="sum">verified · 2 checks ok · /undo reverts</p>
          </div>
          <CopyCmd cmd={install} />
          <p className="links">
            <Link href="/docs/quickstart">Try it in two minutes</Link>
            <a href="#proof">See it beat a mock test</a>
          </p>
          <Models />
          <p className="fine">Free to run on macOS, Linux and Windows.</p>
        </div>
      </div>
    </section>
  );
}

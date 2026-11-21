"use client";
import { useEffect, useRef, useState } from "react";
import { Check, Cross } from "../icons";

/* ───────── 1. Walk through a real run, with the real screenshots ───────── */

export type Pin = { x: number; y: number; label: string };
export type Shot = { title: string; one: string; more: string; img: string; alt: string; pins: Pin[] };

export function Walkthrough({ shots }: { shots: Shot[] }) {
  const [on, setOn] = useState(0);
  const [pin, setPin] = useState(0);
  const panels = useRef<(HTMLElement | null)[]>([]);

  useEffect(() => {
    const io = new IntersectionObserver((es) => {
      for (const e of es) if (e.isIntersecting) { setOn(Number((e.target as HTMLElement).dataset.i)); setPin(0); }
    }, { rootMargin: "-42% 0px -42% 0px" });
    panels.current.forEach((el) => el && io.observe(el));
    return () => io.disconnect();
  }, []);

  const go = (i: number) => {
    const k = Math.max(0, Math.min(shots.length - 1, i));
    panels.current[k]?.scrollIntoView({ behavior: "smooth", block: "center" });
    setOn(k); setPin(0);
  };
  const s = shots[on];

  return (
    <div className="wt">
      <div className="wt-text">
        {shots.map((h, i) => (
          <article key={h.title} data-i={i} ref={(el) => { panels.current[i] = el; }} className={`wt-panel ${i === on ? "on" : ""}`} onClick={() => go(i)}>
            <p className="kicker">Step {i + 1} of {shots.length}</p>
            <h3>{h.title}</h3>
            <p className="one">{h.one}</p>
            <p className="more">{h.more}</p>
            {/* on small screens the picture sits inside the step */}
            {/* eslint-disable-next-line @next/next/no-img-element */}
            <img className="wt-inline" src={h.img} alt={h.alt} loading="lazy" width={1200} height={818} />
          </article>
        ))}
      </div>

      <div className="wt-view" tabIndex={0} aria-label="Screenshot of zerd for the selected step. Use the arrow keys to change step."
        onKeyDown={(e) => { if (e.key === "ArrowRight" || e.key === "ArrowDown") { e.preventDefault(); go(on + 1); } if (e.key === "ArrowLeft" || e.key === "ArrowUp") { e.preventDefault(); go(on - 1); } }}>
        <div className="wt-win">
          <div className="wt-bar" aria-hidden="true"><i /><i /><i /><span>zerd · a real recording</span></div>
          <div className="wt-stage">
            {shots.map((h, i) => (
              // eslint-disable-next-line @next/next/no-img-element
              <img key={h.img} className={i === on ? "on" : ""} src={h.img} alt={i === on ? h.alt : ""} aria-hidden={i !== on} width={1200} height={818} draggable={false} />
            ))}
            {s.pins.map((p, i) => (
              <button key={`${on}-${i}`} type="button" className={`pin ${pin === i ? "open" : ""}`} style={{ left: `${p.x}%`, top: `${p.y}%` }}
                onClick={() => setPin(i)} onMouseEnter={() => setPin(i)} onFocus={() => setPin(i)} aria-label={p.label}>
                <i aria-hidden="true" />
                <span className={p.x > 58 ? "left" : ""}>{p.label}</span>
              </button>
            ))}
          </div>
        </div>
        <div className="wt-ctl">
          <button type="button" onClick={() => go(on - 1)} disabled={on === 0} aria-label="Previous step">←</button>
          <div className="wt-dots" role="tablist" aria-label="Steps">
            {shots.map((h, i) => (<button key={h.title} role="tab" aria-selected={i === on} aria-label={`Step ${i + 1}: ${h.title}`} className={i === on ? "on" : i < on ? "done" : ""} onClick={() => go(i)}>{i + 1}</button>))}
          </div>
          <button type="button" onClick={() => go(on + 1)} disabled={on === shots.length - 1} aria-label="Next step">→</button>
        </div>
        <p className="wt-hint">Hover or tap the dots on the screenshot.</p>
      </div>
    </div>
  );
}

/* ───────── 2. Tabs: the checks you don't see in the recording ───────── */

export type Tab = { name: string; title: string; text: string; lines: string[] };

export function Tabs({ tabs, label }: { tabs: Tab[]; label: string }) {
  const [i, setI] = useState(0);
  const t = tabs[i];
  return (
    <div className="tabs">
      <div className="tabs-list" role="tablist" aria-label={label}
        onKeyDown={(e) => { if (e.key === "ArrowRight") setI((i + 1) % tabs.length); if (e.key === "ArrowLeft") setI((i - 1 + tabs.length) % tabs.length); }}>
        {tabs.map((x, k) => (<button key={x.name} role="tab" id={`tab-${label}-${k}`} aria-selected={k === i} tabIndex={k === i ? 0 : -1} onClick={() => setI(k)}>{x.name}</button>))}
      </div>
      <div className="tabs-body" role="tabpanel" aria-labelledby={`tab-${label}-${i}`} key={i}>
        <h3>{t.title}</h3>
        <p>{t.text}</p>
        {t.lines.length > 0 && <pre><code>{t.lines.join("\n")}</code></pre>}
      </div>
    </div>
  );
}

/* ───────── 3. Whose fault is the failing test? Pick what happens ───────── */

export function Fork() {
  const [pick, setPick] = useState<"same" | "pass" | null>(null);
  return (
    <div className="fk">
      <ol className="fk-steps">
        <li><b>The test fails</b>after the model’s edits.</li>
        <li><b>Restore the original files</b>for a moment, as they were before the run.</li>
        <li><b>Run the same test again</b>on that clean copy. Then the model’s files go back.</li>
      </ol>
      <p className="fk-q">What happens on the clean copy? Pick one:</p>
      <div className="fk-pick" role="group" aria-label="Result on the original files">
        <button type="button" aria-pressed={pick === "same"} onClick={() => setPick("same")}>The same test fails</button>
        <button type="button" aria-pressed={pick === "pass"} onClick={() => setPick("pass")}>The test passes</button>
      </div>
      <div className="fk-out" aria-live="polite">
        {pick === "same" && (<div className="fk-card a"><b>Not the model’s fault.</b><p>It was already broken before zerd started. zerd leaves it alone, tells you so, and the run ends as <code>unverified</code>. The model isn’t sent to fix what it didn’t break.</p></div>)}
        {pick === "pass" && (<div className="fk-card b"><b>The model broke it.</b><p>The real error goes back to the model as a repair request, with a rule: don’t weaken the test to make it pass. It gets two attempts, then a stronger model takes over.</p></div>)}
        {pick === null && <div className="fk-card idle"><p>Choose a result to see what zerd does next.</p></div>}
      </div>
    </div>
  );
}

/* ───────── 4. Review stages: step through them ───────── */

export function Stages({ stages }: { stages: [string, string][] }) {
  const [i, setI] = useState(0);
  return (
    <div className="stg">
      <ol className="stg-nav" aria-label="Review stages">
        {stages.map(([t], k) => (
          <li key={t} className={k === i ? "on" : k < i ? "done" : ""}><button type="button" onClick={() => setI(k)} aria-current={k === i ? "step" : undefined}><span>{k + 1}</span>{t}</button></li>
        ))}
      </ol>
      <div className="stg-body" key={i}>
        <h3>{stages[i][0]}</h3>
        <p>{stages[i][1]}</p>
        <div className="stg-ctl">
          <button type="button" onClick={() => setI(Math.max(0, i - 1))} disabled={i === 0}>Back</button>
          <button type="button" onClick={() => setI(Math.min(stages.length - 1, i + 1))} disabled={i === stages.length - 1}>Next stage</button>
        </div>
      </div>
    </div>
  );
}

/* ───────── 5. Five endings: click one to see when it happens ───────── */

export function Endings({ ends }: { ends: { name: string; tone: string; what: string; when: string }[] }) {
  const [i, setI] = useState(0);
  const e = ends[i];
  return (
    <div className="en">
      <div className="en-list" role="tablist" aria-label="Ways a run can end" aria-orientation="vertical">
        {ends.map((x, k) => (<button key={x.name} role="tab" aria-selected={k === i} data-tone={x.tone} onClick={() => setI(k)}><i aria-hidden="true" />{x.name}</button>))}
      </div>
      <div className="en-body" role="tabpanel" key={i}>
        <code data-tone={e.tone}>{e.name}</code>
        <p className="what">{e.what}</p>
        <p className="when"><b>For example:</b> {e.when}</p>
      </div>
    </div>
  );
}

/* ───────── 6. Safety limits: what is allowed and what is refused ───────── */

export type Limit = { name: string; text: string; ok: string[]; no: string[] };

export function Limits({ limits }: { limits: Limit[] }) {
  const [i, setI] = useState(0);
  const l = limits[i];
  return (
    <div className="lim">
      <div className="tabs-list" role="tablist" aria-label="Safety limits">
        {limits.map((x, k) => (<button key={x.name} role="tab" aria-selected={k === i} onClick={() => setI(k)}>{x.name}</button>))}
      </div>
      <div className="lim-body" role="tabpanel" key={i}>
        <p>{l.text}</p>
        <div className="lim-cols">
          <div><h4><Check /> Allowed</h4><ul>{l.ok.map((x) => <li key={x}><code>{x}</code></li>)}</ul></div>
          <div><h4><Cross /> Refused</h4><ul>{l.no.map((x) => <li key={x}><code>{x}</code></li>)}</ul></div>
        </div>
      </div>
    </div>
  );
}

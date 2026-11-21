"use client";
import { useEffect, useRef, useState } from "react";
import { Brand } from "../brand";
import { Check, Cross } from "../icons";

/* ───────── scoreboard: cells fill in when it scrolls into view ───────── */

function Cells({ n, of, tone, delay = 0 }: { n: number; of: number; tone: "z" | "c"; delay?: number }) {
  return (
    <span className="cells" aria-hidden="true">
      {Array.from({ length: of }, (_, i) => (<i key={i} className={i < n ? "f" : ""} style={{ transitionDelay: `${delay + i * 55}ms` }} data-tone={tone} />))}
    </span>
  );
}

export function Score() {
  const ref = useRef<HTMLDivElement>(null);
  const [on, setOn] = useState(false);
  useEffect(() => {
    const el = ref.current; if (!el) return;
    const io = new IntersectionObserver(([e]) => { if (e.isIntersecting) { setOn(true); io.disconnect(); } }, { threshold: 0.3 });
    io.observe(el); return () => io.disconnect();
  }, []);
  return (
    <div className={`score ${on ? "on" : ""}`} ref={ref}>
      <article className="sc z">
        <header><Brand name="minimax" size={26} /><div><b>zerd</b><small>MiniMax M3</small></div></header>
        <div className="sc-row"><div className="sc-n">12</div><div><p>real bugs found</p><Cells n={12} of={12} tone="z" /></div></div>
        <div className="sc-row"><div className="sc-n">15<small>/15</small></div><div><p>scenarios with an automated test</p><Cells n={15} of={15} tone="z" delay={300} /></div></div>
        <ul className="sc-tags"><li>Real PostgreSQL</li><li>Real Redis</li><li>Really killed processes</li></ul>
      </article>
      <article className="sc c">
        <header><Brand name="claudecode" size={26} /><div><b>Claude Code</b><small>Sonnet 5.5</small></div></header>
        <div className="sc-row"><div className="sc-n">3</div><div><p>real bugs found</p><Cells n={3} of={12} tone="c" /></div></div>
        <div className="sc-row"><div className="sc-n">14<small>/15</small></div><div><p>scenarios with an automated test</p><Cells n={14} of={15} tone="c" delay={300} /></div></div>
        <ul className="sc-tags"><li>In-memory fake database</li><li>Simulated failures</li></ul>
      </article>
    </div>
  );
}

/* ───────── compare by area ───────── */

export type Area = { name: string; z: string; c: string; why: string };

export function Compare({ areas }: { areas: Area[] }) {
  const [i, setI] = useState(0);
  const a = areas[i];
  return (
    <div className="cmpx">
      <div className="tabs-list" role="tablist" aria-label="Area tested"
        onKeyDown={(e) => { if (e.key === "ArrowRight") setI((i + 1) % areas.length); if (e.key === "ArrowLeft") setI((i - 1 + areas.length) % areas.length); }}>
        {areas.map((x, k) => (<button key={x.name} role="tab" aria-selected={k === i} tabIndex={k === i ? 0 : -1} onClick={() => setI(k)}>{x.name}</button>))}
      </div>
      <div className="cmpx-body" role="tabpanel" key={i}>
        <div className="vs2">
          <div className="side z"><h4><Brand name="minimax" size={20} /> zerd <em><Check /> Real</em></h4><p>{a.z}</p></div>
          <div className="vs-mid" aria-hidden="true">vs</div>
          <div className="side c"><h4><Brand name="claudecode" size={20} /> Claude Code <em><Cross /> Pretend</em></h4><p>{a.c}</p></div>
        </div>
        <p className="why"><b>Why it matters.</b> {a.why}</p>
      </div>
    </div>
  );
}

/* ───────── what zerd found: open one ───────── */

export function Finds({ items }: { items: { bug: string; fix: string }[] }) {
  const [open, setOpen] = useState(0);
  return (
    <ol className="fnd">
      {items.map((x, i) => (
        <li key={x.bug} className={open === i ? "open" : ""}>
          <button type="button" aria-expanded={open === i} onClick={() => setOpen(open === i ? -1 : i)}>
            <span className="n">{i + 1}</span><span className="t">{x.bug}</span><span className="pm" aria-hidden="true" />
          </button>
          <div className="fnd-body"><div><p><b>What was wrong and how zerd fixed it.</b> {x.fix}</p></div></div>
        </li>
      ))}
    </ol>
  );
}

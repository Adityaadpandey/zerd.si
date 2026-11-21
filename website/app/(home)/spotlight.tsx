"use client";
import { useEffect } from "react";

// A soft blue light follows the pointer inside the nav bar, the install command and every card.
// It adds one small layer to the element under the pointer, behind its content, and moves it with the cursor.
const TARGETS = [
  ".nav-pill", ".cmd", ".modal-cmd", ".modal-card", ".close-win", ".wt-win", ".lp .win",
  ".sc", ".vs2 .side", ".tabs-body", ".lim-body", ".lim-cols > div", ".stg-body", ".stg-nav button", ".en-body",
  ".fk-card", ".fk-steps li", ".chips li", ".why", ".honest", ".snip", ".teaser-link", ".faq-list details",
].join(",");

export default function Spotlight() {
  useEffect(() => {
    if (matchMedia("(prefers-reduced-motion: reduce)").matches || matchMedia("(hover: none)").matches) return;
    let cur: HTMLElement | null = null;
    const layerOf = (el: HTMLElement) => {
      let l = el.querySelector<HTMLElement>(":scope > .glow-layer");
      if (!l) {
        l = document.createElement("i");
        l.className = "glow-layer";
        l.setAttribute("aria-hidden", "true");
        el.prepend(l);
        if (getComputedStyle(el).position === "static") el.style.position = "relative";
        el.style.isolation = "isolate";
      }
      return l;
    };
    const off = () => { cur?.querySelector<HTMLElement>(":scope > .glow-layer")?.style.setProperty("opacity", "0"); cur = null; };
    const move = (e: PointerEvent) => {
      const el = (e.target as Element | null)?.closest<HTMLElement>(TARGETS) ?? null;
      if (el !== cur) { off(); cur = el; }
      if (!el) return;
      const r = el.getBoundingClientRect(), l = layerOf(el);
      l.style.setProperty("--gx", `${e.clientX - r.left}px`);
      l.style.setProperty("--gy", `${e.clientY - r.top}px`);
      l.style.opacity = "1";
    };
    document.addEventListener("pointermove", move, { passive: true });
    document.addEventListener("pointerleave", off);
    return () => { document.removeEventListener("pointermove", move); document.removeEventListener("pointerleave", off); };
  }, []);
  return null;
}

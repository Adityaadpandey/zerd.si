"use client";
import Link from "next/link";
import { useEffect, useRef, useState } from "react";
import { Arrow } from "./icons";

// The nav's Install button. It opens a dialog with the command for the visitor's own OS, so they can copy it
// and go. The other OS is one click away.
type Os = "unix" | "windows";
const CMD: Record<Os, string> = {
  unix: "curl -fsSL https://www.zerd.si/install.sh | sh",
  windows: "irm https://www.zerd.si/install.ps1 | iex",
};
const NAME = (os: Os, mac: boolean) => (os === "windows" ? "Windows" : mac ? "macOS" : "Linux");

function detect(): { os: Os; mac: boolean } {
  const ua = navigator.userAgent || "";
  const plat = (navigator as Navigator & { userAgentData?: { platform?: string } }).userAgentData?.platform || navigator.platform || "";
  const s = `${plat} ${ua}`;
  if (/win/i.test(s) && !/darwin/i.test(s)) return { os: "windows", mac: false };
  if (/mac|iphone|ipad/i.test(s)) return { os: "unix", mac: true };
  return { os: "unix", mac: false };
}

export default function InstallButton() {
  const [open, setOpen] = useState(false);
  const [os, setOs] = useState<Os>("unix");
  const [mac, setMac] = useState(false);
  const [done, setDone] = useState(false);
  const opener = useRef<HTMLButtonElement>(null);
  const first = useRef<HTMLButtonElement>(null);

  useEffect(() => { const d = detect(); setOs(d.os); setMac(d.mac); }, []);
  useEffect(() => {
    if (!open) return;
    const prev = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    first.current?.focus();
    const key = (e: KeyboardEvent) => { if (e.key === "Escape") setOpen(false); };
    document.addEventListener("keydown", key);
    return () => { document.body.style.overflow = prev; document.removeEventListener("keydown", key); opener.current?.focus(); };
  }, [open]);

  const copy = async () => {
    try { await navigator.clipboard.writeText(CMD[os]); setDone(true); setTimeout(() => setDone(false), 2200); } catch { /* the text stays selectable */ }
  };

  return (
    <>
      <button ref={opener} type="button" className="nav-install" onClick={() => setOpen(true)} aria-haspopup="dialog">Install <Arrow /></button>
      {open && (
        <div className="modal" role="presentation" onMouseDown={(e) => { if (e.target === e.currentTarget) setOpen(false); }}>
          <div className="modal-card" role="dialog" aria-modal="true" aria-labelledby="inst-h">
            <button type="button" className="modal-x" aria-label="Close" onClick={() => setOpen(false)}>×</button>
            <h2 id="inst-h">Install zerd</h2>
            <p className="modal-sub">One file, about two minutes. Paste this in your terminal{os === "windows" ? " (PowerShell)" : ""}.</p>
            <div className="modal-tabs" role="tablist" aria-label="Operating system">
              <button ref={first} role="tab" aria-selected={os === "unix"} onClick={() => setOs("unix")}>{mac || os === "windows" ? (mac ? "macOS" : "macOS and Linux") : "Linux"}{os === "unix" && mac ? "" : ""}</button>
              <button role="tab" aria-selected={os === "windows"} onClick={() => setOs("windows")}>Windows</button>
            </div>
            <div className="modal-cmd">
              <code><span aria-hidden="true">$ </span>{CMD[os]}</code>
              <button type="button" onClick={copy}>{done ? "Copied" : "Copy"}</button>
              <span className="sr-only" role="status">{done ? "Install command copied" : ""}</span>
            </div>
            <p className="modal-note">{os === "windows" ? "Detected: Windows 10 or 11. Needs Git for Windows." : `Detected: ${NAME("unix", mac)}.`} Then run <code>zerd</code> in any project folder.</p>
            <p className="modal-links"><Link href="/docs/quickstart" onClick={() => setOpen(false)}>Read the quickstart</Link><Link href="/docs/windows" onClick={() => setOpen(false)}>Windows notes</Link></p>
          </div>
        </div>
      )}
    </>
  );
}

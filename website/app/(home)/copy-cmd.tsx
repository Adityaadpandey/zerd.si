"use client";
import { useEffect, useState } from "react";

// The install command with a copy button. When both commands are given it shows the one for the visitor's
// OS and says underneath that the other systems work too. Selecting the text by hand works as well.
export default function CopyCmd({ cmd, winCmd }: { cmd: string; winCmd?: string }) {
  const [done, setDone] = useState(false);
  const [win, setWin] = useState(false);
  useEffect(() => {
    const plat = (navigator as Navigator & { userAgentData?: { platform?: string } }).userAgentData?.platform || navigator.platform || "";
    setWin(!!winCmd && /win/i.test(`${plat} ${navigator.userAgent}`) && !/darwin/i.test(plat));
  }, [winCmd]);
  const shown = win && winCmd ? winCmd : cmd;
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(shown);
      setDone(true);
      setTimeout(() => setDone(false), 2200);
    } catch {
      // clipboard blocked; the text is still selectable
    }
  };
  return (
    <>
      <div className="cmd">
        <code><span aria-hidden="true">{win ? "PS> " : "$ "}</span>{shown}</code>
        <button type="button" onClick={copy}>{done ? "Copied" : "Copy"}</button>
        <span className="sr-only" role="status">{done ? "Install command copied" : ""}</span>
      </div>
      {winCmd && <p className="cmd-note">Works on macOS, Linux and Windows.</p>}
    </>
  );
}

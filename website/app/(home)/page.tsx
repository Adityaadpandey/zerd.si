import type { Metadata } from "next";
import Link from "next/link";
import { latestRelease } from "@/lib/latest";
import { JsonLd } from "@/lib/jsonld";
import CopyCmd from "./copy-cmd";
import Demo from "./demo";
import Scene from "./scene";
import { BrandIcon, Check, Cross } from "./icons";
import Models from "./models";
import { FAQ } from "@/lib/faq";
import { DESCRIPTION, HOME_TITLE, NAME, SITE } from "@/lib/site";
import { AUTHOR } from "@/lib/author";

// The homepage states its own canonical and robots. It used to inherit them from the root layout, but a
// global `canonical: "/"` also landed on the 404 page, telling Google the 404 was another copy of the
// homepage. A global `index, follow` did the same thing to the 404's robots tags, which is how a 404
// ended up looking indexable.
export const metadata: Metadata = {
  title: HOME_TITLE,
  description: DESCRIPTION,
  alternates: { canonical: "/" },
  robots: { index: true, follow: true, googleBot: { "max-snippet": -1, "max-image-preview": "large" } },
  openGraph: { type: "website", siteName: NAME, locale: "en_US", url: SITE, title: HOME_TITLE, description: DESCRIPTION },
  twitter: { card: "summary_large_image", title: HOME_TITLE, description: DESCRIPTION },
};

const INSTALL = "curl -fsSL https://www.zerd.si/install.sh | sh";

const VERSUS: [string, string, string][] = [
  ["Real PostgreSQL", "Yes", "No"],
  ["Real Redis", "Yes", "No"],
  ["Worker crash", "kill -9", "Simulated"],
  ["Concurrent claims", "25 real races", "In-memory"],
  ["Outages", "Exercised", "Simulated"],
  ["Scenarios", "15 of 15 run", "15 of 15 attempted"],
];

const PIPE: [string, string, string?][] = [
  ["It edits", "zerd makes the change in your files."],
  ["It tests", "Tests and lint run on exactly what changed.", "npx tsc --noEmit   ✓ Ran"],
  ["A second AI reviews", "A different model reads the real diff, not a summary."],
  ["A third pass objects", "A verifier tries to prove each finding wrong, so you only see the ones that hold."],
  ["It checks its own claims", "Every file and line it mentions is looked up on disk. No invented paths."],
  ["You get the receipt", "What ran, what passed, what is still unverified.", "verified  1 files · 2 checks ok · /undo reverts"],
];

export default async function Home() {
  const rel = await latestRelease();
  return (
    <main>
      <JsonLd
        data={{
          "@context": "https://schema.org",
          "@type": "FAQPage",
          "@id": `${SITE}/#faq`,
          url: SITE,
          isPartOf: { "@id": `${SITE}/#site` },
          mainEntity: FAQ.map(([q, a]) => ({
            "@type": "Question",
            name: q,
            acceptedAnswer: { "@type": "Answer", text: a },
            // Marks the question as one a voice assistant should read out, for the answers that are a
            // single spoken sentence.
            speakable: { "@type": "SpeakableSpecification", cssSelector: ["details summary"] },
          })),
        }}
      />

      <Scene install={INSTALL} steps={PIPE} />

      <div className="after">
      <section className="lift wrap" aria-label="A real recording of zerd">
        <Demo />
      </section>

      <section className="sec wrap" id="proof">
        <div className="sec-head">
          <h2>It finds what mock tests can’t</h2>
          <p>Same payments project, 15 attack scenarios. zerd ran them on real Postgres and Redis and found 12 real bugs. Claude Code found real bugs too, but never touched real infrastructure.</p>
        </div>
        <div className="vs glow" role="table" aria-label="zerd and Claude Code compared">
          <div className="vs-h" role="row">
            <span role="columnheader"></span>
            <span role="columnheader" className="who z"><i className="logo-tick" aria-hidden />zerd <small>MiniMax M3</small></span>
            <span role="columnheader" className="who"><BrandIcon name="claude" size={20} />Claude Code <small>Sonnet 5.5</small></span>
          </div>
          {VERSUS.map(([k, z, c]) => (
            <div className="vs-r" role="row" key={k}>
              <span role="rowheader">{k}</span>
              <span role="cell" className="yes">{z === "Yes" && <Check />}{z}</span>
              <span role="cell" className="no">{c === "No" && <Cross />}{c}</span>
            </div>
          ))}
        </div>
        <p className="caveat">One project, one run each. This shows how deeply each tool verifies, not which model is better.</p>
      </section>

      <section className="sec wrap install" id="install">
        <div className="sec-head">
          <h2>Try it on one small change</h2>
          <p>One binary, no setup, two minutes. If it doesn’t save you review time, /undo puts everything back.</p>
        </div>
        <CopyCmd cmd={INSTALL} />
        <Models />
        <p className="hero-fine">Windows: <code>irm https://www.zerd.si/install.ps1 | iex</code></p>
        <p className="hero-links">
          <Link href="/docs/quickstart">Get started</Link>
          <Link href="/docs/verification">Why zerd</Link>
          {rel && <span>v{rel.version}</span>}
        </p>
        <p className="hero-fine">Built by <a href={AUTHOR.site} rel="author">Aditya Dutt Pandey</a> · <Link href="/docs/about">About zerd</Link></p>
      </section>
      <section className="sec wrap faq" id="faq">
        <div className="sec-head">
          <h2>Common questions about zerd</h2>
        </div>
        <div className="faq-list">
          {FAQ.map(([q, a]) => (
            <details key={q}>
              <summary>{q}</summary>
              <p>{a}</p>
            </details>
          ))}
        </div>
      </section>
      </div>
    </main>
  );
}

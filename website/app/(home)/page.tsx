import type { Metadata } from "next";
import Link from "next/link";
import { latestRelease } from "@/lib/latest";
import { JsonLd } from "@/lib/jsonld";
import CopyCmd from "./copy-cmd";
import Demo from "./demo";
import Receipt from "./receipt";
import { FAQ } from "@/lib/faq";
import { TASKS, WEBHOOK } from "@/lib/results";
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

const PIPE: [string, string][] = [
  ["It edits", "zerd makes the change in your files."],
  ["It tests", "Tests and lint run on exactly what changed, never the whole build."],
  ["A second AI reviews", "A different model reads the real diff, not a summary."],
  ["A third pass objects", "A verifier tries to prove each finding wrong, so you only see the ones that hold."],
  ["It checks its own claims", "Every file and line it mentions is looked up on disk. No invented paths."],
  ["You get the receipt", "What ran, what passed, what is still unverified."],
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

      <section className="hero">
        {/* The hero art is a still. It used to be a looping 7.6 MB video of the same artwork, which
            pushed the first real content below the fold and cost megabytes on the critical path for
            a background nobody reads twice. */}
        <div className="hero-img" aria-hidden="true">
          <picture>
            <source srcSet="/hero/hero.avif" type="image/avif" />
            <img src="/hero/hero.webp" alt="" draggable={false} width={2560} height={1441} fetchPriority="high" decoding="async" />
          </picture>
        </div>
        <div className="hero-copy">
          <p className="eyebrow">Terminal AI coding agent</p>
          <h1>Your agent says “done.” zerd shows the proof.</h1>
          <p className="lead">It runs tests on the files it changed, has a second model read the real diff, and tells you what it couldn’t verify. You read a short receipt, not the whole change.</p>
          <CopyCmd cmd={INSTALL} />
          <p className="hero-links">
            <Link href="/docs/quickstart">Try it in two minutes</Link>
            <a href="#receipt">See the receipt</a>
          </p>
          <p className="hero-fine">Free to run · macOS, Linux, Windows · Claude, GPT, DeepSeek or local models</p>
          <p className="hero-fine">
            Built by <a href={AUTHOR.site} rel="author">Aditya Dutt Pandey</a> · <Link href="/docs/about">About zerd</Link>
          </p>
        </div>
      </section>

      <div className="after">
      {/* The receipt is the product. It goes above the recording, because a visitor should picture
          what they get back before they sit through a video of a terminal. */}
      <section className="sec wrap receipt-sec" id="receipt">
        <div className="receipt-grid">
          <div className="sec-head">
            <p className="eyebrow">The receipt</p>
            <h2>Every run ends with evidence, not a claim</h2>
            <p>Six steps run on their own after every edit. None of them wait for you. What comes back is a receipt: what ran, what passed, and what nobody could check.</p>
            <ol className="pipe">
              {PIPE.map(([t, d]) => (
                <li key={t}>
                  <h3>{t}</h3>
                  <p>{d}</p>
                </li>
              ))}
            </ol>
          </div>
          <Receipt />
        </div>
      </section>

      <section className="lift wrap" aria-label="A real recording of zerd">
        <Demo />
      </section>

      <section className="sec wrap" id="proof">
        <div className="sec-head">
          <p className="eyebrow">Measured</p>
          <h2>It finds what mock tests can’t</h2>
          <p>Same payments project, 15 attack scenarios. zerd ran them on real Postgres and Redis and found 12 real bugs. Claude Code found real bugs too, but never touched real infrastructure.</p>
        </div>
        <div className="vs" role="table" aria-label="zerd and Claude Code compared on the webhook reliability test">
          <div className="vs-h" role="row">
            <span role="columnheader">{WEBHOOK.scenarios}</span>
            <span role="columnheader">zerd, MiniMax M3</span>
            <span role="columnheader">{WEBHOOK.vs}</span>
          </div>
          {WEBHOOK.rows.map((r) => (
            <div className="vs-r" role="row" key={r.label}>
              <span role="rowheader">{r.label}</span>
              <span role="cell" className="yes">{r.zerd}</span>
              <span role="cell">{r.other}</span>
            </div>
          ))}
        </div>
        <p className="caveat">{WEBHOOK.caveat}</p>

        <div className="sec-head sub">
          <h3>And on plain coding tasks, it matched</h3>
          <p>{TASKS.label}</p>
        </div>
        <div className="vs" role="table" aria-label="zerd and Claude Code compared on 11 coding tasks">
          <div className="vs-h" role="row">
            <span role="columnheader">11 tasks</span>
            <span role="columnheader">zerd, GPT-5.6 Luna</span>
            <span role="columnheader">{TASKS.vs}</span>
          </div>
          {TASKS.rows.map((r) => (
            <div className="vs-r" role="row" key={r.label}>
              <span role="rowheader">{r.label}</span>
              <span role="cell" className="yes">{r.zerd}</span>
              <span role="cell">{r.other}</span>
            </div>
          ))}
        </div>
        <p className="caveat">{TASKS.caveat}</p>
      </section>

      <section className="sec wrap install" id="install">
        <div className="sec-head">
          <h2>Try it on one small change</h2>
          <p>One binary, no setup, two minutes. If it doesn’t save you review time, /undo puts everything back.</p>
        </div>
        <CopyCmd cmd={INSTALL} />
        <p className="hero-fine">Windows: <code>irm https://www.zerd.si/install.ps1 | iex</code></p>
        <p className="hero-links">
          <Link href="/docs/quickstart">Get started</Link>
          <Link href="/docs/verification">Why zerd</Link>
          {rel && <span>v{rel.version}</span>}
        </p>
      </section>
      <section className="sec wrap faq" id="faq">
        <div className="sec-head">
          <p className="eyebrow">Questions</p>
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

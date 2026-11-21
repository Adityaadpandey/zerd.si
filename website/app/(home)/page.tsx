import type { Metadata } from "next";
import Link from "next/link";
import { latestRelease } from "@/lib/latest";
import { JsonLd } from "@/lib/jsonld";
import CopyCmd from "./copy-cmd";
import Demo from "./demo";
import Scene from "./scene";
import Models from "./models";
import { Arrow } from "./icons";
import { FAQ } from "@/lib/faq";
import { TASKS, WEBHOOK } from "@/lib/results";
import { DESCRIPTION, HOME_TITLE, NAME, SITE } from "@/lib/site";

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

const WIN = "irm https://www.zerd.si/install.ps1 | iex";
const INSTALL = "curl -fsSL https://www.zerd.si/install.sh | sh";

const PIPE: [string, string, string?][] = [
  ["It makes the change", "You say what you want in plain words. zerd edits your files."],
  ["It runs your tests", "Only the tests and checks for the files it just changed.", "npx tsc --noEmit   ✓ passed"],
  ["A second AI reviews it", "A different AI reads the actual changes, like a teammate doing a code review."],
  ["It argues with itself", "Another pass tries to prove each problem wrong, so you only see the ones that are real."],
  ["It checks it isn’t making things up", "Every file and line number it mentions is looked up in your project. No invented files."],
  ["You get a short report", "What ran, what passed, and what it couldn’t check. Don’t like the result? One command undoes it.", "verified · 1 file changed · 2 checks passed · /undo puts it back"],
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

      <Scene install={INSTALL} steps={PIPE} version={rel?.version} />

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

      <section className="sec wrap teaser" id="proof" aria-labelledby="proof-h">
        <h2 id="proof-h">We tested it against Claude Code.</h2>
        <p>Same payments project, same 15 things that could go wrong. zerd ran them on a real database and found 12 real bugs.</p>
        <Link className="teaser-link" href="/proof">See the proof <Arrow /></Link>
      </section>

      <section className="sec wrap install" id="install">
        <div className="sec-head">
          <h2>Try it on one small change</h2>
          <p>One file to install, about two minutes. If you don’t like what it did, type /undo and everything goes back.</p>
        </div>
        <CopyCmd cmd={INSTALL} winCmd={WIN} />
        <Models />
        <p className="hero-links">
          <Link href="/docs/quickstart">Get started</Link>
          <Link href="/how-it-works">How it works</Link>
          {rel && <span>v{rel.version}</span>}
        </p>
      </section>
      <section className="sec wrap faq" id="faq">
        <div className="sec-head">
          <h2>Questions</h2>
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

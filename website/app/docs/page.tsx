import type { Metadata } from "next";
import Link from "next/link";
import { DocsBody, DocsDescription, DocsPage, DocsTitle } from "fumadocs-ui/layouts/docs/page";
import { source } from "@/lib/source";
import { pageMeta } from "@/lib/seo";
import { JsonLd } from "@/lib/jsonld";
import { NAME, SITE } from "@/lib/site";

export const metadata: Metadata = pageMeta({
  path: "/docs",
  image: "/opengraph-image",
  title: "zerd docs: install, models, safety, CLI reference",
  description: "Learn zerd, the terminal AI coding agent: install in two minutes, see how every run is verified, and read the full CLI, config and tools reference.",
  keywords: ["zerd docs", "AI coding agent documentation", "coding agent CLI reference"],
});

// Groups come from the sidebar tree (the separators in meta.json), so this index never drifts from it.
// One line per sidebar group, saying who it is for. Keyed by the separator names in meta.json.
const BLURBS: Record<string, string> = {
  "Start here": "New here? Read these three in order.",
  Guides: "Everyday use: what to type, which model to pick, and how zerd keeps your code safe.",
  "Under the hood": "How zerd decides a change is actually finished.",
  Reference: "Every flag, setting and command, for when you know what you’re after.",
  Evidence: "Head-to-head results, honest limits, and the license.",
  About: "Who writes zerd, how it is built, and where the source lives.",
  Compare: "zerd next to Claude Code, Aider, Codex CLI and OpenCode, and which to pick.",
};

function groups() {
  const out: { title: string; pages: { url: string; name: string; description?: string }[] }[] = [];
  for (const node of source.getPageTree().children) {
    if (node.type === "separator") out.push({ title: String(node.name), pages: [] });
    else if (node.type === "page" && out.length)
      out.at(-1)!.pages.push({
        url: node.url,
        name: String(node.name),
        description: source.getPages().find((p) => p.url === node.url)?.data.description,
      });
  }
  return out;
}

export default function DocsIndex() {
  return (
    <DocsPage toc={[]}>
      {/* The index is a collection of every docs page. Saying so in schema lets an answer engine pick
          this page when the question is "what does zerd document" rather than one specific page. */}
      <JsonLd
        data={{
          "@context": "https://schema.org",
          "@graph": [
            {
              "@type": "CollectionPage", "@id": `${SITE}/docs#collection`, url: `${SITE}/docs`,
              name: "zerd docs", description: "Install, models, safety, CLI and tools reference for zerd, the terminal AI coding agent.",
              isPartOf: { "@id": `${SITE}/#site` }, about: { "@id": `${SITE}/#app` },
            },
            {
              "@type": "ItemList", "@id": `${SITE}/docs#list`,
              numberOfItems: source.getPages().length,
              itemListElement: source.getPages().map((p, i) => ({
                "@type": "ListItem", position: i + 1, name: p.data.title, url: `${SITE}${p.url}`,
              })),
            },
          ],
        }}
      />
      <DocsTitle>Learn zerd</DocsTitle>
      <DocsDescription>zerd is a terminal coding agent that checks its own work. Install it in two minutes, see how a run is verified, and read where it still falls short.</DocsDescription>
      <DocsBody>
        {groups().map((g, i) => (
          <section key={g.title} className="docs-group">
            <h2>{g.title}</h2>
            {BLURBS[g.title] && <p className="docs-blurb">{BLURBS[g.title]}</p>}
            <div className={`docs-cards${i === 0 ? " docs-featured" : ""}`}>
              {g.pages.map((p) => (
                <Link key={p.url} href={p.url} className="docs-card">
                  <strong>{p.name}</strong>
                  {p.description && <span>{p.description}</span>}
                </Link>
              ))}
            </div>
          </section>
        ))}
      </DocsBody>
    </DocsPage>
  );
}

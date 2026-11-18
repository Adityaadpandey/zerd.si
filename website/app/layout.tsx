import type { Metadata } from "next";
import { Bricolage_Grotesque, Instrument_Serif, JetBrains_Mono } from "next/font/google";
import { Provider } from "@/components/provider";
import { JsonLd } from "@/lib/jsonld";
import { DESCRIPTION, HOME_TITLE, NAME, SITE } from "@/lib/site";
import { REPO } from "@/lib/docs";
import { AUTHOR } from "@/lib/author";
import "./globals.css";

const sans = Bricolage_Grotesque({ subsets: ["latin"], variable: "--font-sans" });
const serif = Instrument_Serif({ subsets: ["latin"], weight: "400", style: ["italic", "normal"], variable: "--font-serif" });
const mono = JetBrains_Mono({ subsets: ["latin"], variable: "--font-mono" });

export const metadata: Metadata = {
  metadataBase: new URL(SITE),
  title: { default: HOME_TITLE, template: `%s · ${NAME}` },
  description: DESCRIPTION,
  applicationName: NAME,
  keywords: ["terminal coding agent", "AI coding agent", "AI coding assistant CLI", "Claude Code alternative", "Aider alternative", "Codex CLI alternative", "AI code review", "verify AI generated code", "agentic coding", "Rust CLI"],
  authors: [{ name: AUTHOR.name, url: AUTHOR.site }],
  creator: NAME,
  category: "technology",
  openGraph: { type: "website", siteName: NAME, locale: "en_US", url: SITE, title: HOME_TITLE, description: DESCRIPTION },
  twitter: { card: "summary_large_image", title: HOME_TITLE, description: DESCRIPTION },
  // No global `robots` here on purpose. Next.js puts `noindex` on the 404 page, and a second global
  // tag saying `index, follow` left the 404 with two conflicting robots directives, which Google reads
  // as an indexable page. Each page states its own indexing instead, and the 404 keeps only `noindex`.
};

// Tells search and answer engines what zerd is and who makes it. The nodes point at each other by @id,
// so the app, the site and the publisher read as one connected entity instead of three loose blobs.
const schema = {
  "@context": "https://schema.org",
  "@graph": [
    {
      "@type": "Organization", "@id": `${SITE}/#org`, name: NAME, url: SITE, logo: `${SITE}/icon.svg`,
      sameAs: [`https://github.com/${REPO}`],
      founder: { "@id": `${SITE}/#author` },
    },
    {
      // A person, not the product name. An answer engine asked "who says this" needs a name it can
      // attribute a claim to, and "zerd" is a tool, not a source.
      "@type": "Person", "@id": `${SITE}/#author`, name: AUTHOR.name, url: AUTHOR.site, sameAs: [AUTHOR.site, AUTHOR.github],
      jobTitle: AUTHOR.jobTitle, knowsAbout: [...AUTHOR.knowsAbout], worksFor: { "@id": `${SITE}/#org` },
    },
    { "@type": "WebSite", "@id": `${SITE}/#site`, url: SITE, name: NAME, description: DESCRIPTION, inLanguage: "en", publisher: { "@id": `${SITE}/#org` } },
    {
      "@type": "SoftwareApplication", "@id": `${SITE}/#app`, name: NAME, alternateName: "zerd coding agent", url: SITE, description: DESCRIPTION,
      applicationCategory: "DeveloperApplication", applicationSubCategory: "AI coding agent", operatingSystem: "macOS, Linux, Windows",
      offers: { "@type": "Offer", price: "0", priceCurrency: "USD" }, isAccessibleForFree: true,
      author: { "@id": `${SITE}/#org` }, publisher: { "@id": `${SITE}/#org` },
      sameAs: [`https://github.com/${REPO}`],
      codeRepository: `https://github.com/${REPO}`, downloadUrl: `${SITE}/docs/quickstart`, installUrl: `${SITE}/docs/quickstart`,
      softwareHelp: { "@type": "WebPage", url: `${SITE}/docs` }, license: `${SITE}/docs/license`,
      featureList: [
        "Runs tests and lint on exactly the files it changed",
        "A second model reviews the real diff",
        "A verifier tries to disprove each review finding",
        "Looks up every file path and line it mentions on disk",
        "Works with Claude, GPT, Codex, DeepSeek and local models, with failover",
        "OS sandbox and confined file tools",
      ],
    },
  ],
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" className={`${sans.variable} ${serif.variable} ${mono.variable}`} suppressHydrationWarning>
      <body>
        <JsonLd data={schema} />
        <Provider>{children}</Provider>
      </body>
    </html>
  );
}

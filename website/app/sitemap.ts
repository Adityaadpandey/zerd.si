import type { MetadataRoute } from "next";
import { source } from "@/lib/source";
import { SITE } from "@/lib/site";
import { lastModified, lastModifiedAt } from "@/lib/lastmod";

export const dynamic = "force-static";

// Google ignores priority and changefreq, so they are left out. lastModified is the real git date of each
// page's source file, and left out when git can't tell us, rather than claiming everything changed today.
//
// Every docs page is listed, plus the plain-markdown twin of each one. The .md twin sends a
// `Link: rel="canonical"` header pointing at the HTML page, so the two are one page, not two: listing
// both tells Google the .md URL exists instead of leaving it to be discovered by crawling, which is how
// a "page with redirect" report starts.
export default function sitemap(): MetadataRoute.Sitemap {
  const pages = source.getPages();
  const docs = pages.map((p) => ({
    url: `${SITE}${p.url}`,
    lastModified: lastModified(`content/docs/${p.path}`),
    at: lastModifiedAt(`content/docs/${p.path}`),
  }));
  // The docs index changes whenever any docs page does, so it carries the newest of them.
  // The dates come from different commits in different timezones, so they are compared as instants.
  const newest = docs.reduce<number | undefined>((max, d) => (d.at === undefined ? max : max === undefined ? d.at : Math.max(max, d.at)), undefined);
  const markdown = pages.map((p) => ({ url: `${SITE}${p.url}.md`, lastModified: lastModified(`content/docs/${p.path}`) }));
  return [
    { url: SITE, lastModified: lastModified("app/(home)/page.tsx") },
    { url: `${SITE}/how-it-works`, lastModified: lastModified("app/(home)/how-it-works/page.tsx") },
    { url: `${SITE}/proof`, lastModified: lastModified("app/(home)/proof/page.tsx") },
    { url: `${SITE}/docs`, lastModified: newest === undefined ? undefined : new Date(newest).toISOString() },
    ...docs.map(({ url, lastModified: lm }) => ({ url, lastModified: lm })),
    ...markdown,
  ];
}

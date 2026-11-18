import { source } from "@/lib/source";
import { FAQ } from "@/lib/faq";
import { DESCRIPTION, NAME, SITE, TAGLINE } from "@/lib/site";
import { AUTHOR } from "@/lib/author";

export const dynamic = "force-static";

// Short index for AI agents (llmstxt.org): what zerd is, the common questions, then every docs page.
//
// Each page is listed twice, once as the HTML page and once as its plain-markdown twin. The .md URL
// sends a `Link: rel="canonical"` header pointing at the HTML page, so it is the same content in a
// format an agent can read without parsing, not a second copy to worry about.
export function GET() {
  const pages = source.getPages().map((p) => `- [${p.data.title}](${SITE}${p.url}): ${p.data.description ?? ""}`);
  const markdown = source.getPages().map((p) => `- [${p.data.title} (markdown)](${SITE}${p.url}.md)`);
  const faq = FAQ.map(([q, a]) => `**${q}** ${a}`).join("\n\n");
  const body = `# ${NAME}, ${TAGLINE}\n\n> ${DESCRIPTION}\n\nHome page: ${SITE}\nWritten by ${AUTHOR.name} (${AUTHOR.site}, ${AUTHOR.github})\nInstall (macOS/Linux): \`curl -fsSL ${SITE}/install.sh | sh\`\nInstall (Windows): \`irm ${SITE}/install.ps1 | iex\`\nFull docs in one file: ${SITE}/llms-full.txt\n\n## Common questions\n\n${faq}\n\n## Docs\n\n${pages.join("\n")}\n\n### Plain markdown\n\nThe same pages, ready to read without parsing HTML.\n\n${markdown.join("\n")}\n`;
  return new Response(body, { headers: { "content-type": "text/plain; charset=utf-8" } });
}

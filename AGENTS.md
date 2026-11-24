## Learned by zerd
- Explain the exact remediation steps for the Vercel redirect and the 404 metadata conflict before making changes.
- Delete the `www.zerd.si` → `zerd.si` domain redirect, since Vercel’s built-in apex-to-www redirect already sends `zerd.si` → `www.zerd.si`.
- Report the site's canonical host and redirect chain (zerd.si → www.zerd.si via 308) when answering SEO questions, and prefer www.zerd.si as the canonical domain.
- Include every canonical documentation page in the sitemap and improve the sitemap/SEO setup comprehensively.
- Complete the full SEO, AEO, and GEO optimization, not only the sitemap changes.
- Commit the changes in separate, logical parts.
- Ensure the terminal AI coding agent section has sufficient top margin.
- In a `/loop` round, read the `.zerd/loop/` file, do the next part without changing unrelated code, record what was done and what is left under `# Progress`, and set `STATUS: DONE` only after running every "Done when" command.

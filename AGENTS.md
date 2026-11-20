## Learned by zerd
- Explain the exact remediation steps for the Vercel redirect and the 404 metadata conflict before making changes.
- Delete the `www.zerd.si` → `zerd.si` domain redirect, since Vercel’s built-in apex-to-www redirect already sends `zerd.si` → `www.zerd.si`.
- Report the site's canonical host and redirect chain (zerd.si → www.zerd.si via 308) when answering SEO questions, and prefer www.zerd.si as the canonical domain.
- Include every canonical documentation page in the sitemap and improve the sitemap/SEO setup comprehensively.
- Complete the full SEO, AEO, and GEO optimization, not only the sitemap changes.
- Commit the changes in separate, logical parts.
- Read `.zerd/loop/20261120-145934.md`, continue the next specified work without changing unrelated code, run each exact verification command, and record the completed and remaining work under `# Progress`; set the final line to `STATUS: DONE` only after every checklist item passes.
- Ensure the terminal AI coding agent section has sufficient top margin.

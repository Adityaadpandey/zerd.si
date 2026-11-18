// One place for the site's name, address and one-line pitch, so metadata, JSON-LD and llms.txt never disagree.
// The site's canonical address. Everything SEO-critical points here: page canonicals, the sitemap,
// robots.txt and the JSON-LD. It is the host that actually serves the site, so the other host can
// redirect to it without Google seeing every URL as "an info page that redirects".
export const SITE = "https://www.zerd.si";
export const NAME = "zerd";
export const TAGLINE = "the coding agent that proves its work";
export const DESCRIPTION =
  "zerd is a terminal AI coding agent that runs tests on its changes, has a second model review the real diff, and verifies every file path it mentions.";
// The home page title. Leads with the search phrase, keeps the brand first so it still reads as a name.
export const HOME_TITLE = `${NAME}: the terminal AI coding agent that verifies its own work`;

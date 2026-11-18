// Who writes the docs, as one entity the site can point at.
//
// The docs are written by one person, and saying so is what lets an answer engine attribute a claim to
// a source instead of to "zerd", a tool name. Every page's `author` points at this one node, so the
// person is the same entity everywhere on the site.
//
// The name and the personal site match what zerd itself tells the model when a user asks who made it
// (src/prompts.rs), so the site and the tool give the same answer.
export const AUTHOR = {
  name: "Aditya Dutt Pandey",
  // The handle the work is published under.
  github: "https://github.com/Adityaadpandey",
  site: "https://lab.adpandey.dev",
  jobTitle: "Author of zerd",
  // What this person is known for on the site. Answer engines read these as subject-matter signals.
  knowsAbout: [
    "AI coding agents",
    "terminal AI coding agents",
    "code verification and static analysis",
    "LLM prompt caching and context management",
    "Rust CLI development",
    "sandboxing and application security",
  ],
} as const;

export const AUTHOR_URL = AUTHOR.github;

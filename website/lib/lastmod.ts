import { execFileSync } from "node:child_process";

// The date a file last changed in git, as an ISO string, or undefined when git can't say
// (a shallow clone, a build with no .git). Returning nothing is deliberate: a made-up "modified today"
// on every page teaches Google to ignore the date, so we only send it when it is true.
//
// The date is also returned as the epoch milliseconds, because comparing two ISO strings that carry
// different UTC offsets by text order gives the wrong answer: "2026-11-17T09:13:48+05:30" and
// "2026-11-17T02:00:00+01:00" are the same moment, but the second sorts first.
export function lastModified(file: string): string | undefined {
  const stamp = lastModifiedAt(file);
  return stamp === undefined ? undefined : new Date(stamp).toISOString();
}

// The same date as milliseconds since the epoch, for comparing dates across timezones.
export function lastModifiedAt(file: string): number | undefined {
  return gitDate(file, ["log", "-1", "--format=%cI"]);
}

// The date the file was first committed, for `datePublished`. A page that only says when it last
// changed reads as brand new whenever it is touched, so the publish date stays put and the modified
// date moves. Undefined when git can't say, for the same reason as above.
export function published(file: string): string | undefined {
  const stamp = gitDate(file, ["log", "--diff-filter=A", "--follow", "-1", "--format=%cI"]);
  return stamp === undefined ? undefined : new Date(stamp).toISOString();
}

function gitDate(file: string, args: string[]): number | undefined {
  try {
    const out = execFileSync("git", [...args, "--", file], { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }).trim();
    if (!out) return undefined;
    const ms = Date.parse(out);
    return Number.isNaN(ms) ? undefined : ms;
  } catch {
    return undefined;
  }
}

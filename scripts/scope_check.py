#!/usr/bin/env python3
"""Post-run scope check: did the agent stay inside its lane?

    scope_check.py BASE_SHA [--repo DIR] [--forbid GLOB ...] [--allow-tests]
    scope_check.py --selftest

Compares the working tree (committed and uncommitted, new files included)
against BASE_SHA and prints one receipt check entry as JSON, for example:

    {"name": "scope", "status": "fail", "detail": "forbidden path: prisma/migrations/0042_x/migration.sql"}

Exit code is 0 on pass, 1 on fail. It fails when the change:
  - touches a forbidden path (default: prisma/migrations, .env*, .github)
  - adds a new .sql file (migrations are raw SQL written by people)
  - deletes an existing test file
  - weakens an existing test: fewer assertions than before, or a new skip
Pass --allow-tests when the issue is itself about tests.
"""
import fnmatch
import json
import os
import re
import subprocess
import sys
import tempfile

DEFAULT_FORBIDDEN = ["prisma/migrations/*", ".env*", ".github/*"]

TEST_FILE = re.compile(r"(\.(spec|test)\.[jt]sx?$)|(^|/)(__tests__|tests?|e2e)/")
ASSERTION = re.compile(r"\bexpect\s*\(|\bassert\w*\s*\(|\.should\b")
SKIP = re.compile(r"\b(it|test|describe)\.(skip|fixme)\b|\b(xit|xdescribe|xtest)\s*\(")


def git(repo, *args):
    return subprocess.run(["git", "-C", repo, *args], capture_output=True, text=True, check=True).stdout


def is_forbidden(path, patterns):
    # A pattern like "prisma/migrations/*" must also catch "apps/server/prisma/migrations/x/y.sql",
    # so try it against every tail of the path: a/b/c -> a/b/c, b/c, c.
    parts = path.split("/")
    tails = ["/".join(parts[i:]) for i in range(len(parts))]
    return any(fnmatch.fnmatch(t, p) for p in patterns for t in tails)


def changes(repo, base):
    """Returns (status, path) pairs. Status is A, M or D; untracked files count as A."""
    out = []
    for line in git(repo, "diff", "--name-status", "--no-renames", base).splitlines():
        status, path = line.split("\t", 1)
        out.append((status[0], path))
    for path in git(repo, "ls-files", "--others", "--exclude-standard").splitlines():
        out.append(("A", path))
    return out


def assertion_delta(repo, base, path):
    """Added minus removed assertions in one file. Negative means the test got weaker."""
    delta, new_skips = 0, 0
    for line in git(repo, "diff", "-U0", base, "--", path).splitlines():
        if line.startswith(("+++", "---")):
            continue
        if line.startswith("+"):
            delta += len(ASSERTION.findall(line))
            new_skips += len(SKIP.findall(line))
        elif line.startswith("-"):
            delta -= len(ASSERTION.findall(line))
            new_skips -= len(SKIP.findall(line))
    return delta, new_skips


def check(repo, base, forbidden=None, allow_tests=False):
    forbidden = forbidden or DEFAULT_FORBIDDEN
    problems = []
    for status, path in changes(repo, base):
        if is_forbidden(path, forbidden):
            problems.append(f"forbidden path: {path}")
        if status == "A" and path.endswith(".sql"):
            problems.append(f"new .sql file: {path}")
        if allow_tests or not TEST_FILE.search(path):
            continue
        if status == "D":
            problems.append(f"test deleted: {path}")
        elif status == "M":
            # ponytail: counts assertion-looking lines, so a rewritten assertion nets zero
            # and a clever weakening (looser matcher, same count) slips through. Hidden tests catch those.
            delta, skips = assertion_delta(repo, base, path)
            if delta < 0:
                problems.append(f"test weakened, {-delta} fewer assertions: {path}")
            if skips > 0:
                problems.append(f"test skipped: {path}")
    if problems:
        return {"name": "scope", "status": "fail", "detail": "; ".join(problems)}
    return {"name": "scope", "status": "pass", "detail": "no forbidden paths touched, no tests weakened"}


def selftest():
    """Builds a throwaway repo and checks each rule fires, and a clean change passes."""
    with tempfile.TemporaryDirectory() as repo:
        def sh(*a):
            subprocess.run(["git", "-C", repo, *a], check=True, capture_output=True)

        def write(rel, text):
            full = os.path.join(repo, rel)
            os.makedirs(os.path.dirname(full), exist_ok=True)
            with open(full, "w") as f:
                f.write(text)

        sh("init", "-q")
        sh("config", "user.email", "t@t")
        sh("config", "user.name", "t")
        write("src/a.ts", "export const a = 1\n")
        write("src/a.spec.ts", "expect(a).toBe(1)\nexpect(b).toBe(2)\n")
        write("src/b.spec.ts", "expect(b).toBe(2)\n")
        sh("add", "-A")
        sh("commit", "-qm", "base")
        base = git(repo, "rev-parse", "HEAD").strip()

        def reset():
            sh("reset", "-q", "--hard", base)
            sh("clean", "-qfd")

        def status():
            return check(repo, base)["status"]

        assert status() == "pass", "no change should pass"

        write("src/a.ts", "export const a = 2\n")
        assert status() == "pass", "plain source edit should pass"
        reset()

        write("apps/server/prisma/migrations/0042_x/migration.sql", "alter table t;\n")
        assert status() == "fail", "migration (nested path) should fail"
        reset()

        write("db/new.sql", "select 1;\n")
        assert status() == "fail", "new .sql should fail"
        reset()

        write(".env.local", "X=1\n")
        assert status() == "fail", ".env should fail"
        reset()

        write(".github/workflows/ci.yml", "on: push\n")
        assert status() == "fail", ".github should fail"
        reset()

        os.remove(os.path.join(repo, "src/b.spec.ts"))
        assert status() == "fail", "deleted test should fail"
        assert check(repo, base, allow_tests=True)["status"] == "pass", "allow-tests should let a delete through"
        reset()

        write("src/a.spec.ts", "expect(a).toBe(1)\n")
        assert status() == "fail", "dropped assertion should fail"
        reset()

        write("src/a.spec.ts", "expect(a).toBe(1)\nexpect(b).toEqual(2)\n")
        assert status() == "pass", "rewritten assertion nets zero and should pass"
        reset()

        write("src/a.spec.ts", "it.skip('x', () => {})\nexpect(a).toBe(1)\nexpect(b).toBe(2)\n")
        assert status() == "fail", "new skip should fail"
    print("scope_check selftest ok")


def main(argv):
    if "--selftest" in argv:
        selftest()
        return 0
    args = [a for a in argv if not a.startswith("--")]
    if not args:
        print(__doc__, file=sys.stderr)
        return 2
    repo = argv[argv.index("--repo") + 1] if "--repo" in argv else "."
    forbid = [argv[i + 1] for i, a in enumerate(argv) if a == "--forbid"] or None
    # --repo and --forbid values are not the base sha, so take the first bare arg that is neither
    skip = {argv[i + 1] for i, a in enumerate(argv) if a in ("--repo", "--forbid")}
    base = next(a for a in args if a not in skip)
    result = check(repo, base, forbid, "--allow-tests" in argv)
    print(json.dumps(result))
    return 0 if result["status"] == "pass" else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))

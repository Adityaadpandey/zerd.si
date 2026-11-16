# Changelog

All notable changes to zerd. Newest first. Versions follow [semver](https://semver.org).

## Unreleased

- Checks run automatically in the interactive prompt again. One still running after 30 seconds (`verify_wait_sec`) moves to the background and prints its result when it finishes; a failure reaches the model with your next message.
- When no check covers the changed files, zerd asks the model once to run something that proves the change.
- An `unverified` result now says why (no check for these files, checks still running, token budget, a check that was already failing, docs only). It used to print "No verification commands found" for all of them.

## 0.1.2

- The project is now called **zerd** (it was melon). Config moves to `.zerd/` and `~/.zerd`, env vars to `ZERD_*`. Nothing is read from the old names, so move your files by hand.
- Install from `https://zerd.si/install.sh` or `https://zerd.si/install.ps1`.
- New site at [zerd.si](https://zerd.si): a landing page with a recorded zerd run, and these docs as searchable pages. The docs are built from the README, so the README stays the one place they are written.
- Community files: contributing guide, code of conduct, security policy, issue forms and a PR template. CI gets weekly dependency updates and a RustSec audit.

## 0.1.1

First tagged release of the Rust binary.

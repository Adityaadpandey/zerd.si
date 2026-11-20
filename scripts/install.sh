#!/bin/sh
# Downloads and verifies the latest zerd release for this machine into ~/.local/bin.
#   curl -fsSL https://www.zerd.si/install.sh | sh
set -e
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64)              t=aarch64-apple-darwin ;;
  Darwin-x86_64)             t=x86_64-apple-darwin ;;
  Linux-x86_64)              t=x86_64-unknown-linux-musl ;;
  Linux-aarch64|Linux-arm64) t=aarch64-unknown-linux-musl ;;
  *) echo "no prebuilt zerd for $(uname -s)-$(uname -m); build from source with cargo" >&2; exit 1 ;;
esac
dir="${ZERD_BIN:-$HOME/.local/bin}"
mkdir -p "$dir"
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
rel="https://github.com/Adityaadpandey/zerd.si/releases/latest/download/zerd-$t"
# Prefer the xz build (about 30% smaller, so a faster download); older releases and machines without xz use gz.
ext=tar.gz; flag=-z
if command -v xz >/dev/null 2>&1 && curl -fsIL --retry 2 "$rel.tar.xz" >/dev/null 2>&1; then ext=tar.xz; flag=-J; fi
base="$rel.$ext"
# The tiny checksum file downloads alongside the binary instead of after it, and a real progress bar shows on a terminal.
curl -fsSL --retry 3 --retry-connrefused "$base.sha256" -o "$tmp/m.sha256" &
sumpid=$!
if [ -t 2 ]; then
  echo "downloading zerd ($t)" >&2
  curl -fL --retry 3 --retry-connrefused --progress-bar "$base" -o "$tmp/m.pkg" || { kill "$sumpid" 2>/dev/null; echo "download failed" >&2; exit 1; }
else
  curl -fsSL --retry 3 --retry-connrefused "$base" -o "$tmp/m.pkg" || { kill "$sumpid" 2>/dev/null; echo "download failed" >&2; exit 1; }
fi
wait "$sumpid" || { echo "couldn't fetch the checksum, not installing" >&2; exit 1; }
want="$(cut -d' ' -f1 "$tmp/m.sha256")"
got="$( (sha256sum "$tmp/m.pkg" 2>/dev/null || shasum -a 256 "$tmp/m.pkg") | cut -d' ' -f1)"
[ "$want" = "$got" ] || { echo "checksum mismatch, not installing" >&2; exit 1; }
tar $flag -x -C "$dir" -f "$tmp/m.pkg" zerd
echo "installed $dir/zerd"
case ":$PATH:" in
  *":$dir:"*) ;;
  *) echo "  $dir isn't on your PATH yet. Add this to your shell profile:  export PATH=\"$dir:\$PATH\"" ;;
esac
command -v git >/dev/null 2>&1 || echo "  note: zerd needs git; install it first"
echo
echo "Next: cd into a repo and run  zerd   (first launch walks you through connecting a model)"
echo "      zerd doctor   checks that everything is set up"

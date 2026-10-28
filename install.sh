#!/usr/bin/env bash
set -euo pipefail

repo="${SNAG_REPO:-zetsuexe10/snag}"
prefix="${PREFIX:-$HOME/.local}"
bin="$prefix/bin"

target_triple() {
  case "$(uname -s)-$(uname -m)" in
    Linux-x86_64) echo x86_64-unknown-linux-musl ;;
    Darwin-x86_64) echo x86_64-apple-darwin ;;
    Darwin-arm64) echo aarch64-apple-darwin ;;
    *) return 1 ;;
  esac
}

mkdir -p "$bin"

if command -v cargo >/dev/null 2>&1; then
  cargo install --git "https://github.com/$repo" --locked --root "$prefix"
  echo "installed $bin/snag"
  exit 0
fi

triple="$(target_triple)" || {
  echo "no prebuilt binary for $(uname -s)-$(uname -m)" >&2
  echo "install Rust, then run: cargo install --git https://github.com/$repo --locked" >&2
  exit 1
}

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
archive="snag-$triple.tar.gz"

if command -v gh >/dev/null 2>&1 &&
  gh release download --repo "$repo" --pattern "$archive" --dir "$work" --clobber 2>/dev/null; then
  :
else
  curl -fsSL "https://github.com/$repo/releases/latest/download/$archive" -o "$work/$archive"
fi

tar -xzf "$work/$archive" -C "$work"
install -m 755 "$work/snag" "$bin/snag"
echo "installed $bin/snag"

case ":$PATH:" in
  *":$bin:"*) ;;
  *) echo "add $bin to PATH to run snag from anywhere" ;;
esac

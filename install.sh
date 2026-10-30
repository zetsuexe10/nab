#!/usr/bin/env bash
set -euo pipefail

repo="${NAB_REPO:-zetsuexe10/nab"
prefix="${PREFIX:-$HOME/.local}"
bin="$prefix/bin"

target_triple() {
  case "$(uname -s)-$(uname -m)" in
    Linux-x86_64) echo x86_64-unknown-linux-musl ;;
    Linux-aarch64) echo aarch64-unknown-linux-musl ;;
    *) return 1 ;;
  esac
}

mkdir -p "$bin"

if command -v cargo >/dev/null 2>&1; then
  cargo install --git "https://github.com/$repo" --locked --root "$prefix"
  echo "installed $bin/nab"
  exit 0
fi

triple="$(target_triple)" || {
  echo "no prebuilt binary for $(uname -s)-$(uname -m)" >&2
  echo "install Rust, then run: cargo install --git https://github.com/$repo --locked" >&2
  exit 1
}

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
archive="nab-$triple.tar.gz"

if command -v gh >/dev/null 2>&1 &&
  gh release download --repo "$repo" --pattern "$archive" --dir "$work" --clobber 2>/dev/null; then
  :
else
  curl -fsSL "https://github.com/$repo/releases/latest/download/$archive" -o "$work/$archive"
fi

tar -xzf "$work/$archive" -C "$work"
install -m 755 "$work/nab" "$bin/nab"
echo "installed $bin/nab"

case ":$PATH:" in
  *":$bin:"*) ;;
  *) echo "add $bin to PATH to run nab from anywhere" ;;
esac

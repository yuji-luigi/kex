#!/bin/sh
# Install the latest kex release: curl -fsSL https://raw.githubusercontent.com/yuji-luigi/kex/main/install.sh | sh
# KEX_INSTALL_DIR overrides the folder (default ~/.local/bin).
set -eu

repo="yuji-luigi/kex"
dir="${KEX_INSTALL_DIR:-$HOME/.local/bin}"

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) target=aarch64-apple-darwin ;;
  Darwin-x86_64) target=x86_64-apple-darwin ;;
  Linux-x86_64) target=x86_64-unknown-linux-musl ;;
  Linux-aarch64 | Linux-arm64) target=aarch64-unknown-linux-musl ;;
  *) echo "kex: no prebuilt binary for $(uname -s) $(uname -m); use: cargo install --git https://github.com/$repo" >&2; exit 1 ;;
esac

name="kex-$target.tar.gz"
url="https://github.com/$repo/releases/latest/download/$name"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "downloading $name"
curl -fsSL --proto '=https' "$url" -o "$tmp/$name"
curl -fsSL --proto '=https' "$url.sha256" -o "$tmp/$name.sha256"
expected="$(cut -d' ' -f1 "$tmp/$name.sha256")"
if command -v sha256sum >/dev/null; then actual="$(sha256sum "$tmp/$name" | cut -d' ' -f1)"
else actual="$(shasum -a 256 "$tmp/$name" | cut -d' ' -f1)"; fi
[ "$expected" = "$actual" ] || { echo "kex: checksum mismatch, not installing" >&2; exit 1; }

tar -xzf "$tmp/$name" -C "$tmp"
mkdir -p "$dir"
mv "$tmp/kex" "$dir/kex"
chmod 755 "$dir/kex"
echo "installed $("$dir/kex" --version) to $dir/kex"
case ":$PATH:" in
  *":$dir:"*) ;;
  *) echo "add $dir to your PATH, e.g.: echo 'export PATH=\"$dir:\$PATH\"' >> ~/.zshrc" ;;
esac

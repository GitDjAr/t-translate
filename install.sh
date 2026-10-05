#!/bin/sh
# One-click install for Linux / macOS:
#   curl -fsSL https://raw.githubusercontent.com/GitDjAr/t-translate/main/install.sh | sh
# Optional env: T_MIRROR (download prefix), T_INSTALL_DIR (default ~/.local/bin)
set -e
REPO="GitDjAr/t-translate"
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64)  ASSET="t-linux-x64" ;;
  Darwin-arm64)  ASSET="t-macos-arm64" ;;
  *) echo "unsupported platform: $(uname -s)-$(uname -m)" >&2; exit 1 ;;
esac
DIR="${T_INSTALL_DIR:-$HOME/.local/bin}"
URL="${T_MIRROR}https://github.com/$REPO/releases/latest/download/$ASSET"

mkdir -p "$DIR"
echo "Downloading $URL"
curl -fL "$URL" -o "$DIR/t.new"
chmod +x "$DIR/t.new"
mv -f "$DIR/t.new" "$DIR/t"

case ":$PATH:" in
  *":$DIR:"*) ;;
  *) echo "Add to PATH:  export PATH=\"$DIR:\$PATH\"  (put it in ~/.bashrc or ~/.zshrc)" ;;
esac
"$DIR/t" --version
echo "Done. Try:  t git -h     (rename the command with: t --alias)"

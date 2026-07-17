#!/usr/bin/env bash
set -euo pipefail

PKG_INPUT="${1:?usage: packaging/macos/smoke-installer.sh <pkg>}"
[[ -f "$PKG_INPUT" ]] || { echo "pkg does not exist: $PKG_INPUT" >&2; exit 1; }
PKG="$(cd "$(dirname "$PKG_INPUT")" && pwd -P)/$(basename "$PKG_INPUT")"
SENTINEL_DIR="$HOME/Library/Application Support/Ariadne"
SENTINEL="$SENTINEL_DIR/release-smoke-sentinel"
mkdir -p "$SENTINEL_DIR"
printf 'preserve-on-upgrade-and-uninstall\n' > "$SENTINEL"

cleanup() {
  sudo rm -rf /Applications/Ariadne.app
}
trap cleanup EXIT

sudo installer -pkg "$PKG" -target /
/Applications/Ariadne.app/Contents/MacOS/Ariadne.Desktop --verify-installation
sudo installer -pkg "$PKG" -target /
test "$(cat "$SENTINEL")" = "preserve-on-upgrade-and-uninstall"
sudo rm -rf /Applications/Ariadne.app
test "$(cat "$SENTINEL")" = "preserve-on-upgrade-and-uninstall"

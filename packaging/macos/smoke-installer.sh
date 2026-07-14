#!/usr/bin/env bash
set -euo pipefail

PKG="$(realpath "${1:?usage: packaging/macos/smoke-installer.sh <pkg>}")"
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

#!/usr/bin/env bash
set -euo pipefail

PKG_INPUT="${1:?usage: packaging/macos/smoke-installer.sh <pkg> <dmg>}"
DMG_INPUT="${2:?usage: packaging/macos/smoke-installer.sh <pkg> <dmg>}"
[[ -f "$PKG_INPUT" ]] || { echo "pkg does not exist: $PKG_INPUT" >&2; exit 1; }
[[ -f "$DMG_INPUT" ]] || { echo "dmg does not exist: $DMG_INPUT" >&2; exit 1; }
PKG="$(cd "$(dirname "$PKG_INPUT")" && pwd -P)/$(basename "$PKG_INPUT")"
DMG="$(cd "$(dirname "$DMG_INPUT")" && pwd -P)/$(basename "$DMG_INPUT")"
SENTINEL_DIR="$HOME/Library/Application Support/Ariadne"
SENTINEL="$SENTINEL_DIR/release-smoke-sentinel"
MOUNT_POINT="$(mktemp -d "${TMPDIR:-/tmp}/ariadne-dmg-smoke.XXXXXX")"
DMG_MOUNTED=0
mkdir -p "$SENTINEL_DIR"
printf 'preserve-on-upgrade-and-uninstall\n' > "$SENTINEL"

cleanup() {
  if [[ "$DMG_MOUNTED" == "1" ]]; then
    hdiutil detach -quiet "$MOUNT_POINT" || true
  fi
  rmdir "$MOUNT_POINT" 2>/dev/null || true
  sudo rm -rf /Applications/Ariadne.app
}
trap cleanup EXIT

hdiutil attach -readonly -nobrowse -mountpoint "$MOUNT_POINT" "$DMG" >/dev/null
DMG_MOUNTED=1
DMG_APP="$MOUNT_POINT/Ariadne.app"
[[ -d "$DMG_APP" ]] || { echo "dmg does not contain Ariadne.app" >&2; exit 1; }
codesign --verify --deep --strict --verbose=2 "$DMG_APP"
"$DMG_APP/Contents/MacOS/Ariadne.Desktop" --verify-installation
if [[ "${ARIADNE_REQUIRE_SIGNED_RELEASE:-0}" == "1" ]]; then
  pkgutil --check-signature "$PKG"
  xcrun stapler validate "$PKG"
  xcrun stapler validate "$DMG"
  spctl --assess --type install --verbose=2 "$PKG"
  spctl --assess --type execute --verbose=2 "$DMG_APP"
fi
hdiutil detach -quiet "$MOUNT_POINT"
DMG_MOUNTED=0

sudo installer -pkg "$PKG" -target /
/Applications/Ariadne.app/Contents/MacOS/Ariadne.Desktop --verify-installation
sudo installer -pkg "$PKG" -target /
test "$(cat "$SENTINEL")" = "preserve-on-upgrade-and-uninstall"
sudo rm -rf /Applications/Ariadne.app
test "$(cat "$SENTINEL")" = "preserve-on-upgrade-and-uninstall"

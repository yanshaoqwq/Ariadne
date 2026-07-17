#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PACKAGE_INPUT="${1:?usage: packaging/macos/build-installer.sh <assembled-package> [output-directory]}"
[[ -d "$PACKAGE_INPUT" ]] || { echo "assembled package does not exist: $PACKAGE_INPUT" >&2; exit 1; }
PACKAGE_DIR="$(cd "$PACKAGE_INPUT" && pwd -P)"
OUTPUT_INPUT="${2:-$ROOT/artifacts}"
mkdir -p "$OUTPUT_INPUT"
OUTPUT_DIR="$(cd "$OUTPUT_INPUT" && pwd -P)"
MANIFEST="$PACKAGE_DIR/release-manifest.json"
VERSION="$(jq -er '.version' "$MANIFEST")"
RID="$(jq -er '.rid' "$MANIFEST")"
case "$RID" in osx-x64|osx-arm64) ;; *) echo "macOS packaging requires an osx RID" >&2; exit 2 ;; esac

if [[ "${ARIADNE_REQUIRE_SIGNED_RELEASE:-0}" == "1" ]]; then
  [[ -n "${ARIADNE_MACOS_SIGNING_IDENTITY:-}" ]] || { echo "formal release requires ARIADNE_MACOS_SIGNING_IDENTITY" >&2; exit 1; }
  [[ -n "${ARIADNE_MACOS_INSTALLER_IDENTITY:-}" ]] || { echo "formal release requires ARIADNE_MACOS_INSTALLER_IDENTITY" >&2; exit 1; }
fi

STAGE="$(mktemp -d "${TMPDIR:-/tmp}/ariadne-macos.XXXXXX")"
trap 'rm -rf "$STAGE"' EXIT
PKG_ROOT="$STAGE/pkgroot"
APP="$PKG_ROOT/Ariadne.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp -a "$PACKAGE_DIR/." "$APP/Contents/MacOS/"
cp "$ROOT/packaging/macos/Info.plist" "$APP/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $VERSION" "$APP/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $VERSION" "$APP/Contents/Info.plist"

ICONSET="$STAGE/Ariadne.iconset"
mkdir -p "$ICONSET"
for size in 16 32 128 256 512; do
  cp "$PACKAGE_DIR/Integration/icons/ariadne-$size.png" "$ICONSET/icon_${size}x${size}.png"
  double=$((size * 2))
  if [[ -f "$PACKAGE_DIR/Integration/icons/ariadne-$double.png" ]]; then
    cp "$PACKAGE_DIR/Integration/icons/ariadne-$double.png" "$ICONSET/icon_${size}x${size}@2x.png"
  fi
done
sips -z 1024 1024 "$PACKAGE_DIR/Integration/icons/ariadne-512.png" --out "$ICONSET/icon_512x512@2x.png" >/dev/null
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/Ariadne.icns"

SIGNING_IDENTITY="${ARIADNE_MACOS_SIGNING_IDENTITY:--}"
codesign --force --deep --options runtime --sign "$SIGNING_IDENTITY" "$APP" >&2
"$APP/Contents/MacOS/Ariadne.Desktop" --verify-installation >&2

PKG="$OUTPUT_DIR/Ariadne-$VERSION-$RID.pkg"
PKG_ARGS=(--root "$PKG_ROOT" --identifier io.github.yanshaoqwq.ariadne --version "$VERSION" --install-location /Applications)
if [[ -n "${ARIADNE_MACOS_INSTALLER_IDENTITY:-}" ]]; then
  PKG_ARGS+=(--sign "$ARIADNE_MACOS_INSTALLER_IDENTITY")
fi
pkgbuild "${PKG_ARGS[@]}" "$PKG" >&2

DMG_ROOT="$STAGE/dmg"
mkdir -p "$DMG_ROOT"
cp -a "$APP" "$DMG_ROOT/"
ln -s /Applications "$DMG_ROOT/Applications"
hdiutil create -quiet -fs HFS+ -srcfolder "$DMG_ROOT" -volname Ariadne "$OUTPUT_DIR/Ariadne-$VERSION-$RID.dmg" >&2
DMG="$OUTPUT_DIR/Ariadne-$VERSION-$RID.dmg"

NOTARY_ARGS=()
if [[ -n "${ARIADNE_MACOS_NOTARY_PROFILE:-}" ]]; then
  NOTARY_ARGS=(--keychain-profile "$ARIADNE_MACOS_NOTARY_PROFILE")
elif [[ -n "${ARIADNE_MACOS_NOTARY_APPLE_ID:-}${ARIADNE_MACOS_NOTARY_TEAM_ID:-}${ARIADNE_MACOS_NOTARY_PASSWORD:-}" ]]; then
  [[ -n "${ARIADNE_MACOS_NOTARY_APPLE_ID:-}" && -n "${ARIADNE_MACOS_NOTARY_TEAM_ID:-}" && -n "${ARIADNE_MACOS_NOTARY_PASSWORD:-}" ]] || {
    echo "macOS notarization requires Apple ID, team ID and app-specific password together" >&2
    exit 1
  }
  NOTARY_ARGS=(
    --apple-id "$ARIADNE_MACOS_NOTARY_APPLE_ID"
    --team-id "$ARIADNE_MACOS_NOTARY_TEAM_ID"
    --password "$ARIADNE_MACOS_NOTARY_PASSWORD"
  )
fi
if [[ "${ARIADNE_REQUIRE_SIGNED_RELEASE:-0}" == "1" && ${#NOTARY_ARGS[@]} -eq 0 ]]; then
  echo "formal release requires macOS notarization credentials or a keychain profile" >&2
  exit 1
fi
if (( ${#NOTARY_ARGS[@]} > 0 )); then
  xcrun notarytool submit "$PKG" "${NOTARY_ARGS[@]}" --wait >&2
  xcrun stapler staple "$PKG" >&2
  xcrun notarytool submit "$DMG" "${NOTARY_ARGS[@]}" --wait >&2
  xcrun stapler staple "$DMG" >&2
fi
printf '%s\n' "$PKG"

#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PACKAGE_DIR="$(realpath "${1:?usage: packaging/linux/build-deb.sh <assembled-package> [output-directory]}")"
OUTPUT_DIR="${2:-$ROOT/artifacts}"
MANIFEST="$PACKAGE_DIR/release-manifest.json"
VERSION="$(jq -er '.version' "$MANIFEST")"
RID="$(jq -er '.rid' "$MANIFEST")"
case "$RID" in
  linux-x64) ARCH="amd64" ;;
  linux-arm64) ARCH="arm64" ;;
  *) echo "deb packaging requires linux-x64 or linux-arm64, got $RID" >&2; exit 2 ;;
esac
if [[ "${ARIADNE_REQUIRE_SIGNED_RELEASE:-0}" == "1" && -z "${ARIADNE_LINUX_SIGNING_KEY:-}" ]]; then
  echo "formal release requires ARIADNE_LINUX_SIGNING_KEY" >&2
  exit 1
fi

STAGE="$(mktemp -d "${TMPDIR:-/tmp}/ariadne-deb.XXXXXX")"
trap 'rm -rf "$STAGE"' EXIT
mkdir -p "$STAGE/DEBIAN" "$STAGE/opt/ariadne" "$STAGE/usr/share/applications"
cp -a "$PACKAGE_DIR/." "$STAGE/opt/ariadne/"
install -m 0644 "$PACKAGE_DIR/Integration/linux/ariadne.desktop" "$STAGE/usr/share/applications/ariadne.desktop"
for size in 16 24 32 48 64 128 256 512; do
  install -Dm0644 \
    "$PACKAGE_DIR/Integration/icons/ariadne-$size.png" \
    "$STAGE/usr/share/icons/hicolor/${size}x${size}/apps/ariadne.png"
done

INSTALLED_SIZE="$(du -sk "$STAGE/opt/ariadne" | awk '{print $1}')"
sed \
  -e "s/@VERSION@/$VERSION/g" \
  -e "s/@ARCHITECTURE@/$ARCH/g" \
  -e "s/@INSTALLED_SIZE@/$INSTALLED_SIZE/g" \
  "$ROOT/packaging/linux/control.in" > "$STAGE/DEBIAN/control"
install -m 0755 "$ROOT/packaging/linux/postinst" "$STAGE/DEBIAN/postinst"
install -m 0755 "$ROOT/packaging/linux/postrm" "$STAGE/DEBIAN/postrm"

mkdir -p "$OUTPUT_DIR"
DEB="$OUTPUT_DIR/Ariadne_${VERSION}_${ARCH}.deb"
dpkg-deb --root-owner-group --build "$STAGE" "$DEB"
if [[ -n "${ARIADNE_LINUX_SIGNING_KEY:-}" ]]; then
  gpg --batch --yes --local-user "$ARIADNE_LINUX_SIGNING_KEY" --armor --detach-sign "$DEB"
fi
echo "$DEB"

#!/usr/bin/env bash
set -euo pipefail

DESKTOP_INPUT="${1:?usage: packaging/macos/sign-release-binaries.sh <desktop-publish> <rust-bin-directory>}"
RUST_INPUT="${2:?usage: packaging/macos/sign-release-binaries.sh <desktop-publish> <rust-bin-directory>}"
[[ -d "$DESKTOP_INPUT" ]] || { echo "desktop publish directory is missing: $DESKTOP_INPUT" >&2; exit 1; }
[[ -d "$RUST_INPUT" ]] || { echo "Rust binary directory is missing: $RUST_INPUT" >&2; exit 1; }
DESKTOP_DIR="$(cd "$DESKTOP_INPUT" && pwd -P)"
RUST_DIR="$(cd "$RUST_INPUT" && pwd -P)"
if [[ "${ARIADNE_REQUIRE_SIGNED_RELEASE:-0}" == "1" && -z "${ARIADNE_MACOS_SIGNING_IDENTITY:-}" ]]; then
  echo "formal release requires ARIADNE_MACOS_SIGNING_IDENTITY before manifest assembly" >&2
  exit 1
fi
SIGNING_IDENTITY="${ARIADNE_MACOS_SIGNING_IDENTITY:--}"

for required in "$DESKTOP_DIR/Ariadne.Desktop" "$RUST_DIR/ariadne" "$RUST_DIR/ariadne-ipc"; do
  [[ -f "$required" ]] || { echo "required macOS release binary is missing before signing: $required" >&2; exit 1; }
done

SIGNED_COUNT=0
sign_macho() {
  local candidate="$1"
  if file -b "$candidate" | grep -q '^Mach-O'; then
    codesign --force --options runtime --sign "$SIGNING_IDENTITY" "$candidate" >&2
    codesign --verify --strict --verbose=2 "$candidate" >&2
    SIGNED_COUNT=$((SIGNED_COUNT + 1))
  fi
}

while IFS= read -r -d '' candidate; do
  sign_macho "$candidate"
done < <(find "$DESKTOP_DIR" -type f -print0)
sign_macho "$RUST_DIR/ariadne"
sign_macho "$RUST_DIR/ariadne-ipc"

(( SIGNED_COUNT > 0 )) || { echo "macOS release publish contains no Mach-O files to sign" >&2; exit 1; }
printf 'signed and verified %d Mach-O release files before manifest assembly\n' "$SIGNED_COUNT" >&2

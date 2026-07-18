#!/usr/bin/env bash
set -euo pipefail

DEB="$(realpath "${1:?usage: packaging/linux/smoke-deb.sh <deb>}")"
if [[ "${ARIADNE_REQUIRE_SIGNED_RELEASE:-0}" == "1" ]]; then
  [[ -f "$DEB.asc" ]] || { echo "formal release detached signature is missing: $DEB.asc" >&2; exit 1; }
  gpg --batch --verify "$DEB.asc" "$DEB"
fi
ROOT="$(mktemp -d "${TMPDIR:-/tmp}/ariadne-deb-smoke.XXXXXX")"
USER_DATA="$(mktemp -d "${TMPDIR:-/tmp}/ariadne-user-data.XXXXXX")"
trap 'rm -rf "$ROOT" "$USER_DATA"' EXIT

printf 'preserve-on-upgrade-and-uninstall\n' > "$USER_DATA/sentinel"
dpkg-deb --extract "$DEB" "$ROOT"
"$ROOT/opt/ariadne/Ariadne.Desktop" --verify-installation

# 再次解包模拟同版本覆盖升级；用户数据位于包管理范围外，必须保持。
dpkg-deb --extract "$DEB" "$ROOT"
test "$(cat "$USER_DATA/sentinel")" = "preserve-on-upgrade-and-uninstall"
rm -rf "$ROOT/opt/ariadne" "$ROOT/usr"
test "$(cat "$USER_DATA/sentinel")" = "preserve-on-upgrade-and-uninstall"

CONTENTS="$ROOT/deb-contents.txt"
dpkg-deb --contents "$DEB" > "$CONTENTS"
grep -Eq './opt/ariadne/(LICENSE|Backend/ariadne-ipc)' "$CONTENTS"
if grep -Eq '/(target|obj|\.git|secrets\.json)(/|$)' "$CONTENTS"; then
  echo "deb contains forbidden development or credential content" >&2
  exit 1
fi

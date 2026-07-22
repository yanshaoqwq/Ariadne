#!/usr/bin/env bash
set -euo pipefail

DEB="$(realpath "${1:?usage: packaging/linux/smoke-deb.sh <deb>}")"
if [[ "${ARIADNE_REQUIRE_SIGNED_RELEASE:-0}" == "1" ]]; then
  [[ -f "$DEB.asc" ]] || { echo "formal release detached signature is missing: $DEB.asc" >&2; exit 1; }
  gpg --batch --verify "$DEB.asc" "$DEB"
fi
ROOT="$(mktemp -d "${TMPDIR:-/tmp}/ariadne-deb-smoke.XXXXXX")"
SANDBOX_HOME="$(mktemp -d "${TMPDIR:-/tmp}/ariadne-user-home.XXXXXX")"
export HOME="$SANDBOX_HOME/home"
export XDG_DATA_HOME="$SANDBOX_HOME/xdg-data"
USER_DATA="$XDG_DATA_HOME/Ariadne"
trap 'rm -rf "$ROOT" "$SANDBOX_HOME"' EXIT

mkdir -p "$USER_DATA"
printf 'preserve-on-upgrade-and-uninstall\n' > "$USER_DATA/sentinel"
assert_user_data_preserved() {
  test "$(cat "$XDG_DATA_HOME/Ariadne/sentinel")" = "preserve-on-upgrade-and-uninstall"
}

dpkg-deb --extract "$DEB" "$ROOT"
"$ROOT/opt/ariadne/Ariadne.Desktop" --verify-installation
assert_user_data_preserved

# 再次解包模拟同版本覆盖升级；用户数据位于包管理范围外，必须保持。
dpkg-deb --extract "$DEB" "$ROOT"
assert_user_data_preserved
rm -rf "$ROOT/opt/ariadne" "$ROOT/usr"
assert_user_data_preserved

CONTENTS="$ROOT/deb-contents.txt"
dpkg-deb --contents "$DEB" > "$CONTENTS"
grep -Eq './opt/ariadne/(LICENSE|Backend/ariadne-ipc)' "$CONTENTS"
if grep -Eq '/(target|obj|\.git|secrets\.json)(/|$)' "$CONTENTS"; then
  echo "deb contains forbidden development or credential content" >&2
  exit 1
fi

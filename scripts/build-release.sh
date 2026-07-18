#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RID="${1:?usage: scripts/build-release.sh <rid> [output-directory]}"
OUTPUT="${2:-$ROOT/artifacts/staging/ariadne-$RID}"
LOCAL_TOOLCHAIN="$ROOT/.rustup/toolchains/stable-aarch64-unknown-linux-gnu/bin"
CARGO_BIN="${CARGO:-}"

case "$RID" in
  linux-x64) RUST_TARGET="x86_64-unknown-linux-gnu" ;;
  linux-arm64) RUST_TARGET="aarch64-unknown-linux-gnu" ;;
  win-x64) RUST_TARGET="x86_64-pc-windows-msvc" ;;
  osx-x64) RUST_TARGET="x86_64-apple-darwin" ;;
  osx-arm64) RUST_TARGET="aarch64-apple-darwin" ;;
  *) echo "unsupported release RID: $RID" >&2; exit 2 ;;
esac

if [[ -z "$CARGO_BIN" ]]; then
  if [[ -x "$LOCAL_TOOLCHAIN/cargo" ]]; then
    CARGO_BIN="$LOCAL_TOOLCHAIN/cargo"
  else
    CARGO_BIN="cargo"
  fi
fi
if [[ -z "${RUSTC:-}" && -x "$LOCAL_TOOLCHAIN/rustc" ]]; then
  export RUSTC="$LOCAL_TOOLCHAIN/rustc"
fi
if [[ -z "${RUSTDOC:-}" && -x "$LOCAL_TOOLCHAIN/rustdoc" ]]; then
  export RUSTDOC="$LOCAL_TOOLCHAIN/rustdoc"
fi

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
DESKTOP_PUBLISH="$ROOT/artifacts/publish/$RID"
RUST_BIN_DIR="$CARGO_TARGET_DIR/$RUST_TARGET/release"

"$CARGO_BIN" build \
  --manifest-path "$ROOT/core/Cargo.toml" \
  --release \
  --locked \
  --features system-keychain \
  --target "$RUST_TARGET" \
  --bin ariadne \
  --bin ariadne-ipc

dotnet restore "$ROOT/desktop/Ariadne.Desktop/Ariadne.Desktop.csproj" --runtime "$RID"
dotnet publish "$ROOT/desktop/Ariadne.Desktop/Ariadne.Desktop.csproj" \
  --configuration Release \
  --runtime "$RID" \
  --self-contained true \
  --no-restore \
  --output "$DESKTOP_PUBLISH" \
  -p:DebugType=None \
  -p:DebugSymbols=false \
  -p:PublishSingleFile=false

# Windows 第一方二进制必须在 ReleaseTool 生成 manifest 前完成签名，
# 否则安装器中的文件与 release-manifest.json 哈希会产生第二份身份。
if [[ "$RID" == "win-x64" ]]; then
  pwsh -NoProfile -NonInteractive -File "$ROOT/packaging/windows/sign-release-binaries.ps1" \
    -DesktopPublishDirectory "$DESKTOP_PUBLISH" \
    -RustBinaryDirectory "$RUST_BIN_DIR"
fi

dotnet run --project "$ROOT/tools/Ariadne.ReleaseTool/Ariadne.ReleaseTool.csproj" -- \
  assemble \
  --root "$ROOT" \
  --rid "$RID" \
  --desktop-publish "$DESKTOP_PUBLISH" \
  --rust-bin-dir "$RUST_BIN_DIR" \
  --output "$OUTPUT"

dotnet run --project "$ROOT/tools/Ariadne.ReleaseTool/Ariadne.ReleaseTool.csproj" -- \
  verify-package \
  --root "$ROOT" \
  --package "$OUTPUT"

echo "$OUTPUT"

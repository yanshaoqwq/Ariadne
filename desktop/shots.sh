#!/usr/bin/env bash
# 一次多截：复用同一个 Xvfb，逐页启动应用并截图，省去每页重开 Xvfb / 搜端口的开销。
#   ./shots.sh [页面...] [--out DIR]
# 页面 id：welcome workspace works git run_logs templates settings
# 不传页面时截全部主页面。输出默认 /tmp/ar-<page>.png。
#
# 说明：应用页面在启动时由 ARIADNE_UI_START_PAGE 固定（App.axaml.cs），
# 故每页需重启应用一次；但 Xvfb / 构建只做一次。
set -euo pipefail

PROJ_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$PROJ_DIR/.." && pwd)"
CSPROJ="$PROJ_DIR/Ariadne.Desktop"
LOCAL_TOOLCHAIN="$ROOT_DIR/.rustup/toolchains/stable-aarch64-unknown-linux-gnu/bin"
CARGO_BIN="${CARGO:-}"
if [[ -z "$CARGO_BIN" ]]; then
  if [[ -x "$LOCAL_TOOLCHAIN/cargo" ]]; then CARGO_BIN="$LOCAL_TOOLCHAIN/cargo"; else CARGO_BIN="$(command -v cargo)"; fi
fi
[[ -z "${RUSTC:-}" && -x "$LOCAL_TOOLCHAIN/rustc" ]] && export RUSTC="$LOCAL_TOOLCHAIN/rustc"
[[ -z "${RUSTDOC:-}" && -x "$LOCAL_TOOLCHAIN/rustdoc" ]] && export RUSTDOC="$LOCAL_TOOLCHAIN/rustdoc"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT_DIR/target}"
BACKEND_IPC="$CARGO_TARGET_DIR/debug/ariadne-ipc"

OUT_DIR="/tmp"
PAGES=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --out) OUT_DIR="$2"; shift 2 ;;
    *) PAGES+=("$1"); shift ;;
  esac
done
[[ ${#PAGES[@]} -eq 0 ]] && PAGES=(welcome workspace works git run_logs templates settings)
mkdir -p "$OUT_DIR"

# 构建（后端 IPC + 桌面），仅一次
if [[ ! -x "$BACKEND_IPC" ]]; then
  "$CARGO_BIN" build --manifest-path "$ROOT_DIR/core/Cargo.toml" --bin ariadne-ipc
fi
dotnet build "$CSPROJ/Ariadne.Desktop.csproj" -v quiet --nologo --no-restore

# 选一个空闲 display
VDISP=""
for n in $(seq 90 119); do
  if [[ ! -e "/tmp/.X${n}-lock" && ! -S "/tmp/.X11-unix/X${n}" ]]; then VDISP=":${n}"; break; fi
done
[[ -z "$VDISP" ]] && { echo "无可用 Xvfb display" >&2; exit 1; }

Xvfb "$VDISP" -screen 0 1440x900x24 -ac +extension GLX +render -noreset >"/tmp/ar-shots-xvfb.log" 2>&1 &
XVFB_PID=$!
cleanup() { kill "$XVFB_PID" 2>/dev/null || true; }
trap cleanup EXIT
for _ in $(seq 1 40); do DISPLAY="$VDISP" xdpyinfo >/dev/null 2>&1 && break; sleep 0.25; done

png_ok() {
  python3 - "$1" <<'PY' 2>/dev/null || return 1
import sys
from pathlib import Path
p=Path(sys.argv[1])
try:
    from PIL import Image
except ImportError:
    sys.exit(0 if p.stat().st_size>20000 else 1)
im=Image.open(p); px=im.load(); w,h=im.size; nb=t=0
for y in range(0,h,4):
    for x in range(0,w,4):
        t+=1; r,g,b=px[x,y][:3]
        if r+g+b>60: nb+=1
sys.exit(0 if t and nb/t>=0.02 else 1)
PY
}

for page in "${PAGES[@]}"; do
  OUT="$OUT_DIR/ar-$page.png"
  echo "[shots] $page -> $OUT"
  DISPLAY="$VDISP" ARIADNE_BACKEND_IPC="$BACKEND_IPC" ARIADNE_UI_START_PAGE="$page" \
    dotnet run --project "$CSPROJ/Ariadne.Desktop.csproj" -v quiet --nologo --no-build \
    >"/tmp/ar-shots-app-$page.log" 2>&1 &
  APP_PID=$!
  # 等窗口映射
  for _ in $(seq 1 90); do
    kill -0 "$APP_PID" 2>/dev/null || break
    DISPLAY="$VDISP" xwininfo -root -tree 2>/dev/null | grep -Eiq 'Ariadne|Avalonia' && break
    sleep 1
  done
  sleep 4
  for attempt in 1 2 3 4 5; do
    ffmpeg -y -f x11grab -draw_mouse 0 -video_size 1440x900 -i "$VDISP" -frames:v 1 -update 1 "$OUT" >/dev/null 2>&1 || true
    png_ok "$OUT" && break
    sleep 3
  done
  kill "$APP_PID" 2>/dev/null || true
  wait "$APP_PID" 2>/dev/null || true
done
echo "[shots] 完成，输出在 $OUT_DIR/ar-*.png"

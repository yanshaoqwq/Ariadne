#!/usr/bin/env bash
# Ariadne 桌面 UI 启动器
#   ./run-ui.sh         在真实显示（DISPLAY，默认 :0）上开窗口，供人工查看
#   ./run-ui.sh shot    用 Xvfb 无头跑起来并截图为 PNG（供无显示环境验证渲染）
#   ./run-ui.sh build   仅编译
#   ./run-ui.sh probe   用 Release + Xvfb 采集 100/500/1000 节点 UI 性能证据
#
# 说明：UI 文案来自 core/resources/display_name.json；
# 后端 IPC 未连接时页面显示真实空态（不伪造数据）。
set -euo pipefail

PROJ_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$PROJ_DIR/.." && pwd)"
CSPROJ="$PROJ_DIR/Ariadne.Desktop"
MODE="${1:-run}"
LOCAL_TOOLCHAIN="$ROOT_DIR/.rustup/toolchains/stable-aarch64-unknown-linux-gnu/bin"
CARGO_BIN="${CARGO:-}"

if [[ -z "$CARGO_BIN" ]]; then
  if [[ -x "$LOCAL_TOOLCHAIN/cargo" ]]; then
    CARGO_BIN="$LOCAL_TOOLCHAIN/cargo"
  elif command -v cargo >/dev/null 2>&1; then
    CARGO_BIN="$(command -v cargo)"
  else
    echo "[build] cargo 未找到；请安装 Rust 或保留仓库 .rustup 本地工具链" >&2
    exit 127
  fi
fi

if [[ -z "${RUSTC:-}" && -x "$LOCAL_TOOLCHAIN/rustc" ]]; then
  export RUSTC="$LOCAL_TOOLCHAIN/rustc"
fi

if [[ -z "${RUSTDOC:-}" && -x "$LOCAL_TOOLCHAIN/rustdoc" ]]; then
  export RUSTDOC="$LOCAL_TOOLCHAIN/rustdoc"
fi

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT_DIR/target}"
BACKEND_IPC="$CARGO_TARGET_DIR/debug/ariadne-ipc"

build() {
  echo "[build] cargo build ($CARGO_BIN) ..."
  "$CARGO_BIN" build --manifest-path "$ROOT_DIR/core/Cargo.toml" --bin ariadne-ipc
  if [[ ! -x "$BACKEND_IPC" ]]; then
    echo "[build] 后端 IPC 未生成：$BACKEND_IPC" >&2
    exit 1
  fi
  echo "[build] dotnet build ..."
  dotnet build "$CSPROJ/Ariadne.Desktop.csproj" -v quiet --nologo --no-restore
}

case "$MODE" in
  build)
    build
    ;;

  run)
    build
    REAL_DISPLAY="${DISPLAY:-:0}"
    ICON_PNG="$CSPROJ/Assets/app-icon-512.png"
    echo "[run] 在显示 $REAL_DISPLAY 上启动窗口（Ctrl+C 关闭）..."
    echo "[run] 应用图标母版（定稿 35）：$ICON_PNG"
    DISPLAY="$REAL_DISPLAY" ARIADNE_BACKEND_IPC="$BACKEND_IPC" dotnet run --project "$CSPROJ/Ariadne.Desktop.csproj" -v quiet --nologo
    ;;

  install-dev-desktop)
    # .desktop 使用 Icon=ariadne；运行时 AppIconDesktopSync 会按个性化写入
    # ~/.local/share/icons/hicolor/*/apps/ariadne.png 并刷新缓存
    build
    OUT_DIR="${2:-$HOME/.local/share/applications}"
    ICON_SRC="$CSPROJ/Assets/app-icon-512.png"
    HICOLOR="$HOME/.local/share/icons/hicolor"
    BIN_DIR="$CSPROJ/bin/Debug/net10.0"
    mkdir -p "$OUT_DIR"
    for s in 16 24 32 48 64 128 256 512; do
      mkdir -p "$HICOLOR/${s}x${s}/apps"
      # 开发态先用静态母版；应用启动/改主题后会被主题色覆盖
      SRC="$CSPROJ/Assets/app-icon-${s}.png"
      [[ -f "$SRC" ]] || SRC="$ICON_SRC"
      cp -f "$SRC" "$HICOLOR/${s}x${s}/apps/ariadne.png"
    done
    mkdir -p "$HOME/.local/share/Ariadne/icons"
    cp -f "$ICON_SRC" "$HOME/.local/share/Ariadne/icons/ariadne.png"
    cat > "$OUT_DIR/ariadne.desktop" <<EOF
[Desktop Entry]
Type=Application
Version=1.0
Name=Ariadne
Comment=长篇小说创作工作台
Icon=ariadne
Exec=env ARIADNE_BACKEND_IPC=$BACKEND_IPC $BIN_DIR/Ariadne.Desktop
Path=$BIN_DIR
Terminal=false
Categories=Office;Publishing;
StartupWMClass=Ariadne.Desktop
EOF
    chmod +x "$OUT_DIR/ariadne.desktop" 2>/dev/null || true
    gtk-update-icon-cache -f -t "$HICOLOR" 2>/dev/null || true
    xdg-desktop-menu forceupdate 2>/dev/null || true
    echo "[install-dev-desktop] 仅开发机使用：已写入 $OUT_DIR/ariadne.desktop (Icon=ariadne)"
    echo "[install-dev-desktop] 正式安装请使用 packaging/ 生成的发行包"
    ;;

  shot)
    # Prefer existing ariadne-ipc when present so shot still works if cargo tree is dirty.
    OUT="${2:-$PROJ_DIR/ui-preview.png}"
    if [[ -x "$BACKEND_IPC" ]]; then
      echo "[shot] 复用已有后端 IPC：$BACKEND_IPC"
      echo "[build] dotnet build ..."
      dotnet build "$CSPROJ/Ariadne.Desktop.csproj" -v quiet --nologo --no-restore
    else
      build
    fi
    VDISP="${ARIADNE_SHOT_DISPLAY:-}"
    if [[ -z "$VDISP" ]]; then
      for display_num in $(seq 90 119); do
        if [[ ! -e "/tmp/.X${display_num}-lock" && ! -S "/tmp/.X11-unix/X${display_num}" ]]; then
          VDISP=":${display_num}"
          break
        fi
      done
    fi
    if [[ -z "$VDISP" ]]; then
      echo "[shot] 未找到可用 Xvfb display；请清理 /tmp/.X*-lock 后重试" >&2
      exit 1
    fi
    DISPLAY_ID="${VDISP#:}"
    SCRATCH_SHOT_DIR="${ARIADNE_SHOT_LOG_DIR:-/tmp}"
    mkdir -p "$SCRATCH_SHOT_DIR"
    XVFB_LOG="$SCRATCH_SHOT_DIR/ariadne-xvfb-${DISPLAY_ID}.log"
    APP_LOG="$SCRATCH_SHOT_DIR/ariadne-desktop-${DISPLAY_ID}.log"
    FFMPEG_LOG="$SCRATCH_SHOT_DIR/ariadne-ffmpeg-${DISPLAY_ID}.log"
    # Max wait for first mapped window (seconds); then settle before grab.
    SHOT_WAIT_MAX="${ARIADNE_SHOT_WAIT_MAX:-90}"
    SHOT_SETTLE_SEC="${ARIADNE_SHOT_SETTLE_SEC:-4}"
    XVFB_PID=""
    APP_PID=""
    cleanup_shot() {
      if [[ -n "$APP_PID" ]]; then
        kill "$APP_PID" 2>/dev/null || true
      fi
      if [[ -n "$XVFB_PID" ]]; then
        kill "$XVFB_PID" 2>/dev/null || true
      fi
    }
    trap cleanup_shot EXIT

    # Return 0 if PNG has enough non-black pixels (not an empty Xvfb frame).
    png_has_content() {
      local png="$1"
      [[ -f "$png" ]] || return 1
      python3 - "$png" <<'PY'
import sys
from pathlib import Path
path = Path(sys.argv[1])
try:
    from PIL import Image
except ImportError:
    # Fallback: reject tiny files (all-black 1440x900 PNG is usually ~4KB).
    sys.exit(0 if path.stat().st_size > 20000 else 1)
im = Image.open(path)
# Sample every 4th pixel for speed.
px = im.load()
w, h = im.size
nonblack = 0
total = 0
for y in range(0, h, 4):
    for x in range(0, w, 4):
        total += 1
        p = px[x, y]
        r, g, b = p[0], p[1], p[2]
        if r + g + b > 60:
            nonblack += 1
# Need at least ~2% of samples non-black (real UI, not pure Xvfb black).
sys.exit(0 if total and (nonblack / total) >= 0.02 else 1)
PY
    }

    # Block until a non-trivial client window is mapped on the virtual display.
    wait_for_ui_window() {
      local disp="$1"
      local deadline=$((SECONDS + SHOT_WAIT_MAX))
      local tree=""
      echo "[shot] 阻塞等待 UI 窗口出现（最长 ${SHOT_WAIT_MAX}s）..."
      while (( SECONDS < deadline )); do
        if [[ -n "$APP_PID" ]] && ! kill -0 "$APP_PID" 2>/dev/null; then
          echo "[shot] UI 进程已退出，日志如下：" >&2
          sed -n '1,200p' "$APP_LOG" >&2 || true
          return 1
        fi
        tree="$(DISPLAY="$disp" xwininfo -root -tree 2>/dev/null || true)"
        # Prefer Ariadne / Avalonia window names; fall back to any sizable child of root.
        if echo "$tree" | grep -Eiq 'Ariadne|Avalonia'; then
          echo "[shot] 检测到 Ariadne/Avalonia 窗口"
          return 0
        fi
        # "  0x... \"title\"  WxH+X+Y" with W,H both >= 200
        if echo "$tree" | grep -E '^\s+0x[0-9a-fA-F]+ .*: \(.*\)  [0-9]{3,}x[0-9]{3,}\+' >/dev/null 2>&1; then
          echo "[shot] 检测到已映射的客户端窗口"
          return 0
        fi
        sleep 1
      done
      echo "[shot] 等待窗口超时（${SHOT_WAIT_MAX}s）。xwininfo -root -tree：" >&2
      DISPLAY="$disp" xwininfo -root -tree 2>&1 | sed -n '1,80p' >&2 || true
      sed -n '1,200p' "$APP_LOG" >&2 || true
      return 1
    }

    echo "[shot] 启动 Xvfb $VDISP (1440x900) ..."
    Xvfb "$VDISP" -screen 0 1440x900x24 -ac +extension GLX +render -noreset >"$XVFB_LOG" 2>&1 &
    XVFB_PID=$!
    # Block until X server answers (not fixed sleep only).
    for _ in $(seq 1 40); do
      if DISPLAY="$VDISP" xdpyinfo >/dev/null 2>&1; then
        break
      fi
      if ! kill -0 "$XVFB_PID" 2>/dev/null; then
        echo "[shot] Xvfb 启动失败，日志如下：" >&2
        sed -n '1,120p' "$XVFB_LOG" >&2 || true
        exit 1
      fi
      sleep 0.25
    done
    if ! DISPLAY="$VDISP" xdpyinfo >/dev/null 2>&1; then
      echo "[shot] Xvfb 未就绪，日志如下：" >&2
      sed -n '1,120p' "$XVFB_LOG" >&2 || true
      exit 1
    fi

    echo "[shot] 无头启动 UI ..."
    DISPLAY="$VDISP" ARIADNE_BACKEND_IPC="$BACKEND_IPC" \
      dotnet run --project "$CSPROJ/Ariadne.Desktop.csproj" -v quiet --nologo --no-build \
      >"$APP_LOG" 2>&1 &
    APP_PID=$!

    if ! wait_for_ui_window "$VDISP"; then
      exit 1
    fi
    echo "[shot] 窗口已出现，再 settle ${SHOT_SETTLE_SEC}s 等首帧绘制..."
    sleep "$SHOT_SETTLE_SEC"

    capture_ok=0
    for attempt in 1 2 3 4 5; do
      echo "[shot] 截图尝试 #$attempt -> $OUT（-draw_mouse 0）"
      if ! ffmpeg -y -f x11grab -draw_mouse 0 -video_size 1440x900 -i "$VDISP" \
          -frames:v 1 -update 1 "$OUT" >"$FFMPEG_LOG" 2>&1; then
        echo "[shot] ffmpeg 失败，日志：" >&2
        sed -n '1,80p' "$FFMPEG_LOG" >&2 || true
        sleep 2
        continue
      fi
      if png_has_content "$OUT"; then
        capture_ok=1
        break
      fi
      echo "[shot] 截图疑似全黑/空帧（文件 $(stat -c%s "$OUT" 2>/dev/null || echo 0) bytes），继续等待绘制..."
      sleep 3
    done

    if [[ "$capture_ok" -ne 1 ]]; then
      echo "[shot] 多次截图仍无有效画面；app 日志：" >&2
      sed -n '1,200p' "$APP_LOG" >&2 || true
      exit 1
    fi

    cleanup_shot
    trap - EXIT
    echo "[shot] 完成：$OUT ($(stat -c%s "$OUT" 2>/dev/null || echo '?') bytes, non-black content verified)"
    ;;

  probe)
    OUT="${2:-/tmp/ariadne-desktop-ui-performance.json}"
    echo "[probe] dotnet Release build ..."
    dotnet build "$CSPROJ/Ariadne.Desktop.csproj" \
      --configuration Release \
      --no-restore \
      -v quiet \
      --nologo
    RELEASE_DLL="$CSPROJ/bin/Release/net10.0/Ariadne.Desktop.dll"
    if [[ ! -f "$RELEASE_DLL" ]]; then
      echo "[probe] Release 桌面程序集未生成：$RELEASE_DLL" >&2
      exit 1
    fi
    echo "[probe] Xvfb 1600x900 -> $OUT"
    xvfb-run -a -s "-screen 0 1600x900x24" \
      dotnet "$RELEASE_DLL" --release-ui-probe "$OUT"
    echo "[probe] 完成：$OUT"
    ;;

  *)
    echo "用法: $0 [run|shot|probe|build|install-dev-desktop]"
    exit 2
    ;;
esac

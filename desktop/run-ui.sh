#!/usr/bin/env bash
# Ariadne 桌面 UI 启动器
#   ./run-ui.sh         在真实显示（DISPLAY，默认 :0）上开窗口，供人工查看
#   ./run-ui.sh shot    用 Xvfb 无头跑起来并截图为 PNG（供无显示环境验证渲染）
#   ./run-ui.sh build   仅编译
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
    echo "[run] 在显示 $REAL_DISPLAY 上启动窗口（Ctrl+C 关闭）..."
    DISPLAY="$REAL_DISPLAY" ARIADNE_BACKEND_IPC="$BACKEND_IPC" dotnet run --project "$CSPROJ/Ariadne.Desktop.csproj" -v quiet --nologo
    ;;

  shot)
    build
    OUT="${2:-$PROJ_DIR/ui-preview.png}"
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
    XVFB_LOG="/tmp/ariadne-xvfb-${DISPLAY_ID}.log"
    APP_LOG="/tmp/ariadne-desktop-${DISPLAY_ID}.log"
    FFMPEG_LOG="/tmp/ariadne-ffmpeg-${DISPLAY_ID}.log"
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
    echo "[shot] 启动 Xvfb $VDISP (1440x900) ..."
    Xvfb "$VDISP" -screen 0 1440x900x24 >"$XVFB_LOG" 2>&1 &
    XVFB_PID=$!
    sleep 1.5
    if ! kill -0 "$XVFB_PID" 2>/dev/null; then
      echo "[shot] Xvfb 启动失败，日志如下：" >&2
      sed -n '1,120p' "$XVFB_LOG" >&2 || true
      exit 1
    fi

    echo "[shot] 无头启动 UI ..."
    DISPLAY="$VDISP" ARIADNE_BACKEND_IPC="$BACKEND_IPC" dotnet run --project "$CSPROJ/Ariadne.Desktop.csproj" -v quiet --nologo >"$APP_LOG" 2>&1 &
    APP_PID=$!
    # 等待窗口绘制完成
    sleep 20
    if ! kill -0 "$APP_PID" 2>/dev/null; then
      echo "[shot] UI 进程已退出，日志如下：" >&2
      sed -n '1,160p' "$APP_LOG" >&2 || true
      exit 1
    fi

    echo "[shot] 截图 -> $OUT"
    if ! ffmpeg -y -f x11grab -video_size 1440x900 -i "$VDISP" -frames:v 1 "$OUT" >"$FFMPEG_LOG" 2>&1; then
      echo "[shot] ffmpeg 截图失败，日志如下：" >&2
      sed -n '1,160p' "$FFMPEG_LOG" >&2 || true
      exit 1
    fi

    cleanup_shot
    trap - EXIT
    echo "[shot] 完成：$OUT"
    ;;

  *)
    echo "用法: $0 [run|shot|build]"
    exit 2
    ;;
esac

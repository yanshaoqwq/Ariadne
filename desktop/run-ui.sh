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

build() {
  echo "[build] cargo build ..."
  cargo build --manifest-path "$ROOT_DIR/core/Cargo.toml" --bin ariadne-ipc
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
    DISPLAY="$REAL_DISPLAY" dotnet run --project "$CSPROJ/Ariadne.Desktop.csproj" -v quiet --nologo
    ;;

  shot)
    build
    OUT="${2:-$PROJ_DIR/ui-preview.png}"
    VDISP=":99"
    echo "[shot] 启动 Xvfb $VDISP (1440x900) ..."
    Xvfb "$VDISP" -screen 0 1440x900x24 >/dev/null 2>&1 &
    XVFB_PID=$!
    sleep 1.5

    echo "[shot] 无头启动 UI ..."
    DISPLAY="$VDISP" dotnet run --project "$CSPROJ/Ariadne.Desktop.csproj" -v quiet --nologo >/dev/null 2>&1 &
    APP_PID=$!
    # 等待窗口绘制完成
    sleep 20

    echo "[shot] 截图 -> $OUT"
    ffmpeg -y -f x11grab -video_size 1440x900 -i "$VDISP" -frames:v 1 "$OUT" >/dev/null 2>&1 || {
      echo "[shot] ffmpeg 截图失败"; }

    kill "$APP_PID" 2>/dev/null || true
    kill "$XVFB_PID" 2>/dev/null || true
    echo "[shot] 完成：$OUT"
    ;;

  *)
    echo "用法: $0 [run|shot|build]"
    exit 2
    ;;
esac

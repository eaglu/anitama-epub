#!/usr/bin/env bash
# 启动 Anitama 装订台（WebUI）。
# 用法：./start.sh [归档根目录] [端口]     默认：.. 8095
# 监听地址默认 0.0.0.0（局域网可访问）；只想本机用就 BIND=127.0.0.1 ./start.sh
set -euo pipefail
cd "$(dirname "$(readlink -f "$0")")"

ROOT="${1:-..}"
PORT="${2:-8095}"
BIND="${BIND:-0.0.0.0}"
LOG="${LOG:-/tmp/anitama-epub-web.log}"

BIN=./target/release/anitama-epub-web
[ -x "$BIN" ] || BIN=./target/debug/anitama-epub-web
if [ ! -x "$BIN" ]; then
  echo "还没有编译，先跑：/home/faye/.cargo/bin/cargo build --release" >&2
  exit 1
fi

if curl -sf -o /dev/null "http://127.0.0.1:$PORT/api/state"; then
  echo "已经在跑了 → http://127.0.0.1:$PORT/"
  exit 0
fi

PORT="$PORT" BIND="$BIND" setsid nohup "$BIN" "$ROOT" >> "$LOG" 2>&1 < /dev/null &
# 启动时要扫一遍归档（330 个专题 / 约 8 万个素材），4~7 秒后才监听端口
for _ in $(seq 1 30); do
  sleep 1
  if curl -sf -o /dev/null "http://127.0.0.1:$PORT/api/state"; then
    echo "就绪 → http://127.0.0.1:$PORT/   （日志：$LOG）"
    exit 0
  fi
done

echo "启动超时，看日志：$LOG" >&2
exit 1

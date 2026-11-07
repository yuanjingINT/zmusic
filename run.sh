#!/usr/bin/env bash
# 启动网易云音乐终端播放器 (每次启动自动更新并确保 API 服务在线)
set -euo pipefail
cd "$(dirname "$0")"

./setup.sh update

exec cargo run --release "$@"

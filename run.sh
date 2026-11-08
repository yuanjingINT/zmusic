#!/usr/bin/env bash
# 启动 zmusic：不联网、不重启服务、不做无谓构建
#   1) 二进制不存在或比源码旧 → 增量 cargo build（不用 cargo run，省掉每次的构建检查）
#   2) ncm-api 用户服务没跑 → 拉起来；已经在跑就完全不动它
#   3) exec 播放器
# 想更新 API 服务请手动：./setup.sh update
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
BIN="$ROOT/target/release/zmusic"
SERVICE="${ZMUSIC_API_SERVICE:-ncm-api}"

needs_build() {
  [[ ! -x "$BIN" ]] && return 0
  local newer
  newer="$(find "$ROOT/src" "$ROOT/Cargo.toml" "$ROOT/Cargo.lock" \
              -newer "$BIN" -print -quit 2>/dev/null || true)"
  [[ -n "$newer" ]]
}

if needs_build; then
  echo "[zmusic] 源码比二进制新，增量构建中…" >&2
  ( cd "$ROOT" && cargo build --release )
fi

# API 服务：只检查、不等待。首屏不依赖它；没跑时启动，起不来说明原因。
if command -v systemctl >/dev/null 2>&1; then
  if ! systemctl --user is-active --quiet "$SERVICE" 2>/dev/null; then
    if systemctl --user start "$SERVICE" 2>/dev/null; then
      echo "[zmusic] 已启动 $SERVICE" >&2
    else
      echo "[zmusic] $SERVICE 启动失败，网易云相关功能可能不可用" >&2
    fi
  fi
fi

exec "$BIN" "$@"

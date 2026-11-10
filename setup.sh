#!/usr/bin/env bash
# 部署并启动网易云音乐 API 服务 (NeteaseCloudMusicApiEnhanced, npm 版)
# 用法:
#   ./setup.sh           # 首次安装/确保服务运行
#   ./setup.sh update    # 更新到最新版并重启 (每次启动播放器时自动执行)
set -euo pipefail

INSTALL_DIR="$HOME/.local/share/ncm-api"
API_DIR="$INSTALL_DIR/node_modules/@neteasecloudmusicapienhanced/api"
PORT="${NCM_API_PORT:-3000}"
SERVICE="ncm-api.service"
MODE="${1:-ensure}"

command -v node >/dev/null || { echo "缺少 node (需要 Node >= 22)"; exit 1; }
command -v npm >/dev/null || { echo "缺少 npm"; exit 1; }

api_version() {
    ( cd "$INSTALL_DIR" && npm ls @neteasecloudmusicapienhanced/api --depth=0 2>/dev/null ) \
        | grep "@neteasecloudmusicapienhanced/api@" \
        | grep -oE "[0-9]+\.[0-9]+\.[0-9]+" | head -1 || echo "?"
}

install_api() {
    if [ ! -f "$API_DIR/app.js" ]; then
        echo "==> 首次安装 API 包 (npm)"
        mkdir -p "$INSTALL_DIR"
        cd "$INSTALL_DIR"
        npm init -y >/dev/null 2>&1 || true
        npm i @neteasecloudmusicapienhanced/api --no-audit --no-fund
    fi
}

patch_server() {
    # 修补 server.js 的解灰 (unblock) 逻辑:
    # 上游用 req.baseUrl 判断路由, 但 app.all() 注册的路由 req.baseUrl 为空,
    # 导致通用解锁永不触发。此处整体替换为匹配 req.path, 并支持批量解锁全部曲目。
    # 若上游结构变更导致补丁失败, 仅告警不中断启动 (服务仍可用, 只是无解灰)。
    SERVER_PY="$API_DIR/server.js"
    if ! grep -q "batch_unblock_patched" "$SERVER_PY"; then
        echo "==> 修补 server.js 解灰逻辑"
        if ! python3 - "$SERVER_PY" <<'PYEOF'
import sys
p = sys.argv[1]
src = open(p, encoding='utf-8').read()
marker = "req.baseUrl === '/song/url/v1'"
if marker not in src:
    sys.exit(0)  # 已修补或版本不同
start = src.find("        // 夹带私货部分")
assert start != -1, 'unblock comment not found'
# 平衡括号扫描, 找到整个 if 块的结束
i = src.find("if (", start)
depth = 0
end = None
in_str = False
while i < len(src):
    c = src[i]
    if in_str:
        if c == '\\':
            i += 2
            continue
        if c in "'\"`":
            in_str = False
    else:
        if c in "'\"`":
            in_str = True
        elif c == '(':
            depth += 1
        elif c == ')':
            depth -= 1
            if depth == 0:
                # 到达 if (...) 的结尾
                j = src.find('\n', i)
                end = src.find('\n        }', j)
                end = src.find('\n        }', end + 9) if end != -1 else -1
                if end != -1:
                    end += len('\n        }')
                break
    i += 1
assert end is not None, 'could not find block end'

new_block = '''        // 夹带私货部分：如果开启了通用解锁，并且是获取歌曲URL的接口，则尝试解锁（如果需要的话）ヾ(≧▽≦*)o // batch_unblock_patched
        if (
          (req.path === '/song/url/v1' || req.baseUrl === '/song/url/v1') &&
          process.env.ENABLE_GENERAL_UNBLOCK === 'true'
        ) {
          const {
            matchID,
          } = require('@neteasecloudmusicapienhanced/unblockmusic-utils')
          const data = moduleResponse.body.data
          if (Array.isArray(data)) {
            for (const song of data) {
              if (
                song &&
                (song.freeTrialInfo !== null ||
                  !song.url ||
                  [1, 4].includes(song.fee))
              ) {
                logger.info('Starting unblock(uses general unblock):', song.id)
                try {
                  const result = await matchID(song.id)
                  if (result && result.data && result.data.url) {
                    song.url = result.data.url
                    song.freeTrialInfo = null
                    logger.info('Unblock success! url:', song.url)
                  } else {
                    logger.warn('Unblock failed for song:', song.id)
                  }
                } catch (e) {
                  logger.error('Unblock error for song:', song.id, e.message)
                }
              }
              if (song && song.url && song.url.includes('kuwo')) {
                const proxy = process.env.PROXY_URL
                const useProxy = process.env.ENABLE_PROXY || 'false'
                if (useProxy === 'true' && proxy) {
                  song.proxyUrl = proxy + song.url
                }
              }
            }
          }
        }'''

src = src[:start] + new_block + src[end:]
open(p, 'w', encoding='utf-8').write(src)
print('patched:', p)
PYEOF
        then
            echo "OK"
        else
            echo "!! 补丁失败 (上游代码可能已变更), 已跳过。服务仍可运行, 但 VIP 解灰可能失效。"
        fi
    fi
}

write_service() {
    mkdir -p "$HOME/.config/systemd/user"
    cat > "$HOME/.config/systemd/user/$SERVICE" <<EOF
[Unit]
Description=NetEase Cloud Music API (NeteaseCloudMusicApiEnhanced)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
Environment=PORT=$PORT
Environment=ENABLE_GENERAL_UNBLOCK=true
WorkingDirectory=$API_DIR
ExecStart=$(command -v node) app.js
Restart=on-failure
RestartSec=3
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=default.target
EOF
    systemctl --user daemon-reload
    systemctl --user enable --now "$SERVICE" >/dev/null 2>&1 || true
}

wait_ready() {
    echo -n "==> 等待 API 就绪"
    for _ in $(seq 1 25); do
        if curl -s -o /dev/null --max-time 2 "http://127.0.0.1:$PORT/personalized?limit=1"; then
            echo " OK -> http://127.0.0.1:$PORT (版本 $(api_version))"
            return 0
        fi
        echo -n "."
        sleep 1
    done
    echo " 失败。查看日志: journalctl --user -u $SERVICE -n 50"
    return 1
}

METING_DIR="$HOME/.local/share/zmuse/meting"
SRC_DIR="$(cd "$(dirname "$0")" && pwd)/sidecar"
PKG_SIDECAR_DIR="${PKG_SIDECAR_DIR:-/usr/lib/zmuse/sidecar}"
USER_UNIT="$HOME/.config/systemd/user/zmuse-meting.service"
LEGACY_UNIT="$HOME/.config/systemd/user/meting-sidecar.service"

# 侧车是不是由 pacman 包提供（包里的更新，绝不能被源码副本盖住）
pkg_sidecar_installed() {
    [ -f "$PKG_SIDECAR_DIR/meting.js" ] &&
        pacman -Qo "$PKG_SIDECAR_DIR/meting.js" >/dev/null 2>&1
}

# 改名遗留：老版本的用户级 unit 叫 meting-sidecar.service，它不归 pacman 管，
# 不停掉就会和新 unit 抢 8123 端口。
cleanup_legacy_unit() {
    [ -f "$LEGACY_UNIT" ] || return 0
    echo "==> 发现改名前的旧 unit：$LEGACY_UNIT"
    systemctl --user disable --now meting-sidecar.service >/dev/null 2>&1 || true
    mv "$LEGACY_UNIT" "$LEGACY_UNIT.bak-$(date +%Y%m%d%H%M%S)" 2>/dev/null || true
    systemctl --user daemon-reload >/dev/null 2>&1 || true
    echo "    已停用并备份（不删原文件），改为启用 zmuse-meting.service"
}

install_meting() {
    # Meting 侧车：覆盖腾讯/酷狗/百度/酷我。没起或装不上时，zmuse 里那组源自动置灰。
    command -v node >/dev/null || { echo "!! 没有 node，跳过 Meting 侧车"; return 0; }

    if pkg_sidecar_installed; then
        # 已经用包安装了：不要复制源码侧车、也不要写用户级 unit（会遮蔽包内 unit）
        echo "==> Meting 侧车由软件包提供（$PKG_SIDECAR_DIR），跳过源码安装"
        if [ -f "$USER_UNIT" ]; then
            echo "!! 发现旧用户级 unit：$USER_UNIT"
            echo "   它会遮蔽包内 unit，跑的是旧的 ~/.local/share/zmuse/meting（没有 type=duration）。"
            echo "   建议处理："
            echo "     mv \"$USER_UNIT\" \"$USER_UNIT.bak-$(date +%Y%m%d)\""
            echo "     systemctl --user daemon-reload && systemctl --user restart zmuse-meting"
        fi
        systemctl --user daemon-reload >/dev/null 2>&1 || true
        systemctl --user enable --now zmuse-meting.service >/dev/null 2>&1 || true
        return 0
    fi

    if [ ! -f "$METING_DIR/node_modules/@meting/core/package.json" ]; then
        echo "==> 安装 Meting 侧车"
        mkdir -p "$METING_DIR"
        cp -f "$SRC_DIR/meting.js" "$SRC_DIR/package.json" "$METING_DIR/" 2>/dev/null || return 0
        ( cd "$METING_DIR" && npm i --no-audit --no-fund ) || { echo "!! Meting 侧车安装失败，跳过"; return 0; }
    fi
    mkdir -p "$HOME/.config/systemd/user"
    cat > "$USER_UNIT" <<UNIT
[Unit]
Description=Meting sidecar for zmuse (loopback only)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
WorkingDirectory=$METING_DIR
Environment=METING_PORT=8123
ExecStart=$(command -v node) meting.js
Restart=on-failure
RestartSec=3

[Install]
WantedBy=default.target
UNIT
    systemctl --user daemon-reload
    systemctl --user enable --now zmuse-meting.service >/dev/null 2>&1 || true
}

install_api
cleanup_legacy_unit
install_meting
write_service

case "$MODE" in
    update)
        echo "==> 检查 API 更新"
        OLD="$(api_version)"
        if ( cd "$INSTALL_DIR" && npm update @neteasecloudmusicapienhanced/api --no-audit --no-fund ); then
            NEW="$(api_version)"
            if [ "$OLD" != "$NEW" ]; then
                echo "==> API 已更新: $OLD -> $NEW"
            else
                echo "==> API 已是最新版 ($NEW)"
            fi
        else
            echo "!! API 更新失败 (网络?), 使用现有版本 $(api_version)"
        fi
        patch_server
        echo "==> 重启 API 服务"
        systemctl --user restart "$SERVICE"
        wait_ready
        ;;
    *)
        if systemctl --user is-active --quiet "$SERVICE"; then
            echo "==> API 已在运行 (版本 $(api_version))"
        else
            patch_server
            echo "==> 启动 API 服务"
            systemctl --user restart "$SERVICE"
            wait_ready
        fi
        ;;
esac

#!/usr/bin/env bash
# 安装 zmusic 到本机
#   - 编译 release 版本
#   - 安装 zmusic 命令到 ~/.local/bin (每次启动自动更新 API)
#   - 安装桌面启动器
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "$0")" && pwd)"
BIN_DIR="$HOME/.local/bin"
APP_DIR="$HOME/.local/share/applications"

echo "==> 编译 zmusic (release)"
cargo build --release --manifest-path "$PROJECT_DIR/Cargo.toml"

echo "==> 安装 zmusic 命令 -> $BIN_DIR/zmusic"
mkdir -p "$BIN_DIR"
cat > "$BIN_DIR/zmusic" <<EOF
#!/usr/bin/env bash
# zmusic 启动器: 每次启动自动更新 API 服务并运行播放器
exec "$PROJECT_DIR/run.sh" "\$@"
EOF
chmod +x "$BIN_DIR/zmusic"

echo "==> 安装桌面启动器 -> $APP_DIR/zmusic.desktop"
mkdir -p "$APP_DIR"
cat > "$APP_DIR/zmusic.desktop" <<EOF
[Desktop Entry]
Name=zmusic
Name[zh_CN]=zmusic 网易云音乐
Comment=NetEase Cloud Music terminal player
Comment[zh_CN]=网易云音乐终端播放器
Exec=$BIN_DIR/zmusic
Terminal=true
Type=Application
Categories=AudioVideo;Audio;Player;Music;
Keywords=music;netease;neteasemusic;
EOF

echo "==> 完成"
if command -v zmusic >/dev/null 2>&1; then
    echo "   已在 PATH 中, 直接输入 zmusic 即可启动"
else
    echo "   请确保 $BIN_DIR 在 PATH 中"
    echo "   可执行: echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.bashrc"
fi

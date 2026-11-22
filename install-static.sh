#!/bin/sh
# zmuse installer — one line, no root:
#
#   curl -fsSL https://repo.dito.asia/zmuse/install.sh | sh
#
# Installs a prebuilt static binary into ~/.local/bin (override with --prefix
# or ZMUSE_PREFIX). Nothing outside your home directory is touched, and the
# script never runs sudo. Re-run it to upgrade; --uninstall removes the binary.
#
# Options:
#   --prefix DIR   install into DIR (default ~/.local/bin or $ZMUSE_PREFIX)
#   --version X    install a specific version instead of "latest"
#   --uninstall    remove the installed binary
#   --insecure-sha-only  install without gpg (sha256 from SHA256SUMS still checked)
#   --no-sidecar   do not install the Meting sidecar / systemd user unit
#   --no-ncm       do not install the local NetEase API service
#   --ncm-port N   port for the NetEase API (default 3000)
#   --from-source  build from a release tag locally instead of downloading a tarball
#   --help
#
# --from-source (every distribution, needs a Rust toolchain):
#   curl -fsSL https://repo.dito.asia/zmuse/install.sh | sh -s -- --from-source
#
#   Fetch tag v<version> (latest release unless --version is given), run
#   `cargo build --release --locked` in a temporary directory, then install the
#   binary, sidecar, unit and docs exactly like the static route. The source tree
#   is deleted afterwards; only the build log is kept (/tmp/zmuse-build.log).
#   Missing build tools are reported with a per-distribution install command; this
#   script never installs them and never runs sudo. Compilation takes 1-3 minutes;
#   the static tarball stays the preferred route.
#
# The NetEase source needs a local API service (the npm package
# @neteasecloudmusicapienhanced/api, Node >= 22, listening on 127.0.0.1:3000).
# It is installed into ~/.local/share/ncm-api and run as a systemd --user unit;
# nothing global is touched and the user's npm config is not modified.
#
# Download layout (published by the release job):
#   <base>/latest                      plain-text version, atomically replaced
#   <base>/v<ver>/SHA256SUMS           one line per artifact
#   <base>/v<ver>/SHA256SUMS.sig       detached signature of SHA256SUMS
#   <base>/v<ver>/zmuse-<ver>-linux-<arch>.tar.gz
#   <base>/v<ver>/zmuse-<ver>-linux-<arch>.tar.gz (unpacked: zmuse-<ver>-linux-<arch>/zmuse)
#
# Environment (mirrors / CI):
#   ZMUSE_INSTALL_BASE  base URL of the download directory
#   ZMUSE_KEY_URL       the repository public key
#   ZMUSE_PREFIX        same as --prefix
#   ZMUSE_SOURCE_REPO   git URL to clone for --from-source
#   ZMUSE_SOURCE_ARCHIVE  fallback tarball URL used when the clone fails
#   ZMUSE_BUILD_LOG     build log path (default /tmp/zmuse-build.log)
set -eu

BASE="${ZMUSE_INSTALL_BASE:-https://repo.dito.asia/zmuse}"
KEY_URL="${ZMUSE_KEY_URL:-https://repo.dito.asia/x86_64/dito-repo.pub}"
# Pinned repository key. Do not make this configurable: it is the whole point.
EXPECTED_FP="6469F0BEB079C3EDCC47947627FEDD0D74F7D714"

PREFIX="${ZMUSE_PREFIX:-$HOME/.local/bin}"
VERSION=""
DO_UNINSTALL=0
DO_SOURCE=0
SOURCE_REPO="${ZMUSE_SOURCE_REPO:-https://git.tuxai.org/personal/zmusic.git}"
SOURCE_ARCHIVE_TMPL="${ZMUSE_SOURCE_ARCHIVE:-}"
BUILD_LOG="${ZMUSE_BUILD_LOG:-${TMPDIR:-/tmp}/zmuse-build.log}"
DO_GPG=1
DO_SIDECAR=1
DO_NCM=1
NCM_PORT="${ZMUSE_NCM_PORT:-3000}"

die() { printf 'zmuse: %s\n' "$*" >&2; exit 1; }
say() { printf '%s\n' "$*"; }

usage() {
    awk 'NR == 1 { next }
         /^set -eu/ { exit }
         { sub(/^# ?/, ""); print }' "$0" 2>/dev/null || true
    exit 0
}

while [ $# -gt 0 ]; do
    case "$1" in
        --prefix)    [ $# -ge 2 ] || die "--prefix needs a directory"; PREFIX="$2"; shift 2 ;;
        --version)   [ $# -ge 2 ] || die "--version needs a value"; VERSION="$2"; shift 2 ;;
        --uninstall) DO_UNINSTALL=1; shift ;;
        --no-gpg|--insecure-sha-only) DO_GPG=0; shift ;;
        --no-sidecar) DO_SIDECAR=0; shift ;;
        --no-ncm)     DO_NCM=0; shift ;;
        --ncm-port)   [ $# -ge 2 ] || die "--ncm-port needs a value"; NCM_PORT="$2"; shift 2 ;;
        --from-source|--source) DO_SOURCE=1; shift ;;
        --help|-h)   usage ;;
        *)           die "unknown option: $1 (try --help)" ;;
    esac
done

# 展开开头的 ~（POSIX：不能用 bash 的 ${VAR/#pat/rep}，dash 会报 Bad substitution）
case "$PREFIX" in
    "~")   PREFIX="$HOME" ;;
    "~/"*) PREFIX="$HOME/${PREFIX#??}" ;;
esac
BIN="$PREFIX/zmuse"

# ---------------------------------------------------------------- NetEase API
ncm_deps_hint() {
    say ""
    say "The NetEase source needs a local API service, which needs Node.js >= 22."
    if [ -r /etc/os-release ]; then
        . /etc/os-release
        case "${ID:-}${ID_LIKE:-}" in
            *arch*)    say "  sudo pacman -S nodejs npm" ;;
            *debian*|*ubuntu*|*linuxmint*) say "  sudo apt install nodejs npm   # 22+ may need nodesource" ;;
            *fedora*)  say "  sudo dnf install nodejs npm" ;;
            *opensuse*|*suse*) say "  sudo zypper install nodejs22 npm" ;;
            *alpine*)  say "  sudo apk add nodejs npm" ;;
            *)         say "  install Node.js >= 22 and npm with your package manager" ;;
        esac
    else
        say "  install Node.js >= 22 and npm with your package manager"
    fi
    say ""
    say "Until then zmuse runs with the sources that need no sidecar (YouTube,"
    say "Audius, ccMixter, ...). Re-run this installer later to add the API."
}

ncm_ready() {
    command -v curl >/dev/null 2>&1 || return 1
    curl -fsS -o /dev/null --max-time 2 "http://127.0.0.1:$NCM_PORT/personalized?limit=1" 2>/dev/null
}

install_ncm() {
    local dir="$HOME/.local/share/ncm-api"
    local api="$dir/node_modules/@neteasecloudmusicapienhanced/api"
    local unit="$HOME/.config/systemd/user/ncm-api.service"

    if ncm_ready; then
        say "  NetEase API already answers on 127.0.0.1:$NCM_PORT, leaving it alone"
        return 0
    fi

    if ! command -v node >/dev/null 2>&1 || ! command -v npm >/dev/null 2>&1; then
        say "  node/npm missing: skipping the local NetEase API"
        ncm_deps_hint
        return 0
    fi
    local ver
    ver="$(node -v 2>/dev/null | sed 's/^v//' | cut -d. -f1)"
    case "$ver" in
        ''|*[!0-9]*) ver=0 ;;
    esac
    if [ "$ver" -lt 22 ]; then
        say "  node $(node -v) is too old (need >= 22): skipping the local NetEase API"
        ncm_deps_hint
        return 0
    fi

    say "  installing the local NetEase API (npm, into ~/.local/share/ncm-api)"
    mkdir -p "$dir"
    ( cd "$dir" &&
      [ -f package.json ] || npm init -y >/dev/null 2>&1
      npm i @neteasecloudmusicapienhanced/api --no-audit --no-fund >/dev/null 2>&1 ) || {
        say "  npm install failed: skipping the local NetEase API (re-run later to retry)"
        return 0
    }
    [ -f "$api/app.js" ] || { say "  the API package did not land where expected, skipping"; return 0; }
    : > "$dir/.zmuse-installed"

    # 解灰补丁：上游用 req.baseUrl 判断路由，app.all() 注册的路由 baseUrl 为空，
    # 通用解锁永不触发；这里换成匹配 req.path 并支持批量。补丁失败不中断安装。
    if command -v python3 >/dev/null 2>&1 && ! grep -q "batch_unblock_patched" "$api/server.js"; then
        if python3 - "$api/server.js" >/dev/null 2>&1 <<'PYEOF'
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
            say "  unblock patch applied"
        else
            say "  unblock patch failed (upstream changed?); the API still runs, VIP unblocking may not"
        fi
    fi

    mkdir -p "$HOME/.config/systemd/user"
    cat > "$unit" <<UNIT
[Unit]
Description=NetEase Cloud Music API (NeteaseCloudMusicApiEnhanced)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
Environment=PORT=$NCM_PORT
Environment=ENABLE_GENERAL_UNBLOCK=true
WorkingDirectory=$api
ExecStart=$(command -v node) app.js
Restart=on-failure
RestartSec=3
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=default.target
UNIT
    if command -v systemctl >/dev/null 2>&1; then
        systemctl --user daemon-reload >/dev/null 2>&1 || true
        systemctl --user enable --now ncm-api.service >/dev/null 2>&1 || true
        printf '  waiting for the API'
        local i=0
        while [ $i -lt 25 ]; do
            if ncm_ready; then
                printf ' ok (127.0.0.1:%s)\n' "$NCM_PORT"
                return 0
            fi
            printf '.'
            i=$((i + 1))
            sleep 1
        done
        printf '\n'
        say "  the API did not answer within 25s; check: journalctl --user -u ncm-api -n 50"
    else
        say "  systemd is not available, so the API was installed but not started"
    fi
}

# ---------------------------------------------------------------- from source
# --from-source 只需要检测工具链并给出安装提示：本脚本不装包、不 sudo。
src_family() {
    ids=""
    if [ -r /etc/os-release ]; then
        ids="$( . /etc/os-release 2>/dev/null; printf '%s %s' "${ID:-}" "${ID_LIKE:-}" )"
    fi
    case "$ids" in
        *arch*)   printf 'arch' ;;
        *debian*|*ubuntu*|*linuxmint*|*pop*|*neon*) printf 'debian' ;;
        *fedora*|*rhel*|*centos*|*rocky*|*alma*)    printf 'fedora' ;;
        *suse*)                                     printf 'suse' ;;
        *alpine*)                                   printf 'alpine' ;;
        *void*)                                     printf 'void' ;;
        *)                                          printf 'other' ;;
    esac
}

# $1 = 工具，$2 = 发行版家族 -> 该发行版里的包名（可能多个）
src_pkg() {
    case "$1" in
        git)  printf 'git' ;;
        rust) case "$2" in
                  arch|fedora|suse|alpine|void) printf 'rust' ;;
                  *)                            printf 'rustc cargo' ;;
              esac ;;
        cc)   case "$2" in
                  debian) printf 'build-essential' ;;
                  alpine) printf 'build-base' ;;
                  *)      printf 'gcc' ;;
              esac ;;
        pkg-config) case "$2" in
                  debian) printf 'pkg-config' ;;
                  fedora) printf 'pkgconf-pkg-config' ;;
                  alpine) printf 'pkgconf' ;;
                  *)      printf 'pkg-config' ;;
              esac ;;
        openssl) case "$2" in
                  arch)   printf 'openssl' ;;
                  fedora|suse) printf 'openssl-devel' ;;
                  alpine) printf 'openssl-dev' ;;
                  *)      printf 'libssl-dev' ;;
              esac ;;
        make) case "$2" in
                  debian) printf 'build-essential' ;;
                  alpine) printf 'build-base' ;;
                  *)      printf 'make' ;;
              esac ;;
    esac
}

src_install_cmd() {
    case "$1" in
        arch)   printf 'sudo pacman -S --needed' ;;
        debian) printf 'sudo apt install' ;;
        fedora) printf 'sudo dnf install' ;;
        suse)   printf 'sudo zypper install' ;;
        alpine) printf 'sudo apk add' ;;
        void)   printf 'sudo xbps-install' ;;
        *)      printf 'install' ;;
    esac
}

# A >= B（点分版本，缺的位数按 0 算）
ver_ge() {
    awk -v a="$1" -v b="$2" 'BEGIN {
        n = split(a, x, "."); m = split(b, y, ".")
        for (i = 1; i <= 3; i++) {
            xv = (i <= n ? x[i] : 0) + 0
            yv = (i <= m ? y[i] : 0) + 0
            if (xv > yv) exit 0
            if (xv < yv) exit 1
        }
        exit 0
    }'
}

check_source_tools() {
    fam="$(src_family)"
    missing=""
    for t in git cargo rustc cc pkg-config make; do
        case "$t" in
            cc) command -v cc >/dev/null 2>&1 || command -v gcc >/dev/null 2>&1 ||
                    missing="$missing cc" ;;
            *)  command -v "$t" >/dev/null 2>&1 || missing="$missing $t" ;;
        esac
    done
    if command -v pkg-config >/dev/null 2>&1; then
        pkg-config --exists openssl 2>/dev/null || missing="$missing openssl"
    elif [ ! -f /usr/include/openssl/ssl.h ]; then
        missing="$missing openssl"
    fi
    [ -n "$missing" ] || return 0

    say ""
    say "--from-source needs a Rust and a C toolchain. Missing:"
    pkgs=""
    for t in $missing; do
        case "$t" in cargo|rustc) id=rust ;; *) id="$t" ;; esac
        pk="$(src_pkg "$id" "$fam")"
        say "  - $t  ->  $pk"
        for w in $pk; do
            case " $pkgs " in *" $w "*) ;; *) pkgs="$pkgs $w" ;; esac
        done
    done
    say ""
    say "Install them yourself (this script installs nothing and never runs sudo):"
    say "  $(src_install_cmd "$fam")$pkgs"
    case "$missing" in
        *cargo*|*rustc*)
            say ""
            say "If your distribution's Rust is older than this version needs (see"
            say "rust-version in Cargo.toml), use rustup instead:"
            say "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
            ;;
    esac
    die "missing build tools for --from-source"
}

check_rust_version() {
    want="$(awk -F'"' '/^rust-version[[:space:]]*=/{print $2; exit}' "$1/Cargo.toml" 2>/dev/null || true)"
    [ -n "$want" ] || return 0
    have="$(rustc --version 2>/dev/null | awk '{print $2}')"
    ver_ge "$have" "$want" && return 0
    say ""
    say "rustc $have is too old: Cargo.toml declares rust-version = $want"
    say "Update the toolchain:"
    say "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    die "need rustc >= $want"
}

# 用户级 unit。仓库里那份是 packaging/zmuse-meting-user.service（build-static 也用它），
# 这里只在老 tag 没有该文件时兜底；两份内容必须一致。
write_user_unit() {
    cat > "$1" <<'UNIT'
[Unit]
Description=Meting sidecar for zmuse (loopback only)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
WorkingDirectory=%h/.local/share/zmuse/sidecar
Environment=METING_PORT=8123
# 可选：把 cookie 放在这个文件里（600），别写进 unit 正文或仓库
EnvironmentFile=-%h/.config/zmuse/meting.env
ExecStart=/usr/bin/env node meting.js
Restart=on-failure
RestartSec=3

[Install]
WantedBy=default.target
UNIT
}

# 拉 tag 源码 -> 编译 -> 按静态包的布局暂存到 $PKG；二进制留给 $SRC
build_from_source() {
    check_source_tools
    : >"$BUILD_LOG"
    say "  fetching tag v$VERSION"
    if git clone --quiet --depth 1 --branch "v$VERSION" "$SOURCE_REPO" "$TMP/src" >>"$BUILD_LOG" 2>&1; then
        say "  cloned $SOURCE_REPO (v$VERSION)"
    else
        rm -rf "$TMP/src"
        archive="$SOURCE_ARCHIVE_TMPL"
        [ -n "$archive" ] ||
            archive="https://git.tuxai.org/personal/zmusic/-/archive/v$VERSION/zmusic-v$VERSION.tar.gz"
        say "  git clone failed (git.tuxai.org may be restarting), trying the archive"
        curl -fsSL -o "$TMP/src.tar.gz" "$archive" >>"$BUILD_LOG" 2>&1 ||
            die "could not fetch the source for v$VERSION: git.tuxai.org did not answer (it returns 502 while its container restarts). Retry later, pass --version, or use the prebuilt install"
        mkdir -p "$TMP/src"
        tar -xzf "$TMP/src.tar.gz" -C "$TMP/src" --strip-components=1 ||
            die "could not unpack the source archive for v$VERSION"
        say "  unpacked $archive"
    fi

    tag_ver="$(awk -F'"' '/^version[[:space:]]*=/{print $2; exit}' "$TMP/src/Cargo.toml" 2>/dev/null || true)"
    [ "$tag_ver" = "$VERSION" ] ||
        die "tag v$VERSION declares version '$tag_ver'; refusing to build the wrong thing"

    check_rust_version "$TMP/src"

    say "  building with cargo (1-3 minutes; log: $BUILD_LOG)"
    started=$(date +%s)
    if ! ( cd "$TMP/src" && CARGO_TARGET_DIR="$TMP/src/target" \
               cargo build --release --locked ) >>"$BUILD_LOG" 2>&1; then
        say ""
        say "the build failed. full log: $BUILD_LOG"
        tail -n 25 "$BUILD_LOG" 2>/dev/null | sed 's/^/    /'
        die "cargo build failed for v$VERSION"
    fi
    say "  built in $(( $(date +%s) - started ))s"
    say "  note: a source build is not signature-checked (HTTPS + tag only)"

    PKG="$TMP/pkg"
    mkdir -p "$PKG"
    # v0.1.1（改名前）编出来的是 release/zmusic，两个名字都认
    bin=""
    for cand in "$TMP/src/target/release/zmuse" "$TMP/src/target/release/zmusic"; do
        if [ -x "$cand" ]; then bin="$cand"; break; fi
    done
    [ -n "$bin" ] || die "the build produced no binary in target/release (see $BUILD_LOG)"
    install -m 755 "$bin" "$PKG/zmuse"
    if [ -d "$TMP/src/sidecar" ]; then
        mkdir -p "$PKG/sidecar"
        cp -a "$TMP/src/sidecar/." "$PKG/sidecar/"
        rm -rf "$PKG/sidecar/node_modules/.cache"
    fi
    for f in README.md LICENSE; do
        if [ -f "$TMP/src/$f" ]; then install -m 644 "$TMP/src/$f" "$PKG/$f"; fi
    done
    if [ -f "$TMP/src/docs/CONFIG.md" ]; then
        install -m 644 "$TMP/src/docs/CONFIG.md" "$PKG/CONFIG.md"
    fi
    if [ -f "$TMP/src/packaging/zmuse-meting-user.service" ]; then
        install -m 644 "$TMP/src/packaging/zmuse-meting-user.service" "$PKG/zmuse-meting.service"
    else
        # 老 tag（prequel：unit 还没进仓库）也装上同样的 unit，别让侧车没服务文件
        write_user_unit "$PKG/zmuse-meting.service"
    fi
    SRC="$PKG/zmuse"
    rm -rf "$TMP/src/target"
}

# ---------------------------------------------------------------- uninstall
if [ "$DO_UNINSTALL" -eq 1 ]; then
    if [ -e "$BIN" ]; then
        rm -f "$BIN"
        say "removed $BIN"
    else
        say "nothing to remove at $BIN"
    fi
    if [ -f "$HOME/.config/systemd/user/zmuse-meting.service" ]; then
        if command -v systemctl >/dev/null 2>&1; then
            systemctl --user disable --now zmuse-meting.service >/dev/null 2>&1 || true
        fi
        rm -f "$HOME/.config/systemd/user/zmuse-meting.service"
        command -v systemctl >/dev/null 2>&1 && systemctl --user daemon-reload >/dev/null 2>&1 || true
        say "removed the Meting sidecar unit"
    fi
    rm -rf "$HOME/.local/share/zmuse" "$HOME/.local/share/doc/zmuse"
    NCM_UNIT_U="$HOME/.config/systemd/user/ncm-api.service"
    NCM_DIR_U="$HOME/.local/share/ncm-api"
    if [ -f "$NCM_DIR_U/.zmuse-installed" ]; then
        if command -v systemctl >/dev/null 2>&1; then
            systemctl --user disable --now ncm-api.service >/dev/null 2>&1 || true
        fi
        rm -f "$NCM_UNIT_U"
        rm -rf "$NCM_DIR_U"
        command -v systemctl >/dev/null 2>&1 && systemctl --user daemon-reload >/dev/null 2>&1 || true
        say "removed the local NetEase API service"
    elif [ -d "$NCM_DIR_U" ]; then
        say "left $NCM_DIR_U alone (it was not installed by this script)"
    fi
    say "your settings were left alone: ~/.config/zmuse and ~/.cache/zmuse"
    exit 0
fi

# ---------------------------------------------------------------- platform
[ "$(uname -s)" = "Linux" ] || die "this installer handles Linux only right now (found $(uname -s))"
case "$(uname -m)" in
    x86_64|amd64)   ARCH=x86_64 ;;
    aarch64|arm64)  ARCH=aarch64 ;;
    *)              die "unsupported architecture: $(uname -m)" ;;
esac

command -v curl >/dev/null 2>&1 || die "curl is required"
command -v tar  >/dev/null 2>&1 || die "tar is required"

# ---------------------------------------------------------------- version
if [ -z "$VERSION" ]; then
    VERSION="$(curl -fsSL "$BASE/latest" 2>/dev/null | tr -d ' \t\r\n')" ||
        die "could not read $BASE/latest"
    [ -n "$VERSION" ] || die "could not read $BASE/latest (empty)"
fi
case "$VERSION" in
    *[!0-9A-Za-z._-]*) die "odd version string: $VERSION" ;;
esac

NAME="zmuse-$VERSION-linux-$ARCH"
TARBALL="$NAME.tar.gz"
VDIR="$BASE/v$VERSION"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/zmuse-install.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT INT TERM

say "zmuse $VERSION ($ARCH) -> $PREFIX"

if [ "$DO_SOURCE" -eq 1 ]; then
    # 源码模式：拉 tag、本地编译；没有 tarball 可验签，安全边界是 HTTPS + tag
    build_from_source
else
    PKG="$TMP/$NAME"
    # ---------------------------------------------------------------- checksum list
    # 先拿 SHA256SUMS 和它的签名：清单是被签名的那一份，tarball 的哈希只从清单里取。
    curl -fsSL -o "$TMP/SHA256SUMS" "$VDIR/SHA256SUMS" ||
        die "could not fetch $VDIR/SHA256SUMS"
    WANT="$(awk -v f="$TARBALL" '$2 == f || $2 == "*"f {print $1; exit}' "$TMP/SHA256SUMS")"
    [ -n "$WANT" ] || die "$TARBALL is not listed in SHA256SUMS (unknown version $VERSION for $ARCH?)"

    # ---------------------------------------------------------------- signature
    if [ "$DO_GPG" -eq 1 ]; then
        if ! command -v gpg >/dev/null 2>&1; then
            die "gpg is required to verify the signature; install gnupg, or pass --insecure-sha-only"
        fi
        curl -fsSL -o "$TMP/SHA256SUMS.sig" "$VDIR/SHA256SUMS.sig" ||
            die "signature SHA256SUMS.sig is missing; refusing to install unverified (use --insecure-sha-only)"
        curl -fsSL -o "$TMP/repo.pub" "$KEY_URL" ||
            die "could not fetch the repository key from $KEY_URL"
        # Throwaway keyring: the user's ~/.gnupg is never modified.
        export GNUPGHOME="$TMP/gnupg"
        mkdir -p "$GNUPGHOME"
        chmod 700 "$GNUPGHOME"
        gpg --batch --quiet --import "$TMP/repo.pub" >/dev/null 2>&1 ||
            die "could not import the repository key"
        FPS="$(gpg --batch --with-colons --fingerprint 2>/dev/null | awk -F: '/^fpr:/{print $10; exit}')"
        [ "$FPS" = "$EXPECTED_FP" ] ||
            die "repository key fingerprint mismatch: got $FPS, expected $EXPECTED_FP"
        gpg --batch --quiet --verify "$TMP/SHA256SUMS.sig" "$TMP/SHA256SUMS" >/dev/null 2>&1 ||
            die "signature verification failed for SHA256SUMS"
        say "  signature ok ($EXPECTED_FP)"
    else
        say "  WARNING: signature check skipped (--insecure-sha-only)"
    fi

    # ---------------------------------------------------------------- download
    say "  downloading $TARBALL"
    curl -fsSL -o "$TMP/$TARBALL" "$VDIR/$TARBALL" || die "download failed: $VDIR/$TARBALL"

    if command -v sha256sum >/dev/null 2>&1; then
        GOT="$(sha256sum "$TMP/$TARBALL" | awk '{print $1}')"
    elif command -v shasum >/dev/null 2>&1; then
        GOT="$(shasum -a 256 "$TMP/$TARBALL" | awk '{print $1}')"
    else
        die "no sha256 tool (need sha256sum or shasum)"
    fi
    [ "$GOT" = "$WANT" ] || die "sha256 mismatch: expected $WANT, got $GOT"
    say "  sha256 ok"

    # ---------------------------------------------------------------- install
    tar -xzf "$TMP/$TARBALL" -C "$TMP" || die "could not unpack $TARBALL"
    SRC="$PKG/zmuse"
    [ -f "$SRC" ] || SRC="$(find "$PKG" -maxdepth 2 -name zmuse -type f 2>/dev/null | head -1)"
    [ -n "${SRC:-}" ] && [ -f "$SRC" ] || die "no zmuse binary inside $TARBALL"
fi

mkdir -p "$PREFIX"
if [ -e "$BIN" ]; then
    OLD="$("$BIN" --version 2>/dev/null || echo unknown)"
    say "  replacing $OLD"
fi
cp -f "$SRC" "$BIN.new"
chmod 755 "$BIN.new"
mv -f "$BIN.new" "$BIN"
say "  installed $BIN"

# ---------------------------------------------------------------- check
GOT_VER="$("$BIN" --version 2>/dev/null || true)"
case "$GOT_VER" in
    *"$VERSION"*) say "  $GOT_VER" ;;
    "")           die "the binary does not run on this machine (glibc/musl mismatch?)" ;;
    *)            die "unexpected version: $GOT_VER" ;;
esac

# ---------------------------------------------------------------- sidecar + docs
SIDE_DST="$HOME/.local/share/zmuse/sidecar"
UNIT_DST="$HOME/.config/systemd/user/zmuse-meting.service"
if [ "$DO_SIDECAR" -eq 1 ] && [ -f "$PKG/sidecar/meting.js" ]; then
    mkdir -p "$SIDE_DST"
    cp -a "$PKG/sidecar/." "$SIDE_DST/"
    say "  sidecar -> $SIDE_DST"
    if [ -f "$PKG/zmuse-meting.service" ]; then
        mkdir -p "$(dirname "$UNIT_DST")"
        install -m 644 "$PKG/zmuse-meting.service" "$UNIT_DST"
        say "  unit    -> $UNIT_DST"
    fi
    if command -v systemctl >/dev/null 2>&1; then
        systemctl --user daemon-reload >/dev/null 2>&1 || true
    fi
elif [ "$DO_SIDECAR" -eq 1 ]; then
    say ""
    say "note: this tarball has no Meting sidecar, so QQ / Kugou / Baidu / Kuwo will"
    say "      report \"sidecar not running\". Everything else works."
fi

# ---------------------------------------------------------------- NetEase API
if [ "$DO_NCM" -eq 1 ]; then
    install_ncm
fi

# 文档和许可证
DOC_DST="$HOME/.local/share/doc/zmuse"
for f in README.md CONFIG.md LICENSE; do
    if [ -f "$PKG/$f" ]; then
        mkdir -p "$DOC_DST"
        install -m 644 "$PKG/$f" "$DOC_DST/$f"
    fi
done

# ---------------------------------------------------------------- runtime deps
if ! command -v mpv >/dev/null 2>&1; then
    say ""
    say "mpv is required for playback. Install it with:"
    if [ -r /etc/os-release ]; then
        . /etc/os-release
        case "${ID:-}${ID_LIKE:-}" in
            *arch*)                 say "  sudo pacman -S mpv" ;;
            *debian*|*ubuntu*|*linuxmint*) say "  sudo apt install mpv" ;;
            *fedora*)               say "  sudo dnf install mpv" ;;
            *opensuse*|*suse*)      say "  sudo zypper install mpv" ;;
            *alpine*)               say "  sudo apk add mpv" ;;
            *void*)                 say "  sudo xbps-install mpv" ;;
            *)                      say "  install mpv with your package manager" ;;
        esac
    else
        say "  install mpv with your package manager"
    fi
fi
if ! command -v node >/dev/null 2>&1; then
    say ""
    say "node >= 22 is optional: it powers the local NetEase API and the Meting"
    say "sidecar (QQ / Kugou / Baidu / Kuwo). zmuse starts without it, with only"
    say "the sources that need no sidecar."
fi

case ":$PATH:" in
    *":$PREFIX:"*) ;;
    *)
        say ""
        say "$PREFIX is not in your PATH. Add it:"
        say "  echo 'export PATH=\"$PREFIX:\$PATH\"' >> ~/.profile"
        say "and open a new shell (or export it for this one)."
        ;;
esac

if command -v zmuse >/dev/null 2>&1; then
    SYS="$(command -v zmuse)"
    if [ "$SYS" != "$BIN" ]; then
        say ""
        say "note: your PATH resolves zmuse to $SYS first, so that one runs."
    fi
fi

if [ "$DO_SIDECAR" -eq 1 ] && [ -f "$UNIT_DST" ] && command -v node >/dev/null 2>&1; then
    if ! systemctl --user is-enabled zmuse-meting.service >/dev/null 2>&1; then
        say ""
        say "Enable the Meting sidecar (QQ / Kugou / Baidu / Kuwo sources):"
        say "  systemctl --user enable --now zmuse-meting.service"
    fi
fi

say ""
say "done. run: zmuse"

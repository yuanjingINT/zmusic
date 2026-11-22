#!/bin/bash
# 构建发布用的 Linux 静态包（musl / static-pie），产出：
#   <out>/zmuse-<ver>-linux-<arch>.tar.gz
#   内含 zmuse-<ver>-linux-<arch>/{zmuse,sidecar/,zmuse-meting.service,README.md,CONFIG.md,LICENSE}
#
# 静态链接的用处：一份文件在任何发行版都能跑（Alpine/musl 与 Debian/glibc 实测同样可执行）。
# 构建在容器里做，避免本机 glibc 工具链把动态依赖带进来。
#
# 用法：
#   packaging/build-static.sh                  # 版本取 Cargo.toml，arch=x86_64
#   packaging/build-static.sh --arch aarch64   # 需要 qemu binfmt（未验证）
#   packaging/build-static.sh --out /tmp/out
#
# 环境变量：
#   ZMUSE_BUILD_IMAGE   默认 docker.m.daocloud.io/library/rust:alpine
#                       （docker.io 直连不通时用镜像；可用 IMAGE=rust:alpine 覆盖）
#   ZMUSE_APK_MIRROR    默认 https://mirrors.tuna.tsinghua.edu.cn/alpine
#   ZMUSE_BUILD_CACHE   默认 ~/.cache/zmuse-static（放 CARGO_TARGET_DIR，别污染仓库的 target/）
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT=$(pwd)

VERSION=""
ARCH=x86_64
OUT="$ROOT/dist"
IMAGE="${ZMUSE_BUILD_IMAGE:-docker.m.daocloud.io/library/rust:alpine}"
APK_MIRROR="${ZMUSE_APK_MIRROR:-https://mirrors.tuna.tsinghua.edu.cn/alpine}"
CACHE="${ZMUSE_BUILD_CACHE:-$HOME/.cache/zmuse-static}"

while [ $# -gt 0 ]; do
    case "$1" in
        --version) VERSION="$2"; shift 2 ;;
        --arch)    ARCH="$2"; shift 2 ;;
        --out)     OUT="$2"; shift 2 ;;
        --image)   IMAGE="$2"; shift 2 ;;
        -h|--help) sed -n '2,22p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
done

if [ -z "$VERSION" ]; then
    VERSION=$(awk -F'"' '/^version[[:space:]]*=/{print $2; exit}' Cargo.toml)
fi
[ -n "$VERSION" ] || { echo "cannot read the version from Cargo.toml" >&2; exit 1; }

case "$ARCH" in
    x86_64)  TRIPLE=x86_64-unknown-linux-musl;  TARGET_ENV=CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_RUSTFLAGS ;;
    aarch64) TRIPLE=aarch64-unknown-linux-musl; TARGET_ENV=CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_RUSTFLAGS ;;
    *) echo "unsupported arch: $ARCH" >&2; exit 2 ;;
esac

NAME="zmuse-$VERSION-linux-$ARCH"
mkdir -p "$OUT" "$CACHE"

echo "==> building $NAME (static, $TRIPLE)"
echo "    image: $IMAGE"
# 关键三点：
#   1) --target 显式给，cargo 才会把 proc-macro/build script 当 host 单元编译
#   2) crt-static 只通过 target 专属 RUSTFLAGS 传（全局传会打到 proc-macro 上，报
#      "cannot produce proc-macro ... does not support these crate types"）
#   3) OPENSSL_STATIC=1：native-tls 走 openssl-sys，静态包必须链静态 openssl
podman run --rm \
    --arch "$ARCH" \
    -v "$ROOT":/src:z \
    -v "$CACHE":/out:z \
    -v "${CARGO_HOME:-$HOME/.cargo}/registry":/usr/local/cargo/registry:z \
    -w /src \
    -e CARGO_TARGET_DIR=/out \
    -e OPENSSL_STATIC=1 \
    -e "$TARGET_ENV=-C target-feature=+crt-static" \
    "$IMAGE" sh -c '
        set -e
        # docker.io 的 Alpine CDN 在部分网络下不可达，换镜像源（可用 ZMUSE_APK_MIRROR 覆盖）
        REL=$(cat /etc/alpine-release | cut -d. -f1,2)
        echo "'"$APK_MIRROR"'/v$REL/main" > /etc/apk/repositories
        echo "'"$APK_MIRROR"'/v$REL/community" >> /etc/apk/repositories
        apk add --no-cache musl-dev openssl-dev openssl-libs-static pkgconf perl make gcc >/dev/null
        cargo build --release --locked --target '"$TRIPLE"'
    '

BIN="$CACHE/$TRIPLE/release/zmuse"
[ -f "$BIN" ] || { echo "build produced no binary at $BIN" >&2; exit 1; }

# 链接性质要自己验一次：动态链接的 musl 包换到 glibc 发行版就跑不了
LINK=$(file -b "$BIN")
case "$LINK" in
    *"statically linked"*|*"static-pie linked"*) ;;
    *) echo "refusing to package a non-static binary: $LINK" >&2; exit 1 ;;
esac

echo "==> assembling $NAME"
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
PKG="$STAGE/$NAME"
mkdir -p "$PKG/sidecar"
install -m 755 "$BIN" "$PKG/zmuse"
# 侧车（Meting：腾讯/酷狗/百度/酷我）：node_modules 是仓库里 vendored 的，必须一起带上
cp -a sidecar/. "$PKG/sidecar/"
rm -rf "$PKG/sidecar/node_modules/.cache"
install -m 644 README.md LICENSE "$PKG/"
install -m 644 docs/CONFIG.md "$PKG/CONFIG.md"
# 用户级 unit（静态安装装在家目录，路径和 pacman 包里的 /usr/lib 版本不同）。
# 内容放在 packaging/zmuse-meting-user.service：install.sh 的 --from-source 也用这一份。
install -m 644 packaging/zmuse-meting-user.service "$PKG/zmuse-meting.service"

# 可复现：同样的 commit 两次构建应当逐字节一致。
#   --sort=name        文件顺序固定
#   --mtime            tar 里记录的时间取 commit 时间，而不是本机打包时刻
#   gzip -n            不把 gzip 头里的时间和文件名写进去
MTIME="$(git -C "$ROOT" log -1 --format=%cI 2>/dev/null || echo 2026-01-01T00:00:00Z)"
( cd "$STAGE" && tar --sort=name --mtime="$MTIME" --owner=0 --group=0 --numeric-owner \
      --use-compress-program='gzip -n' -cf "$OUT/$NAME.tar.gz" "$NAME" )

echo "==> $OUT/$NAME.tar.gz"
ls -l "$OUT/$NAME.tar.gz" | awk '{print "    size:", $5, "bytes"}'
echo "    sha256: $(sha256sum "$OUT/$NAME.tar.gz" | awk '{print $1}')"
echo "    binary: $(du -h "$BIN" | awk '{print $1}') ($LINK)"

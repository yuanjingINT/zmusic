# Maintainer: yuanjing <yuanjing@yuanjing.art>
pkgname=zmuse
pkgver=0.1.1
pkgrel=2
# 从 zmusic 改名过来：老用户 -Syu 时自动替换旧包（replaces），并申明两者不能共存
replaces=('zmusic')
conflicts=('zmusic')
pkgdesc="Terminal NetEase Cloud Music player (Rust / ratatui TUI)"
arch=('x86_64' 'aarch64')
install=zmuse.install
# 仓库路径暂时仍是 personal/zmusic：改项目路径是服务端操作，等和 GitHub 改名一起做；
# 改完这里同步换成 personal/zmuse，并按新路径重钉 _commit 与 sha256sums。
url="https://git.tuxai.org/personal/zmusic"
license=('GPL-3.0-only')
depends=('gcc-libs' 'openssl')
makedepends=('cargo' 'rust')
optdepends=('mpv: audio playback'
            'nodejs: NetEase API service and Meting sidecar (Node >= 22)'
            'yt-dlp: YouTube source search and playback (needs a proxy)')

# 源码固定到 GitLab 上**已推送**的 commit 归档（本地未推的 commit 会 404）。
# 归档顶层目录名是 <项目路径>-<commit>，而项目路径还是 zmusic，所以这里单独用
# _srcdir；不能用 $pkgname（包名已经是 zmuse），否则 build() 会 cd 到不存在的目录。
# 发版时把 _commit 换成要发的那一版，并重算 sha256sums：
#   curl -sSfL "$url/-/archive/$_commit/$_srcdir.tar.gz" | sha256sum
# 之后公开了 tag，也可以换成 tag 归档：$_commit 的位置写 v$pkgver。
_commit=56a7c0f8c21cb4a726ce5c0937a549ef3bab41fb
_srcdir="zmusic-$_commit"
source=("$_srcdir.tar.gz::$url/-/archive/$_commit/$_srcdir.tar.gz")
sha256sums=('ce00ba37fe6a1795993212946e92f5456058387114a29b1ffe5ec76e456c48b0')

build() {
  cd "$srcdir/$_srcdir"
  export CARGO_HOME="$srcdir/cargo-home"
  # 构建可复现：别把家目录/构建目录的绝对路径嵌进二进制（panic 的 Location::file 会带）
  export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$srcdir=/build/zmuse --remap-path-prefix=$HOME=/build/home"
  cargo build --release --locked
}

package() {
  cd "$srcdir/$_srcdir"
  install -Dm755 target/release/zmuse "$pkgdir/usr/bin/zmuse"
  install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
  install -Dm644 README.md "$pkgdir/usr/share/doc/$pkgname/README.md"
  install -Dm644 docs/CONFIG.md "$pkgdir/usr/share/doc/$pkgname/CONFIG.md"

  # Meting 侧车：node_modules 是在仓库里有意 vendored 的（@meting/core，MIT），
  # 为了 makepkg 离线构建；不带它 meting:* 那组源装完就是「侧车未启动」。
  install -d "$pkgdir/usr/lib/$pkgname/sidecar"
  cp -a sidecar/. "$pkgdir/usr/lib/$pkgname/sidecar/"
  find "$pkgdir/usr/lib/$pkgname/sidecar" -name '*.js' -exec chmod 644 {} +
  chmod 755 "$pkgdir/usr/lib/$pkgname/sidecar"

  # systemd user unit（可选启用：systemctl --user enable --now zmuse-meting）
  install -Dm644 packaging/zmuse-meting.service \
    "$pkgdir/usr/lib/systemd/user/zmuse-meting.service"
}

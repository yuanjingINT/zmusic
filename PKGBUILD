# Maintainer: yuanjing <yuanjing@yuanjing.art>
pkgname=zmusic
pkgver=0.1.1
pkgrel=2
pkgdesc="Terminal NetEase Cloud Music player (Rust / ratatui TUI)"
arch=('x86_64' 'aarch64')
url="https://git.tuxai.org/personal/zmusic"
license=('GPL-3.0-only')
depends=('gcc-libs' 'openssl')
makedepends=('cargo' 'rust')
optdepends=('mpv: audio playback'
            'nodejs: NetEase API service and Meting sidecar (Node >= 22)'
            'yt-dlp: YouTube source search and playback (needs a proxy)')

# 源码固定到 GitLab 上**已推送**的 commit 归档（本地未推的 commit 会 404）。
# 发版时把 _commit 换成要发的那一版，并重算 sha256sums：
#   curl -sSfL "$url/-/archive/$_commit/$pkgname-$_commit.tar.gz" | sha256sum
# 之后公开了 tag，也可以换成 tag 归档：$_commit 的位置写 v$pkgver。
_commit=94c872f1dde4c2cdd4144c3ffedcac0d666622d5
source=("$pkgname-$_commit.tar.gz::$url/-/archive/$_commit/$pkgname-$_commit.tar.gz")
sha256sums=('538ca91e5e17793ec5ed048a53b93b9d6bf8d521abc9a826f0f4ae3c72e98958')

build() {
  cd "$srcdir/$pkgname-$_commit"
  export CARGO_HOME="$srcdir/cargo-home"
  # 构建可复现：别把家目录/构建目录的绝对路径嵌进二进制（panic 的 Location::file 会带）
  export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$srcdir=/build/zmusic --remap-path-prefix=$HOME=/build/home"
  cargo build --release --locked
}

package() {
  cd "$srcdir/$pkgname-$_commit"
  install -Dm755 target/release/zmusic "$pkgdir/usr/bin/zmusic"
  install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
  install -Dm644 README.md "$pkgdir/usr/share/doc/$pkgname/README.md"
  install -Dm644 docs/CONFIG.md "$pkgdir/usr/share/doc/$pkgname/CONFIG.md"

  # Meting 侧车：node_modules 是在仓库里有意 vendored 的（@meting/core，MIT），
  # 为了 makepkg 离线构建；不带它 meting:* 那组源装完就是「侧车未启动」。
  install -d "$pkgdir/usr/lib/$pkgname/sidecar"
  cp -a sidecar/. "$pkgdir/usr/lib/$pkgname/sidecar/"
  find "$pkgdir/usr/lib/$pkgname/sidecar" -name '*.js' -exec chmod 644 {} +
  chmod 755 "$pkgdir/usr/lib/$pkgname/sidecar"

  # systemd user unit（可选启用：systemctl --user enable --now meting-sidecar）
  install -Dm644 packaging/meting-sidecar.service \
    "$pkgdir/usr/lib/systemd/user/meting-sidecar.service"
}

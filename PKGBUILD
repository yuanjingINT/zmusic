# Maintainer: yuanjing <yuanjing@yuanjing.art>
pkgname=zmusic
pkgver=0.1.1beta1
pkgrel=1
pkgdesc="网易云音乐终端播放器（Rust / ratatui TUI，beta）"
arch=('x86_64' 'aarch64')
url="https://git.tuxai.org/personal/zmusic"
license=('GPL-3.0-only')
depends=('gcc-libs' 'openssl')
makedepends=('cargo' 'rust')
optdepends=('mpv: 音频播放'
            'nodejs: 网易云 API 服务（setup.sh 需要 Node >= 22）')

# 源码固定到 GitLab 上**已推送**的 commit 归档（本地未推的 commit 会 404）。
# 发版时把 _commit 换成要发的那一版，并重算 sha256sums：
#   curl -sSfL "$url/-/archive/$_commit/$pkgname-$_commit.tar.gz" | sha256sum
# 之后公开了 tag，也可以换成 tag 归档：$_commit 的位置写 v$pkgver。
_commit=4dc69d7985c51507fc3dfa6ccf499b752d0d65aa
source=("$pkgname-$_commit.tar.gz::$url/-/archive/$_commit/$pkgname-$_commit.tar.gz")
sha256sums=('8a6adad4a63c49efcd6acb44cc65e394dd31586765ba84dac80eb3194cade9c8')

build() {
  cd "$srcdir/$pkgname-$_commit"
  export CARGO_HOME="$srcdir/cargo-home"
  cargo build --release --locked
}

package() {
  cd "$srcdir/$pkgname-$_commit"
  install -Dm755 target/release/zmusic "$pkgdir/usr/bin/zmusic"
  install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
  install -Dm644 README.md "$pkgdir/usr/share/doc/$pkgname/README.md"
}

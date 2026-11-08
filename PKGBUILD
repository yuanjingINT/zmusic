# Maintainer: yuanjing <yuanjing@yuanjing.art>
pkgname=zmusic
pkgver=0.1.1beta2
pkgrel=1
pkgdesc="网易云音乐终端播放器（Rust / ratatui TUI，beta）"
arch=('x86_64' 'aarch64')
url="https://git.tuxai.org/personal/zmusic"
license=('GPL-3.0-only')
depends=('gcc-libs' 'openssl')
makedepends=('cargo' 'rust')
optdepends=('mpv: 音频播放'
            'nodejs: 网易云 API 服务（setup.sh 需要 Node >= 22）'
            'yt-dlp: YouTube 源搜索与播放（需要代理）')

# 源码固定到 GitLab 上**已推送**的 commit 归档（本地未推的 commit 会 404）。
# 发版时把 _commit 换成要发的那一版，并重算 sha256sums：
#   curl -sSfL "$url/-/archive/$_commit/$pkgname-$_commit.tar.gz" | sha256sum
# 之后公开了 tag，也可以换成 tag 归档：$_commit 的位置写 v$pkgver。
_commit=ea35531c1c3d7158f0a55bf8129e4edf4c7512fa
source=("$pkgname-$_commit.tar.gz::$url/-/archive/$_commit/$pkgname-$_commit.tar.gz")
sha256sums=('5b709b8c5afacb4b98823c6be70184da30c99ee15498ce0741edea6b423556da')

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

# Maintainer: crusty-dlp contributors
pkgname=crusty-dlp
pkgver=0.3.0
pkgrel=1
pkgdesc='Small terminal UI for safe yt-dlp download queues'
arch=('x86_64' 'aarch64')
url='https://github.com/y-bgmpst/crusty-dlp'
license=('MIT')
depends=('yt-dlp')
optdepends=(
  'ffmpeg: audio extraction, conversion, and format merging'
  'python-curl_cffi: browser request impersonation and BoyfriendTV support'
  'deno: JavaScript challenge solving for full YouTube support'
)
makedepends=('cargo')
source=("$pkgname-$pkgver.tar.gz::$url/archive/v$pkgver.tar.gz")
sha256sums=('SKIP') # Replace with the release archive checksum before publishing.

build() {
  cd "$pkgname-$pkgver"
  cargo build --release --locked
}

check() {
  cd "$pkgname-$pkgver"
  cargo test --locked
}

package() {
  cd "$pkgname-$pkgver"
  install -Dm755 "target/release/$pkgname" "$pkgdir/usr/bin/$pkgname"
  install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
  install -Dm644 README.md "$pkgdir/usr/share/doc/$pkgname/README.md"
}

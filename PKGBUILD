# Maintainer: crusty-dlp contributors
pkgname=crusty-dlp
pkgver=0.6.0
pkgrel=2
pkgdesc='Safe terminal and desktop interfaces for yt-dlp download queues'
arch=('x86_64' 'aarch64')
url='https://github.com/y-bgmpst/crusty-dlp'
license=('MIT')
install="$pkgname.install"
depends=('yt-dlp')
optdepends=(
  'ffmpeg: audio extraction, conversion, and format merging'
  'python-curl_cffi: browser request impersonation and BoyfriendTV support'
  'deno: JavaScript challenge solving for full YouTube support'
  'aria2: multi-connection downloads for direct HTTP files'
)
makedepends=('cargo')
source=("$pkgname-$pkgver.tar.gz::$url/archive/v$pkgver.tar.gz")
sha256sums=('SKIP') # Replace with the release archive checksum before publishing.

build() {
  cd "$pkgname-$pkgver"
  cargo build --release --locked --bins
}

check() {
  cd "$pkgname-$pkgver"
  cargo test --locked
}

package() {
  cd "$pkgname-$pkgver"
  install -Dm755 "target/release/$pkgname" "$pkgdir/usr/bin/$pkgname"
  install -Dm755 "target/release/$pkgname-gui" "$pkgdir/usr/bin/$pkgname-gui"
  install -Dm644 assets/crusty-dlp.desktop \
    "$pkgdir/usr/share/applications/crusty-dlp.desktop"
  install -Dm644 assets/crusty-dlp.svg \
    "$pkgdir/usr/share/icons/hicolor/scalable/apps/crusty-dlp.svg"
  for size in 16 24 32 48 64 128 256 512; do
    install -Dm644 "assets/icons/hicolor/${size}x${size}/apps/crusty-dlp.png" \
      "$pkgdir/usr/share/icons/hicolor/${size}x${size}/apps/crusty-dlp.png"
  done
  install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
  install -Dm644 README.md "$pkgdir/usr/share/doc/$pkgname/README.md"
  install -Dm644 plugins/yt_dlp_plugins/extractor/boyfriendtv.py \
    "$pkgdir/usr/share/$pkgname/plugins/yt_dlp_plugins/extractor/boyfriendtv.py"
  install -Dm644 plugins/yt_dlp_plugins/extractor/pmvhaven.py \
    "$pkgdir/usr/share/$pkgname/plugins/yt_dlp_plugins/extractor/pmvhaven.py"
  install -Dm644 plugins/yt_dlp_plugins/extractor/spankbang.py \
    "$pkgdir/usr/share/$pkgname/plugins/yt_dlp_plugins/extractor/spankbang.py"
}

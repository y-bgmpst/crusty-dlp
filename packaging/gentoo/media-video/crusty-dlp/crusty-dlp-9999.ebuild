EAPI=8

inherit cargo desktop git-r3 xdg

DESCRIPTION="Safe terminal and desktop interfaces for yt-dlp"
HOMEPAGE="https://github.com/y-bgmpst/crusty-dlp"
EGIT_REPO_URI="https://github.com/y-bgmpst/crusty-dlp.git"

LICENSE="GPL-3"
SLOT="0"
KEYWORDS=""
IUSE=""

RDEPEND="
	media-video/yt-dlp
"
DEPEND="${RDEPEND}"
BDEPEND="
	virtual/rust
"

src_unpack() {
	git-r3_src_unpack
	cargo_live_src_unpack
}

src_install() {
	cargo_src_install --path .
	dobin target/release/crusty-dlp-gui
	domenu assets/crusty-dlp.desktop
	doicon -s scalable assets/crusty-dlp.svg
	for size in 16 24 32 48 64 128 256 512; do
		newicon -s ${size} assets/icons/hicolor/${size}x${size}/apps/crusty-dlp.png crusty-dlp.png
	done
	insinto /usr/share/${PN}/plugins/yt_dlp_plugins/extractor
	doins plugins/yt_dlp_plugins/extractor/boyfriendtv.py
	doins plugins/yt_dlp_plugins/extractor/ooxxx.py
	doins plugins/yt_dlp_plugins/extractor/pmvhaven.py
	doins plugins/yt_dlp_plugins/extractor/spankbang.py
	dodoc README.md COMPATIBILITY.md
}

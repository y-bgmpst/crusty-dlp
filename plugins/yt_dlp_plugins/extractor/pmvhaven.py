"""yt-dlp extractor for public PMVHaven video pages."""

from yt_dlp.extractor.common import InfoExtractor


class PMVHavenIE(InfoExtractor):
    IE_NAME = "pmvhaven"
    _VALID_URL = r"https?://(?:www\.)?pmvhaven\.com/video/(?P<id>[^/?#]+)"

    def _real_extract(self, url):
        display_id = self._match_id(url)
        webpage = self._download_webpage(url, display_id, impersonate=True)
        info = self._search_json_ld(
            webpage, display_id, expected_type="VideoObject"
        )
        media_url = info.pop("url")

        if media_url.endswith(".m3u8"):
            formats = self._extract_m3u8_formats(
                media_url,
                display_id,
                "mp4",
                headers={"Referer": url},
            )
        else:
            formats = [{"url": media_url, "http_headers": {"Referer": url}}]

        return {
            **info,
            "id": display_id.rsplit("_", 1)[-1],
            "display_id": display_id,
            "formats": formats,
            "age_limit": 18,
        }

"""yt-dlp extractors for public PMVHaven pages."""

import json
import re

from yt_dlp.extractor.common import InfoExtractor
from yt_dlp.utils import ExtractorError, orderedSet, urljoin


class PMVHavenIE(InfoExtractor):
    IE_NAME = "pmvhaven"
    _VALID_URL = r"https?://(?:www\.)?pmvhaven\.com/videos?/(?P<id>[^/?#]+)"

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


class PMVHavenPlaylistIE(InfoExtractor):
    IE_NAME = "pmvhaven:playlist"
    _VALID_URL = r"https?://(?:www\.)?pmvhaven\.com/playlists/(?P<id>[^/?#]+)"

    def _real_extract(self, url):
        playlist_id = self._match_id(url)
        webpage = self._download_webpage(url, playlist_id, impersonate=True)

        entries = []
        json_ld_matches = re.findall(
            r'<script type="application/ld\+json">([^<]+)</script>',
            webpage,
        )
        for block in json_ld_matches:
            try:
                data = json.loads(block)
            except json.JSONDecodeError:
                continue
            if data.get("@type") != "ItemList":
                continue
            for item in data.get("itemListElement", []):
                video = item.get("item") or {}
                embed_url = video.get("embedUrl")
                if embed_url and re.match(r"https?://(?:www\.)?pmvhaven\.com/videos?/", embed_url):
                    entries.append(
                        self.url_result(
                            embed_url,
                            ie=PMVHavenIE.ie_key(),
                        )
                    )

        for path in orderedSet(
            re.findall(r'href=["\'](/videos?/[^"\']+)["\']', webpage)
        ):
            entries.append(
                self.url_result(
                    urljoin(url, path),
                    ie=PMVHavenIE.ie_key(),
                )
            )

        entries = orderedSet(entries)

        if not entries:
            raise ExtractorError("Could not find any playlist entries", expected=True)

        title = self._search_regex(
            (
                r'<meta[^>]+property=["\']og:title["\'][^>]+content=["\']([^"\']+)',
                r"<title>([^<]+)</title>",
            ),
            webpage,
            "playlist title",
            default=f"PMVHaven playlist {playlist_id}",
        )
        title = re.sub(r"\s*-\s*PMVHaven\s*$", "", title).strip()

        return self.playlist_result(entries, playlist_id, title)

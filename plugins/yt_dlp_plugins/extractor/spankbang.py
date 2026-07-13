"""yt-dlp extractor for public SpankBang video pages.

The site publishes its media variants in a ``stream_data`` JavaScript object.
Browser cookies may still be required to retrieve pages protected by Cloudflare.
"""

import re

from yt_dlp.extractor.common import InfoExtractor
from yt_dlp.utils import (
    ExtractorError,
    determine_ext,
    int_or_none,
    js_to_json,
    orderedSet,
    url_or_none,
    urljoin,
)


SPANKBANG_DOMAIN = "spankbang.com"
SPANKBANG_BASE_URL = f"https://{SPANKBANG_DOMAIN}"


class SpankBangIE(InfoExtractor):
    IE_NAME = "spankbang:crusty"
    _VALID_URL = (
        rf"https?://(?:www\.)?{re.escape(SPANKBANG_DOMAIN)}/"
        r"(?:(?P<id>[a-z0-9]+)/(?:(?:video|embed)/)(?P<display_id>[^/?#]+)?"
        r"|[a-z0-9]+-(?P<playlist_id>[a-z0-9]+)/playlist/(?P<playlist_display_id>[^/?#]+))"
    )
    _KNOWN_QUALITIES = ("4k", "1080p", "720p", "480p", "320p", "240p")

    def _real_extract(self, url):
        match = self._match_valid_url(url)
        video_id = match.group("id") or match.group("playlist_id")
        display_id = match.group("display_id") or match.group("playlist_display_id")
        canonical_url = f"{SPANKBANG_BASE_URL}/{video_id}/video/{display_id or video_id}"
        webpage = self._download_webpage(canonical_url, video_id, impersonate=True)
        stream_data = self._search_json(
            r"\bvar\s+stream_data\s*=",
            webpage,
            "stream data",
            video_id,
            transform_source=js_to_json,
        )

        formats = []
        headers = {"Referer": canonical_url}
        for quality in self._KNOWN_QUALITIES:
            sources = stream_data.get(quality) or []
            if isinstance(sources, str):
                sources = [sources]
            for source in sources:
                media_url = url_or_none(source)
                if not media_url:
                    continue
                height = int_or_none(quality.removesuffix("p"))
                if determine_ext(media_url) == "m3u8":
                    formats.extend(
                        self._extract_m3u8_formats(
                            media_url,
                            video_id,
                            "mp4",
                            m3u8_id=quality,
                            fatal=False,
                            headers=headers,
                        )
                    )
                else:
                    formats.append(
                        {
                            "url": media_url,
                            "format_id": quality,
                            "height": height,
                            "http_headers": headers,
                        }
                    )

        title = self._html_search_meta(
            ["og:title", "twitter:title"], webpage, default=display_id or video_id
        )
        thumbnail = self._html_search_meta(
            ["og:image", "twitter:image"], webpage, default=None
        )
        return {
            "id": video_id,
            "display_id": display_id,
            "title": title,
            "thumbnail": thumbnail,
            "formats": formats,
            "age_limit": 18,
        }


class SpankBangPlaylistIE(InfoExtractor):
    IE_NAME = "spankbang:playlist"
    _VALID_URL = rf"https?://(?:www\.)?{re.escape(SPANKBANG_DOMAIN)}/(?P<id>[\da-z]+)/playlist/(?P<display_id>[^/?#]+)"

    def _real_extract(self, url):
        playlist_id, display_id = self._match_valid_url(url).group("id", "display_id")
        canonical_url = f"{SPANKBANG_BASE_URL}/{playlist_id}/playlist/{display_id}"
        webpage = self._download_webpage(canonical_url, playlist_id, impersonate=True)

        pattern = r'href=(["\'])(?P<path>/[\da-z]+-(?P<video_id>[\da-z]+)/playlist/[^"\']+)\1'
        entries = [
            self.url_result(urljoin(canonical_url, path), ie=SpankBangIE.ie_key())
            for path in orderedSet(
                match.group("path") for match in re.finditer(pattern, webpage)
            )
        ]

        if not entries:
            entries = [
                self.url_result(urljoin(canonical_url, path), ie=SpankBangIE.ie_key())
                for path in orderedSet(
                    re.findall(r'href=["\'](/[\da-z]+/(?:video|embed)/[^"\']+)["\']', webpage)
                )
            ]
        if not entries:
            raise ExtractorError("Could not find any playlist entries", expected=True)

        title = (
            self._html_search_meta(["og:title", "twitter:title"], webpage, default=None)
            or self._search_regex(
                r'data-testid=["\']playlist-title["\'][^>]*>([^<]+)',
                webpage,
                "playlist title",
                default=None,
            )
            or display_id
        )

        return self.playlist_result(entries, playlist_id, title)

"""yt-dlp extractors for public PMVHaven pages."""

import json
import re

from yt_dlp.extractor.common import InfoExtractor
from yt_dlp.utils import ExtractorError, orderedSet, url_or_none, urljoin


class PMVHavenIE(InfoExtractor):
    IE_NAME = "pmvhaven"
    _VALID_URL = r"https?://(?:www\.)?pmvhaven\.com/videos?/(?P<id>[^/?#]+)"

    def _real_extract(self, url):
        display_id = self._match_id(url)
        webpage = self._download_webpage(url, display_id, impersonate=True)
        info = self._search_json_ld(
            webpage, display_id, expected_type="VideoObject"
        )
        media_url = _media_url(info)
        if not media_url:
            raise ExtractorError(
                "PMVHaven did not publish a valid absolute media URL "
                "(the page metadata may contain a tag link instead)",
                expected=True,
            )

        if media_url.endswith(".m3u8"):
            formats = self._extract_m3u8_formats(
                media_url,
                display_id,
                "mp4",
                headers={"Referer": url},
            )
        else:
            formats = [{"url": media_url, "http_headers": {"Referer": url}}]

        tags = _page_tags(self, webpage, info)

        return {
            **info,
            "id": display_id.rsplit("_", 1)[-1],
            "display_id": display_id,
            "formats": formats,
            "tags": tags,
            "age_limit": 18,
        }


def _media_url(info):
    """Return a real media URL, never a relative tag/query link.

    PMVHaven has occasionally published ``url`` as a relative navigation link
    such as ``?tag=...``. Passing that value to yt-dlp makes the generic
    extractor request the tag URL and commonly results in an opaque 403.
    JSON-LD may expose the actual stream under ``contentUrl`` or ``video``.
    Only absolute HTTP(S) URLs are accepted here.
    """
    candidates = []
    for key in ("contentUrl", "url", "video", "embedUrl"):
        value = info.get(key) if isinstance(info, dict) else None
        if isinstance(value, dict):
            value = value.get("contentUrl") or value.get("url")
        if isinstance(value, str):
            candidates.append(value)

    for candidate in candidates:
        media_url = url_or_none(candidate)
        if media_url and media_url.startswith(("http://", "https://")):
            return media_url
    return None


def _page_tags(extractor, webpage, info):
    """Use only tags published by PMVHaven's page metadata or tag links."""
    values = info.get("keywords") if isinstance(info, dict) else None
    if isinstance(values, str):
        values = re.split(r"[,;]", values)
    tags = [str(value).strip() for value in (values or []) if str(value).strip()]
    keywords = extractor._html_search_meta("keywords", webpage, fatal=False)
    if keywords:
        tags.extend(part.strip() for part in re.split(r"[,;]", keywords) if part.strip())
    tags.extend(
        match.group("tag").strip()
        for match in re.finditer(
            r"href=[\"'][^\"']*/tags/[^\"']*[\"'][^>]*>(?P<tag>[^<]+)<",
            webpage,
            flags=re.IGNORECASE,
        )
        if match.group("tag").strip()
    )
    deduped = []
    for tag in tags:
        if tag.casefold() not in {value.casefold() for value in deduped}:
            deduped.append(tag)
    return deduped


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

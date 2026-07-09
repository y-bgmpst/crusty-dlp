"""yt-dlp extractors for Ooxxx-hosted embed players and PornZog wrappers."""

# yt-dlp's public package currently exposes incomplete third-party type stubs.
# Keep useful checking enabled while suppressing only stub-boundary diagnostics.
# pyright: reportMissingModuleSource=false, reportPrivateUsage=false, reportImplicitOverride=false

from __future__ import annotations

import base64
from collections.abc import Iterable
from typing import TYPE_CHECKING, cast

from yt_dlp.extractor.common import InfoExtractor
from yt_dlp.utils import classproperty, int_or_none, url_or_none

if TYPE_CHECKING:
    from yt_dlp.extractor.common import _InfoDict


_CYRILLIC_LOOKALIKES = str.maketrans({
    "А": "A",
    "В": "B",
    "Е": "E",
    "К": "K",
    "М": "M",
    "Н": "H",
    "О": "O",
    "Р": "P",
    "С": "C",
    "Т": "T",
    "Х": "X",
    "а": "a",
    "е": "e",
    "о": "o",
    "р": "p",
    "с": "c",
    "у": "y",
    "х": "x",
})


def _decode_video_url(value: str) -> str | None:
    normalized = value.translate(_CYRILLIC_LOOKALIKES)
    encoded_path, _, encoded_query = normalized.partition(",")
    if not encoded_path:
        return None

    def _decode_part(part: str) -> str:
        part = part.replace("~", "")
        padding = "=" * ((4 - len(part) % 4) % 4)
        return base64.b64decode(part + padding).decode("utf-8")

    path = _decode_part(encoded_path)
    query = _decode_part(encoded_query) if encoded_query else ""
    return f"{path}?{query}" if query else path


def _first_meta(
    extractor: InfoExtractor, names: Iterable[str], webpage: str
) -> str | None:
    for name in names:
        value = extractor._html_search_meta(name, webpage, fatal=False)
        if value:
            return value
    return None


class OoxxxEmbedIE(InfoExtractor):
    _VALID_URL: str = r"https?://(?P<host>(?:(?:www|video)oxxx\.com|ooxxx\.com))/+(?:embed|videos?)/(?P<id>\d+)(?:/video/[^?#/]+)?/?(?:\?(?P<query>[^#]+))?"

    @classproperty
    def IE_NAME(cls) -> str:  # pyright: ignore[reportIncompatibleMethodOverride]
        return "ooxxx:embed"

    def _real_extract(self, url: str) -> _InfoDict:
        match = self._match_valid_url(url)
        assert match is not None
        video_id, query = match.group("id", "query")
        canonical_url = f"https://ooxxx.com/embed/{video_id}/"
        if query:
            canonical_url = f"{canonical_url}?{query}"

        webpage = self._download_webpage(canonical_url, video_id, impersonate=True)
        canonical_url = cast(str, self._html_search_regex(
            r'<link[^>]+rel=["\']canonical["\'][^>]+href=["\']([^"\']+)',
            webpage,
            "canonical url",
            default=canonical_url,
        ))
        api_url = f"https://ooxxx.com/api/videofile.php?video_id={video_id}"
        payload = cast(object, self._download_json(
            api_url,
            video_id,
            headers={"Referer": canonical_url},
            impersonate=True,
        ))
        stream = cast(
            dict[str, object],
            payload[0]
            if isinstance(payload, list) and payload and isinstance(payload[0], dict)
            else {},
        )
        encoded_url = stream.get("video_url")
        decoded_url = _decode_video_url(encoded_url) if isinstance(encoded_url, str) else None
        media_url = url_or_none(decoded_url and f"https://ooxxx.com{decoded_url}")
        if not media_url:
            raise self.raise_no_formats("No media URL was returned by the player API")

        title = _first_meta(self, ("og:title", "twitter:title"), webpage) or f"video-{video_id}"
        thumbnail = _first_meta(self, ("og:image", "twitter:image"), webpage)
        duration = int_or_none(
            self._html_search_meta("og:video:duration", webpage, fatal=False)
        )
        return {
            "id": video_id,
            "title": title,
            "thumbnail": thumbnail,
            "duration": duration,
            "formats": [{
                "url": media_url,
                "format_id": "mp4",
                "ext": "mp4",
                "http_headers": {"Referer": canonical_url},
            }],
            "age_limit": 18,
        }


class PornZogIE(InfoExtractor):
    _VALID_URL: str = r"https?://(?:www\.)?pornzog\.com/video/(?P<id>\d+)(?:/[^?#]+)?/?$"

    @classproperty
    def IE_NAME(cls) -> str:  # pyright: ignore[reportIncompatibleMethodOverride]
        return "pornzog"

    def _real_extract(self, url: str) -> _InfoDict:
        video_id = self._match_id(url)
        webpage = self._download_webpage(url, video_id, impersonate=True)
        iframe_url = self._search_regex(
            r'<iframe[^>]+src=["\'](https?://(?:videooxxx|ooxxx)\.com/[^"\']+)',
            webpage,
            "embedded player",
        )
        title = _first_meta(self, ("og:title", "twitter:title"), webpage) or f"video-{video_id}"
        thumbnail = _first_meta(self, ("og:image", "twitter:image"), webpage)
        return cast(_InfoDict, cast(object, self.url_result(
            iframe_url,
            OoxxxEmbedIE(),
            video_id=video_id,
            video_title=title,
            url_transparent=True,
            **{
                "title": title,
                "thumbnail": thumbnail,
                "age_limit": 18,
            },
        )))

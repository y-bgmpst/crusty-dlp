"""yt-dlp extractors for Ooxxx-hosted embed players and PornZog wrappers."""

import base64

from yt_dlp.extractor.common import InfoExtractor
from yt_dlp.utils import int_or_none, traverse_obj, url_or_none


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


def _decode_video_url(value):
    normalized = value.translate(_CYRILLIC_LOOKALIKES)
    encoded_path, _, encoded_query = normalized.partition(",")
    if not encoded_path:
        return None

    def _decode_part(part):
        part = part.replace("~", "")
        padding = "=" * ((4 - len(part) % 4) % 4)
        return base64.b64decode(part + padding).decode("utf-8")

    path = _decode_part(encoded_path)
    query = _decode_part(encoded_query) if encoded_query else ""
    return f"{path}?{query}" if query else path


class OoxxxEmbedIE(InfoExtractor):
    IE_NAME = "ooxxx:embed"
    _VALID_URL = r"https?://(?P<host>(?:(?:www|video)oxxx\.com|ooxxx\.com))/+(?:embed|videos?)/(?P<id>\d+)(?:/video/[^?#/]+)?/?(?:\?(?P<query>[^#]+))?"

    def _real_extract(self, url):
        match = self._match_valid_url(url)
        host, video_id, query = match.group("host", "id", "query")
        canonical_url = f"https://ooxxx.com/embed/{video_id}/"
        if query:
            canonical_url = f"{canonical_url}?{query}"

        webpage = self._download_webpage(canonical_url, video_id, impersonate=True)
        canonical_url = self._html_search_regex(
            r'<link[^>]+rel=["\']canonical["\'][^>]+href=["\']([^"\']+)',
            webpage,
            "canonical url",
            default=canonical_url,
        )
        api_url = f"https://ooxxx.com/api/videofile.php?video_id={video_id}"
        payload = self._download_json(
            api_url,
            video_id,
            headers={"Referer": canonical_url},
            impersonate=True,
        )
        stream = traverse_obj(payload, (0, {dict})) or {}
        decoded_url = _decode_video_url(stream.get("video_url", ""))
        media_url = url_or_none(decoded_url and f"https://ooxxx.com{decoded_url}")
        if not media_url:
            raise self.raise_no_formats("No media URL was returned by the player API")

        title = self._html_search_meta(
            ["og:title", "twitter:title"], webpage, default=f"video-{video_id}"
        )
        thumbnail = self._html_search_meta(
            ["og:image", "twitter:image"], webpage, default=None
        )
        duration = int_or_none(self._html_search_meta("og:video:duration", webpage, default=None))
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
    IE_NAME = "pornzog"
    _VALID_URL = r"https?://(?:www\.)?pornzog\.com/video/(?P<id>\d+)(?:/[^?#]+)?/?$"

    def _real_extract(self, url):
        video_id = self._match_id(url)
        webpage = self._download_webpage(url, video_id, impersonate=True)
        iframe_url = self._search_regex(
            r'<iframe[^>]+src=["\'](https?://(?:videooxxx|ooxxx)\.com/[^"\']+)',
            webpage,
            "embedded player",
        )
        title = self._html_search_meta(
            ["og:title", "twitter:title"], webpage, default=f"video-{video_id}"
        )
        thumbnail = self._html_search_meta(
            ["og:image", "twitter:image"], webpage, default=None
        )
        return self.url_result(
            iframe_url,
            OoxxxEmbedIE,
            video_id=video_id,
            video_title=title,
            url_transparent=True,
            **{
                "title": title,
                "thumbnail": thumbnail,
                "age_limit": 18,
            },
        )

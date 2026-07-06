"""Independent yt-dlp extractor for public BoyfriendTV video pages."""

from yt_dlp.extractor.common import InfoExtractor
from yt_dlp.utils import clean_html, determine_ext, js_to_json, url_or_none


class BoyfriendTVIE(InfoExtractor):
    IE_NAME = "boyfriendtv"
    _VALID_URL = r"https?://(?:www\.)?boyfriendtv\.com/(?:[a-z]{2}/)?videos/(?P<id>\d+)(?:/[^/?#]+)?"

    def _real_extract(self, url):
        video_id = self._match_id(url)
        slug = url.rstrip("/").rsplit("/", 1)[-1]
        canonical_url = f"https://www.boyfriendtv.com/videos/{video_id}/{slug}/"
        webpage = self._download_webpage(canonical_url, video_id, impersonate=True)
        sources = self._search_json(
            r"\bsources\s*:",
            webpage,
            "media sources",
            video_id,
            transform_source=js_to_json,
        )

        formats = []
        headers = {"Referer": canonical_url}
        if isinstance(sources, dict) and sources.get("hlsAuto"):
            formats.extend(
                self._extract_m3u8_formats(
                    sources["hlsAuto"],
                    video_id,
                    "mp4",
                    m3u8_id="hls",
                    headers=headers,
                )
            )
        source_list = sources if isinstance(sources, list) else (
            sources.get("hls") or sources.get("mp4") or []
        )
        for source in source_list:
            source_url = url_or_none(source.get("src") or source.get("file"))
            if not source_url:
                continue
            label = str(source.get("desc") or source.get("label") or "source")
            if determine_ext(source_url) == "m3u8":
                formats.extend(
                    self._extract_m3u8_formats(
                        source_url,
                        video_id,
                        "mp4",
                        m3u8_id=label,
                        fatal=False,
                        headers=headers,
                    )
                )
            else:
                formats.append(
                    {
                        "url": source_url,
                        "format_id": label,
                        "http_headers": headers,
                    }
                )

        title = self._html_search_meta(
            ["og:title", "twitter:title"], webpage, default=None
        ) or clean_html(
            self._search_regex(
                r"<h1[^>]*>(.+?)</h1>", webpage, "title", default=video_id
            )
        )
        return {
            "id": video_id,
            "title": title,
            "formats": formats,
            "age_limit": 18,
        }

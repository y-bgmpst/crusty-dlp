use anyhow::{anyhow, Result};

use crate::urls;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchPlatform {
    YouTube,
    Vimeo,
    Dailymotion,
    Twitch,
    TikTok,
    Instagram,
    XTwitter,
    SoundCloud,
    SpankBang,
    PornHub,
    XHamster,
    XVideos,
    XNXX,
    YouPorn,
    Eporner,
    RedTube,
    Beeg,
    SunPorno,
    DrTuber,
    TnaFlix,
    Txxx,
    ThisVid,
}

impl SearchPlatform {
    pub const ALL: [Self; 22] = [
        Self::YouTube,
        Self::Vimeo,
        Self::Dailymotion,
        Self::Twitch,
        Self::TikTok,
        Self::Instagram,
        Self::XTwitter,
        Self::SoundCloud,
        Self::SpankBang,
        Self::PornHub,
        Self::XHamster,
        Self::XVideos,
        Self::XNXX,
        Self::YouPorn,
        Self::Eporner,
        Self::RedTube,
        Self::Beeg,
        Self::SunPorno,
        Self::DrTuber,
        Self::TnaFlix,
        Self::Txxx,
        Self::ThisVid,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::YouTube => "YouTube",
            Self::Vimeo => "Vimeo",
            Self::Dailymotion => "Dailymotion",
            Self::Twitch => "Twitch",
            Self::TikTok => "TikTok",
            Self::Instagram => "Instagram",
            Self::XTwitter => "X / Twitter",
            Self::SoundCloud => "SoundCloud",
            Self::SpankBang => "SpankBang",
            Self::PornHub => "PornHub",
            Self::XHamster => "xHamster",
            Self::XVideos => "XVideos",
            Self::XNXX => "XNXX",
            Self::YouPorn => "YouPorn",
            Self::Eporner => "Eporner",
            Self::RedTube => "RedTube",
            Self::Beeg => "Beeg",
            Self::SunPorno => "SunPorno",
            Self::DrTuber => "DrTuber",
            Self::TnaFlix => "TNAFlix",
            Self::Txxx => "TXXX",
            Self::ThisVid => "ThisVid",
        }
    }

    pub fn config_value(self) -> &'static str {
        match self {
            Self::YouTube => "youtube",
            Self::Vimeo => "vimeo",
            Self::Dailymotion => "dailymotion",
            Self::Twitch => "twitch",
            Self::TikTok => "tiktok",
            Self::Instagram => "instagram",
            Self::XTwitter => "x",
            Self::SoundCloud => "soundcloud",
            Self::SpankBang => "spankbang",
            Self::PornHub => "pornhub",
            Self::XHamster => "xhamster",
            Self::XVideos => "xvideos",
            Self::XNXX => "xnxx",
            Self::YouPorn => "youporn",
            Self::Eporner => "eporner",
            Self::RedTube => "redtube",
            Self::Beeg => "beeg",
            Self::SunPorno => "sunporno",
            Self::DrTuber => "drtuber",
            Self::TnaFlix => "tnaflix",
            Self::Txxx => "txxx",
            Self::ThisVid => "thisvid",
        }
    }

    pub fn from_config(value: &str) -> Self {
        match value {
            "vimeo" => Self::Vimeo,
            "dailymotion" => Self::Dailymotion,
            "twitch" => Self::Twitch,
            "tiktok" => Self::TikTok,
            "instagram" => Self::Instagram,
            "x" | "twitter" => Self::XTwitter,
            "soundcloud" => Self::SoundCloud,
            "spankbang" => Self::SpankBang,
            "pornhub" => Self::PornHub,
            "xhamster" => Self::XHamster,
            "xvideos" => Self::XVideos,
            "xnxx" => Self::XNXX,
            "youporn" => Self::YouPorn,
            "eporner" => Self::Eporner,
            "redtube" => Self::RedTube,
            "beeg" => Self::Beeg,
            "sunporno" => Self::SunPorno,
            "drtuber" => Self::DrTuber,
            "tnaflix" => Self::TnaFlix,
            "txxx" => Self::Txxx,
            "thisvid" => Self::ThisVid,
            _ => Self::YouTube,
        }
    }

    pub fn next(self) -> Self {
        let index = Self::ALL
            .iter()
            .position(|value| *value == self)
            .unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    pub fn search_url(self, query: &str) -> String {
        match self {
            Self::YouTube => urls::youtube_search_url(query),
            Self::Vimeo => urls::vimeo_search_url(query),
            Self::Dailymotion => urls::dailymotion_search_url(query),
            Self::Twitch => urls::twitch_search_url(query),
            Self::TikTok => urls::tiktok_search_url(query),
            Self::Instagram => urls::instagram_search_url(query),
            Self::XTwitter => urls::xtwitter_search_url(query),
            Self::SoundCloud => urls::soundcloud_search_url(query),
            Self::SpankBang => urls::spankbang_search_url(query),
            Self::PornHub => urls::pornhub_search_url(query),
            Self::XHamster => urls::xhamster_search_url(query),
            Self::XVideos => urls::xvideos_search_url(query),
            Self::XNXX => urls::xnxx_search_url(query),
            Self::YouPorn => urls::youporn_search_url(query),
            Self::Eporner => urls::eporner_search_url(query),
            Self::RedTube => urls::redtube_search_url(query),
            Self::Beeg => urls::beeg_search_url(query),
            Self::SunPorno => urls::sunporno_search_url(query),
            Self::DrTuber => urls::drtuber_search_url(query),
            Self::TnaFlix => urls::tnaflix_search_url(query),
            Self::Txxx => urls::txxx_search_url(query),
            Self::ThisVid => urls::thisvid_search_url(query),
        }
    }
}

pub fn open_platform_search(query: &str, platform: SearchPlatform) -> Result<String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("Search query cannot be empty"));
    }
    let url = platform.search_url(trimmed);
    webbrowser::open(&url).map_err(|error| anyhow!("Could not open browser: {error}"))?;
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restores_platform_from_config() {
        assert_eq!(
            SearchPlatform::from_config("youtube"),
            SearchPlatform::YouTube
        );
        assert_eq!(
            SearchPlatform::from_config("spankbang"),
            SearchPlatform::SpankBang
        );
        assert_eq!(SearchPlatform::from_config("vimeo"), SearchPlatform::Vimeo);
    }

    #[test]
    fn generates_search_urls() {
        assert_eq!(
            SearchPlatform::YouTube.search_url("test video"),
            "https://www.youtube.com/results?search_query=test%20video"
        );
        assert_eq!(
            SearchPlatform::Vimeo.search_url("demo reel"),
            "https://vimeo.com/search?q=demo%20reel"
        );
        assert_eq!(
            SearchPlatform::PornHub.search_url("hello"),
            "https://www.pornhub.com/video/search?search=hello"
        );
        assert_eq!(
            SearchPlatform::ThisVid.search_url("alpha popper"),
            "https://thisvid.com/search/?q=alpha%20popper"
        );
    }
}

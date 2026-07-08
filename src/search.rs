use anyhow::{anyhow, Result};

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
}

impl SearchPlatform {
    pub const ALL: [Self; 21] = [
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
        let encoded = urlencoding::encode(query);
        match self {
            Self::YouTube => {
                format!("https://www.youtube.com/results?search_query={encoded}")
            }
            Self::Vimeo => format!("https://vimeo.com/search?q={encoded}"),
            Self::Dailymotion => format!("https://www.dailymotion.com/search/{encoded}/videos"),
            Self::Twitch => format!("https://www.twitch.tv/search?term={encoded}"),
            Self::TikTok => format!("https://www.tiktok.com/search?q={encoded}"),
            Self::Instagram => {
                format!("https://www.instagram.com/explore/search/keyword/?q={encoded}")
            }
            Self::XTwitter => format!("https://x.com/search?q={encoded}&src=typed_query&f=live"),
            Self::SoundCloud => format!("https://soundcloud.com/search?q={encoded}"),
            Self::SpankBang => format!("https://spankbang.com/s/{encoded}/"),
            Self::PornHub => format!("https://www.pornhub.com/video/search?search={encoded}"),
            Self::XHamster => format!("https://xhamster.com/search/{encoded}"),
            Self::XVideos => format!("https://www.xvideos.com/?k={encoded}"),
            Self::XNXX => format!("https://www.xnxx.com/search/{encoded}"),
            Self::YouPorn => format!("https://www.youporn.com/search/?query={encoded}"),
            Self::Eporner => format!("https://www.eporner.com/search/{encoded}/"),
            Self::RedTube => format!("https://www.redtube.com/?search={encoded}"),
            Self::Beeg => format!("https://beeg.com/search?query={encoded}"),
            Self::SunPorno => format!("https://www.sunporno.com/search/{encoded}/"),
            Self::DrTuber => format!("https://www.drtuber.com/search/videos/{encoded}"),
            Self::TnaFlix => format!("https://www.tnaflix.com/search/{encoded}"),
            Self::Txxx => format!("https://txxx.com/search/{encoded}"),
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
    }
}

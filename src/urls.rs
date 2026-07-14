pub const YOUTUBE_WATCH_BASE: &str = "https://www.youtube.com/watch?v=";
pub const PMVHAVEN_VIDEO_BASE: &str = "https://pmvhaven.com/video/";
pub const PMVHAVEN_PLAYLIST_BASE: &str = "https://pmvhaven.com/playlists/";
pub const SPANKBANG_VIDEO_BASE: &str = "https://spankbang.com/";

pub fn youtube_search_url(query: &str) -> String {
    format!(
        "https://www.youtube.com/results?search_query={}",
        urlencoding::encode(query)
    )
}

pub fn vimeo_search_url(query: &str) -> String {
    format!("https://vimeo.com/search?q={}", urlencoding::encode(query))
}

pub fn dailymotion_search_url(query: &str) -> String {
    format!(
        "https://www.dailymotion.com/search/{}/videos",
        urlencoding::encode(query)
    )
}

pub fn twitch_search_url(query: &str) -> String {
    format!(
        "https://www.twitch.tv/search?term={}",
        urlencoding::encode(query)
    )
}

pub fn tiktok_search_url(query: &str) -> String {
    format!(
        "https://www.tiktok.com/search?q={}",
        urlencoding::encode(query)
    )
}

pub fn instagram_search_url(query: &str) -> String {
    format!(
        "https://www.instagram.com/explore/search/keyword/?q={}",
        urlencoding::encode(query)
    )
}

pub fn xtwitter_search_url(query: &str) -> String {
    format!(
        "https://x.com/search?q={}&src=typed_query&f=live",
        urlencoding::encode(query)
    )
}

pub fn soundcloud_search_url(query: &str) -> String {
    format!(
        "https://soundcloud.com/search?q={}",
        urlencoding::encode(query)
    )
}

pub fn spankbang_search_url(query: &str) -> String {
    format!("https://spankbang.com/s/{}/", urlencoding::encode(query))
}

pub fn pornhub_search_url(query: &str) -> String {
    format!(
        "https://www.pornhub.com/video/search?search={}",
        urlencoding::encode(query)
    )
}

pub fn xhamster_search_url(query: &str) -> String {
    format!("https://xhamster.com/search/{}", urlencoding::encode(query))
}

pub fn xvideos_search_url(query: &str) -> String {
    format!("https://www.xvideos.com/?k={}", urlencoding::encode(query))
}

pub fn xnxx_search_url(query: &str) -> String {
    format!("https://www.xnxx.com/search/{}", urlencoding::encode(query))
}

pub fn youporn_search_url(query: &str) -> String {
    format!(
        "https://www.youporn.com/search/?query={}",
        urlencoding::encode(query)
    )
}

pub fn eporner_search_url(query: &str) -> String {
    format!(
        "https://www.eporner.com/search/{}/",
        urlencoding::encode(query)
    )
}

pub fn redtube_search_url(query: &str) -> String {
    format!(
        "https://www.redtube.com/?search={}",
        urlencoding::encode(query)
    )
}

pub fn beeg_search_url(query: &str) -> String {
    format!(
        "https://beeg.com/search?query={}",
        urlencoding::encode(query)
    )
}

pub fn sunporno_search_url(query: &str) -> String {
    format!(
        "https://www.sunporno.com/search/{}/",
        urlencoding::encode(query)
    )
}

pub fn drtuber_search_url(query: &str) -> String {
    format!(
        "https://www.drtuber.com/search/videos/{}",
        urlencoding::encode(query)
    )
}

pub fn tnaflix_search_url(query: &str) -> String {
    format!(
        "https://www.tnaflix.com/search/{}",
        urlencoding::encode(query)
    )
}

pub fn txxx_search_url(query: &str) -> String {
    format!("https://txxx.com/search/{}", urlencoding::encode(query))
}

pub fn thisvid_search_url(query: &str) -> String {
    format!(
        "https://thisvid.com/search/?q={}",
        urlencoding::encode(query)
    )
}

pub fn youtube_watch_url(id: &str) -> String {
    format!("{YOUTUBE_WATCH_BASE}{id}")
}

pub fn pmvhaven_video_url(slug: &str) -> String {
    format!("{PMVHAVEN_VIDEO_BASE}{slug}")
}

pub fn pmvhaven_playlist_url(id: &str) -> String {
    format!("{PMVHAVEN_PLAYLIST_BASE}{id}")
}

pub fn spankbang_video_url(id: &str) -> String {
    format!("{SPANKBANG_VIDEO_BASE}{id}/video/{id}")
}

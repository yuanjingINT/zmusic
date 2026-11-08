//! OpenGameArt：没有官方 API，只能抓搜索结果页 + 逐条内容页找音频直链。
//! 页面结构一变就会降级（源置灰），这是已知代价。

use crate::models::{Album, Artist, Lyrics, SearchResult, Song};
use crate::source::http;
use crate::source::{Capabilities, SearchKind, Source, TrackId};
use anyhow::Result;

const BASE: &str = "https://opengameart.org";
const PAGE_LIMIT: usize = 5;

pub struct OpenGameArtSource {
    base: String,
}

impl OpenGameArtSource {
    pub fn new() -> Self {
        OpenGameArtSource {
            base: BASE.to_string(),
        }
    }

    #[cfg(test)]
    pub fn with_base(base: &str) -> Self {
        OpenGameArtSource {
            base: base.to_string(),
        }
    }
}

/// 从搜索页里抽出内容页路径
pub fn parse_content_links(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in html.split("href=\"/content/").skip(1) {
        if let Some(end) = part.find('"') {
            let slug = &part[..end];
            if slug.is_empty() || slug.starts_with("faq") {
                continue;
            }
            let path = format!("/content/{slug}");
            if !out.contains(&path) {
                out.push(path);
            }
        }
    }
    out
}

/// 从内容页里抽音频直链
pub fn parse_audio_link(html: &str) -> Option<String> {
    for ext in [".mp3", ".ogg", ".flac", ".wav", ".m4a"] {
        if let Some(pos) = html.find(ext) {
            let head = &html[..pos + ext.len()];
            if let Some(start) = head.rfind("href=\"") {
                let url = &head[start + 6..];
                if url.starts_with("http") || url.starts_with('/') {
                    return Some(url.to_string());
                }
            }
        }
    }
    None
}

impl Source for OpenGameArtSource {
    fn id(&self) -> &'static str {
        "opengameart"
    }

    fn name(&self) -> &'static str {
        "OpenGameArt"
    }

    fn group(&self) -> &'static str {
        "公开免费"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            kinds: vec![SearchKind::Song],
            lyrics: false,
            qualities: Vec::new(),
        }
    }

    fn is_available(&self) -> bool {
        http::get_text(&format!("{}/", self.base)).is_ok()
    }

    fn search(&self, keyword: &str, limit: i32, kind: SearchKind) -> Result<SearchResult> {
        if kind != SearchKind::Song {
            return Ok(SearchResult::default());
        }
        let html = http::get_text(&format!(
            "{}/art-search-advanced?keys={}&field_art_type_tid%5B%5D=12",
            self.base,
            http::enc(keyword)
        ))?;
        let mut songs = Vec::new();
        for path in parse_content_links(&html).into_iter() {
            if songs.len() >= (limit as usize).min(PAGE_LIMIT) {
                break;
            }
            // 搜索结果里混着非音频页，逐页确认真的有音频再收
            let Ok(page) = http::get_text(&format!("{}{}", self.base, path)) else {
                continue;
            };
            if parse_audio_link(&page).is_none() {
                continue;
            }
            let name = path.trim_start_matches("/content/").replace('-', " ");
            songs.push(Song {
                id: 0,
                name,
                ar: vec![Artist {
                    id: 0,
                    name: "OpenGameArt".to_string(),
                }],
                al: Album {
                    id: 0,
                    name: "OpenGameArt".to_string(),
                    pic_url: None,
                },
                dt: 0,
                fee: 0,
                source: "opengameart".to_string(),
                s_id: path,
            });
        }
        Ok(SearchResult {
            songs,
            ..Default::default()
        })
    }

    fn lyric(&self, _id: &TrackId) -> Result<Lyrics> {
        Ok(Lyrics::default())
    }

    fn stream_url(&self, id: &TrackId, _quality: &str) -> Result<Option<String>> {
        let html = http::get_text(&format!("{}{}", self.base, id.as_str()))?;
        Ok(parse_audio_link(&html).map(|u| {
            if u.starts_with('/') {
                format!("{}{}", self.base, u)
            } else {
                u
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_content_links() {
        let html = r#"<a href="/content/5-chiptunes-action">x</a>
                      <a href="/content/faq">faq</a>
                      <a href="/content/5-chiptunes-action">dup</a>"#;
        let links = parse_content_links(html);
        assert_eq!(links, vec!["/content/5-chiptunes-action".to_string()]);
    }

    #[test]
    fn extracts_audio_link() {
        let html = r#"<audio src="x"><a href="/sites/default/files/tune.mp3">download</a>"#;
        assert_eq!(
            parse_audio_link(html),
            Some("/sites/default/files/tune.mp3".to_string())
        );
        assert_eq!(parse_audio_link("<html>nothing</html>"), None);
    }

    #[test]
    fn unreachable_base_degrades_to_error() {
        let s = OpenGameArtSource::with_base("http://127.0.0.1:9");
        assert!(!s.is_available());
        assert!(s.search("x", 3, SearchKind::Song).is_err());
    }

    #[test]
    #[ignore]
    fn opengameart_live_search() {
        let s = OpenGameArtSource::new();
        let r = s.search("love", 3, SearchKind::Song).expect("真网搜索失败");
        println!("opengameart_live_search: {} 条", r.songs.len());
        for song in r.songs.iter().take(3) {
            println!("  - {}  [{} ms]  s_id={}", song.name, song.dt, song.s_id);
        }
    }
}

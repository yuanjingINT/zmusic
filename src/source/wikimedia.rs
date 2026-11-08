//! Wikimedia Commons：MediaWiki 公开 API。搜 File 命名空间里的音频，再用 imageinfo 取直链。

use crate::models::{Album, Artist, Lyrics, SearchResult, Song};
use crate::source::http;
use crate::source::{Capabilities, SearchKind, Source, TrackId};
use anyhow::Result;
use serde_json::Value;

const BASE: &str = "https://commons.wikimedia.org";

pub struct WikimediaSource {
    base: String,
}

impl WikimediaSource {
    pub fn new() -> Self {
        WikimediaSource {
            base: BASE.to_string(),
        }
    }

    #[cfg(test)]
    pub fn with_base(base: &str) -> Self {
        WikimediaSource {
            base: base.to_string(),
        }
    }
}

fn strip_html(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.trim().to_string()
}

/// 从 `imageinfo` 的单页对象里取直链/时长/作者
fn song_from_page(page: &Value) -> Option<Song> {
    let title = page.get("title")?.as_str()?;
    let ii = page.get("imageinfo")?.as_array()?.first()?;
    ii.get("url")?.as_str()?;
    let mime = ii.get("mime").and_then(|x| x.as_str()).unwrap_or("");
    if !mime.starts_with("audio/") {
        return None;
    }
    let name = title
        .trim_start_matches("File:")
        .rsplit_once('.')
        .map(|(n, _)| n)
        .unwrap_or(title)
        .replace('_', " ");
    let meta = ii.get("extmetadata");
    let artist = meta
        .and_then(|m| m.get("Artist"))
        .and_then(|a| a.get("value"))
        .and_then(|v| v.as_str())
        .map(strip_html)
        .filter(|s| !s.is_empty())
        .unwrap_or("Wikimedia".to_string());
    let dt = meta
        .and_then(|m| m.get("Duration"))
        .and_then(|d| d.get("value"))
        .and_then(|v| v.as_str())
        .map(http::duration_ms)
        .unwrap_or(0);
    Some(Song {
        id: 0,
        name,
        ar: vec![Artist { id: 0, name: artist }],
        al: Album {
            id: 0,
            name: "Wikimedia Commons".to_string(),
            pic_url: None,
        },
        dt,
        fee: 0,
        source: "wikimedia".to_string(),
        s_id: page.get("pageid")?.as_i64()?.to_string(),
    })
}

/// 从 `query.pages` 解析（对象形态，key 是 pageid）
pub fn parse_pages(v: &Value) -> Vec<Song> {
    v.pointer("/query/pages")
        .and_then(|p| p.as_object())
        .map(|m| m.values().filter_map(song_from_page).collect())
        .unwrap_or_default()
}

fn clean_url(u: &str) -> String {
    u.split('?').next().unwrap_or(u).to_string()
}

impl Source for WikimediaSource {
    fn id(&self) -> &'static str {
        "wikimedia"
    }

    fn name(&self) -> &'static str {
        "Wikimedia Commons"
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
        http::get_json(&format!(
            "{}/w/api.php?action=query&list=search&srsearch=audio&srnamespace=6&srlimit=1&format=json",
            self.base
        ))
        .is_ok()
    }

    fn search(&self, keyword: &str, limit: i32, kind: SearchKind) -> Result<SearchResult> {
        if kind != SearchKind::Song {
            return Ok(SearchResult::default());
        }
        let n = limit.clamp(1, 20);
        let search = http::get_json(&format!(
            "{}/w/api.php?action=query&list=search&srsearch={}%20filetype:audio&srnamespace=6&srlimit={}&format=json",
            self.base,
            http::enc(keyword),
            n
        ))?;
        let ids: Vec<String> = search
            .pointer("/query/search")
            .and_then(|s| s.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.get("pageid")?.as_i64().map(|v| v.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        if ids.is_empty() {
            return Ok(SearchResult::default());
        }
        let info = http::get_json(&format!(
            "{}/w/api.php?action=query&pageids={}&prop=imageinfo&iiprop=url%7Cmime%7Cextmetadata&format=json",
            self.base,
            ids.join("%7C")
        ))?;
        Ok(SearchResult {
            songs: parse_pages(&info),
            ..Default::default()
        })
    }

    fn lyric(&self, _id: &TrackId) -> Result<Lyrics> {
        Ok(Lyrics::default())
    }

    fn stream_url(&self, id: &TrackId, _quality: &str) -> Result<Option<String>> {
        let v = http::get_json(&format!(
            "{}/w/api.php?action=query&pageids={}&prop=imageinfo&iiprop=url%7Cmime&format=json",
            self.base,
            http::enc(id.as_str())
        ))?;
        let url = v
            .pointer("/query/pages")
            .and_then(|p| p.as_object())
            .and_then(|m| m.values().next())
            .and_then(|p| p.pointer("/imageinfo/0/url"))
            .and_then(|u| u.as_str())
            .map(clean_url);
        Ok(url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Value {
        json!({"query":{"pages":{"90823813":{
            "pageid":90823813,
            "title":"File:Bourne woods 2020-05-31 0823.mp3",
            "imageinfo":[{
                "url":"https://upload.wikimedia.org/x/Bourne.mp3?utm_source=commons",
                "mime":"audio/mpeg",
                "extmetadata":{
                    "Artist":{"value":"<a href=\"#\">Some Photographer</a>"},
                    "Duration":{"value":"12.5"}
                }
            }]
        }}}})
    }

    #[test]
    fn parses_imageinfo_into_song() {
        let songs = parse_pages(&sample());
        assert_eq!(songs.len(), 1);
        assert_eq!(songs[0].name, "Bourne woods 2020-05-31 0823");
        assert_eq!(songs[0].ar[0].name, "Some Photographer");
        assert_eq!(songs[0].dt, 12_500);
        assert_eq!(songs[0].s_id, "90823813");
    }

    #[test]
    fn non_audio_pages_are_skipped() {
        let v = json!({"query":{"pages":{"1":{"pageid":1,"title":"File:x.jpg",
            "imageinfo":[{"url":"https://x/y.jpg","mime":"image/jpeg"}]}}}});
        assert!(parse_pages(&v).is_empty());
    }

    #[test]
    fn unreachable_base_degrades_to_error() {
        let s = WikimediaSource::with_base("http://127.0.0.1:9");
        assert!(!s.is_available());
        assert!(s.search("x", 5, SearchKind::Song).is_err());
    }

    #[test]
    #[ignore]
    fn wikimedia_live_search() {
        let s = WikimediaSource::new();
        let r = s.search("love", 3, SearchKind::Song).expect("真网搜索失败");
        println!("wikimedia_live_search: {} 条", r.songs.len());
        for song in r.songs.iter().take(3) {
            println!("  - {}  [{} ms]  s_id={}", song.name, song.dt, song.s_id);
        }
    }
}

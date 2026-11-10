//! Audius：开放 API，无需账号，搜索和流都稳定（流是 302 到签名地址，交给 mpv 跟随）。

use crate::models::{Album, Artist, Lyrics, SearchResult, Song};
use crate::source::http;
use crate::source::{Capabilities, SearchKind, Source, TrackId};
use anyhow::Result;
use serde_json::Value;

const BASE: &str = "https://api.audius.co";

pub struct AudiusSource {
    base: String,
}

impl AudiusSource {
    pub fn new() -> Self {
        AudiusSource {
            base: BASE.to_string(),
        }
    }

    #[cfg(test)]
    pub fn with_base(base: &str) -> Self {
        AudiusSource {
            base: base.to_string(),
        }
    }
}

/// 解析 `/v1/tracks/search` 的 `data` 数组
pub fn parse_tracks(v: &Value) -> Vec<Song> {
    v.get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| {
                    let id = t.get("id")?.as_str()?;
                    let title = t
                        .get("title")
                        .and_then(|x| x.as_str())
                        .unwrap_or("(无标题)");
                    let artist = t
                        .get("user")
                        .and_then(|u| u.get("name"))
                        .and_then(|x| x.as_str())
                        .unwrap_or("Audius");
                    let dur = t.get("duration").and_then(|x| x.as_i64()).unwrap_or(0);
                    Some(Song {
                        id: 0,
                        name: title.to_string(),
                        ar: vec![Artist {
                            id: 0,
                            name: artist.to_string(),
                        }],
                        al: Album {
                            id: 0,
                            name: "Audius".to_string(),
                            pic_url: t
                                .get("artwork")
                                .and_then(|a| a.get("480x480"))
                                .and_then(|x| x.as_str())
                                .map(|s| s.to_string()),
                        },
                        dt: dur * 1000,
                        fee: 0,
                        source: "audius".to_string(),
                        s_id: id.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

impl Source for AudiusSource {
    fn id(&self) -> &'static str {
        "audius"
    }

    fn name(&self) -> &'static str {
        "Audius"
    }

    fn group(&self) -> &'static str {
        "公开免费"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            kinds: vec![SearchKind::Song],
            lyrics: false,
            qualities: Vec::new(),
            can_stream: true,
        }
    }

    fn is_available(&self) -> bool {
        http::get_json(&format!(
            "{}/v1/tracks/search?query=a&limit=1&app_name=zmuse",
            self.base
        ))
        .is_ok()
    }

    fn search(&self, keyword: &str, limit: i32, kind: SearchKind) -> Result<SearchResult> {
        if kind != SearchKind::Song {
            return Ok(SearchResult::default());
        }
        let v = http::get_json(&format!(
            "{}/v1/tracks/search?query={}&limit={}&app_name=zmuse",
            self.base,
            http::enc(keyword),
            limit.clamp(1, 50)
        ))?;
        Ok(SearchResult {
            songs: parse_tracks(&v),
            ..Default::default()
        })
    }

    fn lyric(&self, _id: &TrackId) -> Result<Lyrics> {
        Ok(Lyrics::default())
    }

    fn stream_url(&self, id: &TrackId, _quality: &str) -> Result<Option<String>> {
        Ok(Some(format!(
            "{}/v1/tracks/{}/stream?app_name=zmuse",
            self.base,
            http::enc(id.as_str())
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_search_payload() {
        let v = json!({"data":[
            {"id":"6RN8l","title":"Must B Love","duration":320,
             "user":{"name":"Someone"},"artwork":{"480x480":"https://x/y.jpg"}}
        ]});
        let songs = parse_tracks(&v);
        assert_eq!(songs.len(), 1);
        assert_eq!(songs[0].name, "Must B Love");
        assert_eq!(songs[0].ar[0].name, "Someone");
        assert_eq!(songs[0].dt, 320_000);
        assert_eq!(songs[0].source, "audius");
        assert_eq!(songs[0].s_id, "6RN8l");
    }

    #[test]
    fn stream_url_points_at_api() {
        let s = AudiusSource::new();
        let u = s
            .stream_url(&TrackId::from("6RN8l"), "exhigh")
            .unwrap()
            .unwrap();
        assert!(u.contains("/v1/tracks/6RN8l/stream"), "{u}");
    }

    #[test]
    fn unreachable_base_degrades_to_error() {
        let s = AudiusSource::with_base("http://127.0.0.1:9");
        assert!(!s.is_available());
        assert!(s.search("x", 5, SearchKind::Song).is_err());
    }

    #[test]
    #[ignore]
    fn audius_live_search() {
        let s = AudiusSource::new();
        let r = s.search("love", 3, SearchKind::Song).expect("真网搜索失败");
        println!("audius_live_search: {} 条", r.songs.len());
        for song in r.songs.iter().take(3) {
            println!("  - {}  [{} ms]  s_id={}", song.name, song.dt, song.s_id);
        }
    }
}

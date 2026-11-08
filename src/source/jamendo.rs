//! Jamendo：需要免费申请的 client_id（`ZMUSIC_JAMENDO_CLIENT_ID`）。
//! 没有 key 时该源自报不可用，不影响其它源。

use crate::models::{Album, Artist, Lyrics, SearchResult, Song};
use crate::source::http;
use crate::source::{Capabilities, SearchKind, Source, TrackId};
use anyhow::{anyhow, Result};
use serde_json::Value;

const BASE: &str = "https://api.jamendo.com";

pub struct JamendoSource {
    base: String,
    client_id: Option<String>,
}

impl JamendoSource {
    /// `client_id` 来自 config.toml `[jamendo]`（或 ZMUSIC_JAMENDO_CLIENT_ID）
    pub fn with_client_id(client_id: Option<String>) -> Self {
        JamendoSource {
            base: BASE.to_string(),
            client_id: client_id
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty()),
        }
    }

    #[cfg(test)]
    pub fn with_base(base: &str, client_id: Option<&str>) -> Self {
        JamendoSource {
            base: base.to_string(),
            client_id: client_id.map(|s| s.to_string()),
        }
    }

    fn url(&self, path: &str) -> Result<String> {
        let id = self
            .client_id
            .as_deref()
            .ok_or_else(|| anyhow!("Jamendo 需要 ZMUSIC_JAMENDO_CLIENT_ID"))?;
        Ok(format!(
            "{}{}?client_id={}&format=json{}",
            self.base,
            path,
            http::enc(id),
            ""
        ))
    }
}

/// 解析 `results` 数组
pub fn parse_results(v: &Value) -> Vec<Song> {
    v.get("results")
        .and_then(|r| r.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| {
                    let id = t.get("id")?.to_string().trim_matches('"').to_string();
                    Some(Song {
                        id: 0,
                        name: t
                            .get("name")
                            .and_then(|x| x.as_str())
                            .unwrap_or("(无标题)")
                            .to_string(),
                        ar: vec![Artist {
                            id: 0,
                            name: t
                                .get("artist_name")
                                .and_then(|x| x.as_str())
                                .unwrap_or("Jamendo")
                                .to_string(),
                        }],
                        al: Album {
                            id: 0,
                            name: "Jamendo".to_string(),
                            pic_url: t
                                .get("image")
                                .and_then(|x| x.as_str())
                                .map(|s| s.to_string()),
                        },
                        dt: t.get("duration").and_then(|x| x.as_i64()).unwrap_or(0) * 1000,
                        fee: 0,
                        source: "jamendo".to_string(),
                        s_id: id.trim_matches('"').to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

impl Source for JamendoSource {
    fn id(&self) -> &'static str {
        "jamendo"
    }

    fn name(&self) -> &'static str {
        "Jamendo"
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
        let Ok(u) = self.url("/v3.0/tracks/") else {
            return false;
        };
        http::get_json(&format!("{u}&limit=1&search=a")).is_ok()
    }

    fn search(&self, keyword: &str, limit: i32, kind: SearchKind) -> Result<SearchResult> {
        if kind != SearchKind::Song {
            return Ok(SearchResult::default());
        }
        let u = self.url("/v3.0/tracks/")?;
        let v = http::get_json(&format!(
            "{}&limit={}&search={}",
            u,
            limit.clamp(1, 30),
            http::enc(keyword)
        ))?;
        Ok(SearchResult {
            songs: parse_results(&v),
            ..Default::default()
        })
    }

    fn lyric(&self, _id: &TrackId) -> Result<Lyrics> {
        Ok(Lyrics::default())
    }

    fn stream_url(&self, id: &TrackId, _quality: &str) -> Result<Option<String>> {
        let u = self.url("/v3.0/tracks/")?;
        let v = http::get_json(&format!("{}&id={}", u, http::enc(id.as_str())))?;
        Ok(v.pointer("/results/0/audio")
            .and_then(|a| a.as_str())
            .map(|s| s.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_results_with_string_and_numeric_ids() {
        let v = json!({"results":[
            {"id":"123","name":"Song","artist_name":"Artist","duration":200,"audio":"https://x/a.mp3","image":"https://x/i.jpg"},
            {"id":456,"name":"Numeric","artist_name":"A2","duration":1}
        ]});
        let songs = parse_results(&v);
        assert_eq!(songs.len(), 2);
        assert_eq!(songs[0].s_id, "123");
        assert_eq!(songs[0].dt, 200_000);
        assert_eq!(songs[1].s_id, "456");
    }

    #[test]
    fn missing_client_id_is_unavailable() {
        let s = JamendoSource::with_base(BASE, None);
        assert!(!s.is_available());
        let err = s
            .search("x", 5, SearchKind::Song)
            .expect_err("无 key 必须报错");
        assert!(
            err.to_string().contains("ZMUSIC_JAMENDO_CLIENT_ID"),
            "{err}"
        );
    }

    #[test]
    fn unreachable_base_degrades_to_error() {
        let s = JamendoSource::with_base("http://127.0.0.1:9", Some("k"));
        assert!(!s.is_available());
        assert!(s.search("x", 5, SearchKind::Song).is_err());
    }
}

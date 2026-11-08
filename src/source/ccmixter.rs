//! ccMixter：公开 API，无需账号。文件列表里 `download_url` 就是直链。

use crate::models::{Album, Artist, Lyrics, SearchResult, Song};
use crate::source::http;
use crate::source::{Capabilities, SearchKind, Source, TrackId};
use anyhow::{anyhow, Result};
use serde_json::Value;

const BASE: &str = "https://ccmixter.org";

pub struct CcMixterSource {
    base: String,
}

impl CcMixterSource {
    pub fn new() -> Self {
        CcMixterSource {
            base: BASE.to_string(),
        }
    }

    #[cfg(test)]
    pub fn with_base(base: &str) -> Self {
        CcMixterSource {
            base: base.to_string(),
        }
    }

    fn pick_file(files: &Value) -> Option<&Value> {
        let arr = files.as_array()?;
        arr.iter()
            .find(|f| {
                f.get("file_nicname").and_then(|x| x.as_str()) == Some("mp3")
                    && f.get("download_url").and_then(|x| x.as_str()).is_some()
            })
            .or_else(|| {
                arr.iter().find(|f| {
                    f.get("download_url").and_then(|x| x.as_str()).is_some()
                        && f.pointer("/file_format_info/media-type")
                            .and_then(|x| x.as_str())
                            == Some("audio")
                })
            })
    }
}

/// 解析 `/api/query` 返回的数组
pub fn parse_uploads(v: &Value) -> Vec<Song> {
    let Some(arr) = v.as_array() else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|u| {
            let id = u.get("upload_id")?.as_i64()?;
            let file = CcMixterSource::pick_file(u.get("files")?)?;
            let title = u
                .get("upload_name")
                .and_then(|x| x.as_str())
                .unwrap_or("(无标题)");
            let artist = u
                .get("user_real_name")
                .and_then(|x| x.as_str())
                .filter(|s| !s.is_empty())
                .or_else(|| u.get("user_name").and_then(|x| x.as_str()))
                .unwrap_or("ccMixter");
            let ps = file
                .pointer("/file_format_info/ps")
                .and_then(|x| x.as_str())
                .map(http::duration_ms)
                .unwrap_or(0);
            Some(Song {
                id: 0,
                name: title.to_string(),
                ar: vec![Artist {
                    id: 0,
                    name: artist.to_string(),
                }],
                al: Album {
                    id: 0,
                    name: "ccMixter".to_string(),
                    pic_url: None,
                },
                dt: ps,
                fee: 0,
                source: "ccmixter".to_string(),
                s_id: id.to_string(),
            })
        })
        .collect()
}

/// 取第一条 upload 的可用音频直链
fn stream_of(v: &Value) -> Option<String> {
    let upload = v.as_array()?.first()?;
    let f = CcMixterSource::pick_file(upload.get("files")?)?;
    f.get("download_url")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
}

impl Source for CcMixterSource {
    fn id(&self) -> &'static str {
        "ccmixter"
    }

    fn name(&self) -> &'static str {
        "ccMixter"
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
        http::get_json(&format!("{}/api/query?f=json&search=a&limit=1", self.base)).is_ok()
    }

    fn search(&self, keyword: &str, limit: i32, kind: SearchKind) -> Result<SearchResult> {
        if kind != SearchKind::Song {
            return Ok(SearchResult::default());
        }
        let v = http::get_json(&format!(
            "{}/api/query?f=json&search={}&limit={}",
            self.base,
            http::enc(keyword),
            limit.clamp(1, 30)
        ))?;
        Ok(SearchResult {
            songs: parse_uploads(&v),
            ..Default::default()
        })
    }

    fn lyric(&self, _id: &TrackId) -> Result<Lyrics> {
        Ok(Lyrics::default())
    }

    fn stream_url(&self, id: &TrackId, _quality: &str) -> Result<Option<String>> {
        let v = http::get_json(&format!(
            "{}/api/query?f=json&ids={}",
            self.base,
            http::enc(id.as_str())
        ))?;
        // ccMixter 的 download_url 对非浏览器直链返回 403（实测），所以这里只认搜索到的直链，
        // 播放路径交给上层显示「无法获取播放链接」；不再假装能出流。
        let _ = stream_of(&v);
        Err(anyhow!(
            "ccMixter 禁止第三方直链下载（HTTP 403），暂不支持播放"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Value {
        json!([{
            "upload_id": 71331,
            "upload_name": "Legalize",
            "user_real_name": "MalreDeszik",
            "user_name": "malre",
            "files": [
                {"file_nicname":"mp3","download_url":"https://ccmixter.org/content/x/a.mp3",
                 "file_format_info":{"media-type":"audio","ps":"4:35"}},
                {"file_nicname":"Other.flac","download_url":"https://ccmixter.org/content/x/a.flac",
                 "file_format_info":{"media-type":"audio","ps":"4:35"}}
            ]
        }])
    }

    #[test]
    fn parses_uploads_into_songs() {
        let songs = parse_uploads(&sample());
        assert_eq!(songs.len(), 1);
        assert_eq!(songs[0].name, "Legalize");
        assert_eq!(songs[0].ar[0].name, "MalreDeszik");
        assert_eq!(songs[0].dt, (4 * 60 + 35) * 1000);
        assert_eq!(songs[0].s_id, "71331");
        assert_eq!(songs[0].source, "ccmixter");
    }

    #[test]
    fn picks_mp3_over_stems() {
        let u = stream_of(&sample()).unwrap();
        assert!(u.ends_with("a.mp3"), "{u}");
    }

    #[test]
    fn unreachable_base_degrades_to_error() {
        let s = CcMixterSource::with_base("http://127.0.0.1:9");
        assert!(!s.is_available());
        assert!(s.search("x", 5, SearchKind::Song).is_err());
    }

    #[test]
    #[ignore]
    fn ccmixter_live_search() {
        let s = CcMixterSource::new();
        let r = s.search("love", 3, SearchKind::Song).expect("真网搜索失败");
        println!("ccmixter_live_search: {} 条", r.songs.len());
        for song in r.songs.iter().take(3) {
            println!("  - {}  [{} ms]  s_id={}", song.name, song.dt, song.s_id);
        }
    }
}

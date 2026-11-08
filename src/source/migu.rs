//! 咪咕音乐源：走 app 侧的两个公开 GET 接口。
//!
//! 实测（2026-11）：
//! - 搜索：`app.c.nf.migu.cn/MIGUM2.0/v1.0/content/search_all.do` +
//!   `searchSwitch={"song":1}&text=<kw>` → `songResultData.result[]`
//!   有 `name` / `singers[]` / `albums[]` / `imgItems[]`，**没有时长**
//! - 取流：`app.c.nf.migu.cn/MIGUM2.0/v1.0/content/sub/listen.do` 必须带
//!   `resourceType=2`（传 3 会回「查询歌曲资源失败」）、`netType=01`、`channel=014015`、
//!   `toneFlag=PQ`，返回 `songListens[0].url`（`freetyst.nf.migu.cn` 的 mp3）
//! - 那条 mp3 实测 `200 audio/mpeg`、支持 Range、不需要 UA/Referer，可直接给 mpv
//!
//! 会员曲（VIP）会回「查询歌曲资源失败」，这里按「不可播放」降级返回 `Ok(None)`，
//! 不弹错误吓人。

use crate::models::{Album, Artist, Lyrics, SearchResult, Song};
use crate::source::http;
use crate::source::{Capabilities, SearchKind, Source, TrackId};
use anyhow::{anyhow, Context, Result};
use serde_json::Value;

const BASE: &str = "https://app.c.nf.migu.cn";
const CHANNEL: &str = "014015";

pub struct MiguSource {
    base: String,
}

impl MiguSource {
    pub fn new() -> Self {
        MiguSource {
            base: BASE.to_string(),
        }
    }

    #[cfg(test)]
    pub fn with_base(base: &str) -> Self {
        MiguSource {
            base: base.to_string(),
        }
    }
}

/// `Song.s_id` 里同时装 contentId 与 copyrightId（取流两个都要）。
fn pack_id(content_id: &str, copyright_id: &str) -> String {
    format!("{content_id}|{copyright_id}")
}

fn unpack_id(s: &str) -> Option<(&str, &str)> {
    let (c, cr) = s.split_once('|')?;
    if c.is_empty() || cr.is_empty() {
        return None;
    }
    Some((c, cr))
}

fn cover(it: &Value) -> Option<String> {
    let imgs = it.get("imgItems")?.as_array()?;
    imgs.iter()
        .find(|x| x.get("imgSizeType").and_then(|s| s.as_str()) == Some("03"))
        .or_else(|| imgs.last())
        .and_then(|x| x.get("img"))
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
}

/// 搜索返回 `songResultData.result[]` → `Song`。纯函数，方便单测。
pub fn parse_search(v: &Value, limit: usize) -> Vec<Song> {
    let Some(arr) = v
        .get("songResultData")
        .and_then(|d| d.get("result"))
        .and_then(|r| r.as_array())
    else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|it| {
            let content_id = it.get("contentId").and_then(|x| x.as_str())?;
            let copyright_id = it.get("copyrightId").and_then(|x| x.as_str()).unwrap_or("");
            let name = it.get("name").and_then(|x| x.as_str()).unwrap_or("");
            if name.is_empty() {
                return None;
            }
            // 咪咕搜索不给时长，留 0（UI 显示 --:--）
            let artist = it
                .get("singers")
                .and_then(|s| s.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.get("name").and_then(|n| n.as_str()))
                        .collect::<Vec<_>>()
                        .join(" / ")
                })
                .unwrap_or_default();
            let album = it
                .get("albums")
                .and_then(|a| a.as_array())
                .and_then(|a| a.first())
                .and_then(|x| x.get("name"))
                .and_then(|x| x.as_str())
                .unwrap_or("咪咕音乐")
                .to_string();
            Some(Song {
                id: 0,
                name: name.to_string(),
                ar: vec![Artist {
                    id: 0,
                    name: if artist.is_empty() {
                        "咪咕音乐".to_string()
                    } else {
                        artist
                    },
                }],
                al: Album {
                    id: 0,
                    name: album,
                    pic_url: cover(it),
                },
                dt: 0,
                fee: 0,
                source: "migu".to_string(),
                s_id: pack_id(content_id, copyright_id),
            })
        })
        .take(limit)
        .collect()
}

impl Source for MiguSource {
    fn id(&self) -> &'static str {
        "migu"
    }

    fn name(&self) -> &'static str {
        "咪咕音乐"
    }

    fn group(&self) -> &'static str {
        "国内"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            kinds: vec![SearchKind::Song],
            lyrics: false,
            qualities: Vec::new(),
        }
    }

    fn search(&self, keyword: &str, limit: i32, kind: SearchKind) -> Result<SearchResult> {
        if kind != SearchKind::Song {
            return Ok(SearchResult::default());
        }
        let limit = limit.clamp(1, 50) as usize;
        // searchSwitch 里只开歌曲
        let url = format!(
            "{}/MIGUM2.0/v1.0/content/search_all.do?isCorrect=1&pageNo=1&pageSize={}&searchSwitch=%7B%22song%22%3A1%7D&text={}",
            self.base,
            limit,
            http::enc(keyword)
        );
        let v = http::get_json(&url).context("咪咕搜索失败")?;
        if let Some(code) = v.get("code").and_then(|c| c.as_str()) {
            if code != "000000" {
                let msg = v.get("info").and_then(|m| m.as_str()).unwrap_or("");
                return Err(anyhow!("咪咕搜索返回 code={code} {msg}"));
            }
        }
        Ok(SearchResult {
            songs: parse_search(&v, limit),
            ..Default::default()
        })
    }

    fn lyric(&self, _id: &TrackId) -> Result<Lyrics> {
        Ok(Lyrics::default())
    }

    fn stream_url(&self, id: &TrackId, _quality: &str) -> Result<Option<String>> {
        let Some((content_id, copyright_id)) = unpack_id(id.as_str()) else {
            return Err(anyhow!("咪咕曲目 ID 格式不对: {}", id.as_str()));
        };
        // resourceType 必须是 2；3 会「查询歌曲资源失败」
        let url = format!(
            "{}/MIGUM2.0/v1.0/content/sub/listen.do?contentId={}&copyrightId={}&resourceType=2&formatType=PQ&netType=01&channel={}&toneFlag=PQ&scene=&userId=",
            self.base, content_id, copyright_id, CHANNEL
        );
        let v = http::get_json(&url).context("咪咕取流失败")?;
        let code = v.get("code").and_then(|c| c.as_str()).unwrap_or("");
        if code != "000000" {
            // 会员曲/下架：按不可播放降级，不报错
            return Ok(None);
        }
        Ok(v.get("songListens")
            .and_then(|x| x.as_array())
            .and_then(|a| a.first())
            .and_then(|x| x.get("url"))
            .and_then(|x| x.as_str())
            .filter(|u| u.starts_with("http"))
            .map(|u| u.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"code":"000000","info":"成功","songResultData":{"result":[
{"contentId":"600908000009519697","copyrightId":"6005970UMSQ","name":"小星星","singers":[{"id":"1","name":"胎教音乐"}],"albums":[{"id":"2","name":"摇篮曲"}],"imgItems":[{"imgSizeType":"01","img":"https://x/1.webp"},{"imgSizeType":"03","img":"https://x/3.webp"}]},
{"contentId":"600902000009118933","copyrightId":"60078701550","name":"小星星","singers":[{"name":"汪苏泷"},{"name":"某合唱"}],"albums":[{"name":"万有引力"}]},
{"name":"没有 contentId 的条目"}
]}}"#;

    #[test]
    fn parses_search_with_cover_artist_and_packed_id() {
        let v: Value = serde_json::from_str(SAMPLE).unwrap();
        let songs = parse_search(&v, 10);
        assert_eq!(songs.len(), 2, "没有 contentId 的条目要跳过");
        assert_eq!(songs[0].source, "migu");
        assert_eq!(songs[0].name, "小星星");
        assert_eq!(songs[0].ar[0].name, "胎教音乐");
        assert_eq!(songs[0].al.name, "摇篮曲");
        assert_eq!(songs[0].al.pic_url.as_deref(), Some("https://x/3.webp"));
        assert_eq!(songs[0].dt, 0, "咪咕搜索不给时长");
        assert_eq!(songs[0].s_id, "600908000009519697|6005970UMSQ");
        assert_eq!(songs[1].ar[0].name, "汪苏泷 / 某合唱", "多歌手用 / 连");
    }

    #[test]
    fn packs_and_unpacks_track_id() {
        assert_eq!(unpack_id(&pack_id("c1", "cr1")), Some(("c1", "cr1")));
        assert_eq!(unpack_id("no-separator"), None);
        assert_eq!(unpack_id("c1|"), None);
    }

    #[test]
    fn bad_track_id_errors_clearly() {
        let src = MiguSource::with_base("http://127.0.0.1:9");
        let err = src
            .stream_url(&TrackId::from("garbage"), "exhigh")
            .expect_err("ID 格式不对要报错");
        assert!(err.to_string().contains("格式不对"), "{err}");
    }

    #[test]
    fn unreachable_base_degrades_to_error() {
        let src = MiguSource::with_base("http://127.0.0.1:9");
        let err = src
            .search("晴天", 10, SearchKind::Song)
            .expect_err("接口不可达必须报错");
        assert!(err.to_string().contains("咪咕搜索失败"), "{err}");
    }

    /// 真联网冒烟（默认 ignore）：`cargo test --release migu_live -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn migu_live_search_and_stream() {
        let src = MiguSource::new();
        let got = src
            .search("小星星", 5, SearchKind::Song)
            .expect("搜索应成功");
        assert!(!got.songs.is_empty());
        for s in got.songs.iter().take(3) {
            eprintln!("{} | {} | {} | {}s", s.source, s.name, s.s_id, s.dt / 1000);
        }
        let mut ok = None;
        for s in got.songs.iter() {
            if let Ok(Some(u)) = src.stream_url(&crate::source::track_of(s), "exhigh") {
                ok = Some(u);
                break;
            }
        }
        let url = ok.expect("至少应有一首能取到流（会员曲会返回 None）");
        eprintln!("migu stream: {:.70}…", url);
    }
}

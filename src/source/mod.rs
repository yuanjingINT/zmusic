//! 音乐源抽象。
//!
//! 每个平台一个 adapter 实现 [`Source`]，App 只认 trait。
//! 这一版先把接口定型：ID 用字符串包装（YouTube 是 `dQw4w9WgXcQ` 这种）、
//! 搜索类型用枚举（不再把网易云的 1/100/1000/10 漏到上层）。

pub mod audius;
pub mod ccmixter;
pub mod http;
pub mod jamendo;
pub mod netease;
pub mod opengameart;
pub mod wikimedia;
pub mod youtube;

use crate::models::{Lyrics, SearchResult};
use anyhow::{anyhow, Result};
use std::sync::Arc;

/// 跨源通用的曲目 ID。网易云内部是数字，这里统一用字符串承载。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TrackId(pub String);

impl TrackId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for TrackId {
    fn from(s: String) -> Self {
        TrackId(s)
    }
}

impl From<&str> for TrackId {
    fn from(s: &str) -> Self {
        TrackId(s.to_string())
    }
}

impl std::fmt::Display for TrackId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// 搜索类型。各源自己映射到私有协议（网易云：1/100/1000/10）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchKind {
    Song,
    Artist,
    Playlist,
    Album,
}

impl SearchKind {
    /// UI 里的 0..3（歌曲/歌手/歌单/专辑）到枚举
    pub fn from_index(i: usize) -> Self {
        match i {
            1 => SearchKind::Artist,
            2 => SearchKind::Playlist,
            3 => SearchKind::Album,
            _ => SearchKind::Song,
        }
    }
}

/// 一个源支持什么：UI 依据它禁用不支持的搜索类型/音质，不靠猜。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capabilities {
    pub kinds: Vec<SearchKind>,
    pub lyrics: bool,
    /// 空 = 该源不区分音质
    pub qualities: Vec<&'static str>,
}

impl Capabilities {
    pub fn supports(&self, kind: SearchKind) -> bool {
        self.kinds.contains(&kind)
    }
}

/// 一个可搜索、可出流、可出歌词的音乐源。
///
/// App 目前只通过注册表的 `stream_url` 用到其中一部分；`search`/`lyric`/`name`
/// 等是给第二个源（YouTube/Jamendo）接入时用的，先标 dead_code。
#[allow(dead_code)]
pub trait Source: Send + Sync {
    /// 稳定的内部标识，配置与 UI 上都用它（如 `"netease"`）
    fn id(&self) -> &'static str;
    /// 给人看的名字
    fn name(&self) -> &'static str;
    /// 源当前能不能用（进程/依赖在不在）；缺依赖的源在 UI 上置灰，不影响其它源
    fn is_available(&self) -> bool {
        true
    }
    fn capabilities(&self) -> Capabilities;
    fn search(&self, keyword: &str, limit: i32, kind: SearchKind) -> Result<SearchResult>;
    fn lyric(&self, id: &TrackId) -> Result<Lyrics>;
    /// 取播放地址；拿不到（无版权/需会员/流过期）时返回 Ok(None)
    fn stream_url(&self, id: &TrackId, quality: &str) -> Result<Option<String>>;
}

/// 从 `Song` 得到跨源 ID：优先用 `s_id`，老数据没有就退回数字 id。
pub fn track_of(song: &crate::models::Song) -> TrackId {
    if song.s_id.is_empty() {
        TrackId(song.id.to_string())
    } else {
        TrackId(song.s_id.clone())
    }
}

/// 源注册表：按 id 取源，App 以后从这儿拿。
#[allow(dead_code)]
pub struct SourceRegistry {
    sources: Vec<Arc<dyn Source>>,
}

#[allow(dead_code)]
impl SourceRegistry {
    pub fn new() -> Self {
        SourceRegistry {
            sources: Vec::new(),
        }
    }

    /// 默认注册网易云 + YouTube（后者依赖 yt-dlp，缺了就自己标不可用）
    pub fn with_defaults(netease_base: String, proxy: Option<String>) -> Self {
        let mut reg = SourceRegistry::new();
        reg.register(Arc::new(netease::NetEaseSource::new(netease_base)));
        reg.register(Arc::new(audius::AudiusSource::new()));
        reg.register(Arc::new(ccmixter::CcMixterSource::new()));
        reg.register(Arc::new(wikimedia::WikimediaSource::new()));
        reg.register(Arc::new(jamendo::JamendoSource::new()));
        reg.register(Arc::new(opengameart::OpenGameArtSource::new()));
        let yt = match proxy {
            Some(p) => youtube::YouTubeSource::with_proxy("yt-dlp", Some(p)),
            None => youtube::YouTubeSource::new(),
        };
        reg.register(Arc::new(yt));
        reg
    }

    /// 按 source 标识搜索
    pub fn search(
        &self,
        source: &str,
        keyword: &str,
        limit: i32,
        kind: SearchKind,
    ) -> Result<SearchResult> {
        match self.get(source) {
            Some(s) => s.search(keyword, limit, kind),
            None => Err(anyhow!("未知音乐源: {}（没有对应的 adapter）", source)),
        }
    }

    pub fn register(&mut self, src: Arc<dyn Source>) {
        if !self.sources.iter().any(|s| s.id() == src.id()) {
            self.sources.push(src);
        }
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn Source>> {
        self.sources.iter().find(|s| s.id() == id).cloned()
    }

    pub fn all(&self) -> &[Arc<dyn Source>] {
        &self.sources
    }

    pub fn len(&self) -> usize {
        self.sources.len()
    }

    /// 按 source 标识取流；未知源给明确错误，别让上层拿到 None 猜
    pub fn stream_url(
        &self,
        source: &str,
        id: &TrackId,
        quality: &str,
    ) -> Result<Option<String>> {
        match self.get(source) {
            Some(s) => s.stream_url(id, quality),
            None => Err(anyhow!("未知音乐源: {}（没有对应的 adapter）", source)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Song;

    #[test]
    fn netease_registered_by_default() {
        let reg = SourceRegistry::with_defaults("http://127.0.0.1:3000".to_string(), None);
        assert!(reg.len() >= 2);
        let s = reg.get("netease").expect("默认应注册网易云");
        assert_eq!(s.id(), "netease");
        assert_eq!(s.name(), "网易云音乐");
        assert!(reg.get("youtube").is_some(), "YouTube 也应默认注册");
        assert!(reg.get("spotify").is_none());
    }

    #[test]
    fn duplicate_register_is_ignored() {
        let mut reg = SourceRegistry::with_defaults("http://127.0.0.1:3000".to_string(), None);
        reg.register(Arc::new(netease::NetEaseSource::new(
            "http://127.0.0.1:3000".to_string(),
        )));
        assert!(reg.len() >= 2);
    }

    #[test]
    fn unknown_source_stream_url_errors_clearly() {
        let reg = SourceRegistry::with_defaults("http://127.0.0.1:3000".to_string(), None);
        let err = reg
            .stream_url("spotify", &TrackId::from("dQw4w9WgXcQ"), "exhigh")
            .expect_err("未知源必须报错");
        let msg = err.to_string();
        assert!(msg.contains("未知音乐源"), "错误信息应说明未知源: {msg}");
        assert!(msg.contains("spotify"), "错误信息应带上源名: {msg}");
    }

    #[test]
    fn mixed_source_song_round_trips_through_json() {
        let s = Song {
            id: 0,
            name: "Never Gonna Give You Up".to_string(),
            ar: Vec::new(),
            al: Default::default(),
            dt: 213_000,
            fee: 0,
            source: "youtube".to_string(),
            s_id: "dQw4w9WgXcQ".to_string(),
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: Song = serde_json::from_str(&json).unwrap();
        assert_eq!(back.source, "youtube");
        assert_eq!(back.s_id, "dQw4w9WgXcQ");
        assert_eq!(back.name, s.name);
    }

    #[test]
    fn track_id_falls_back_to_numeric_id() {
        let mut song = Song::default();
        song.id = 42;
        assert_eq!(track_of(&song).as_str(), "42");
        song.s_id = "dQw4w9WgXcQ".to_string();
        assert_eq!(track_of(&song).as_str(), "dQw4w9WgXcQ");
    }

    #[test]
    fn search_kind_maps_from_ui_index() {
        assert_eq!(SearchKind::from_index(0), SearchKind::Song);
        assert_eq!(SearchKind::from_index(1), SearchKind::Artist);
        assert_eq!(SearchKind::from_index(2), SearchKind::Playlist);
        assert_eq!(SearchKind::from_index(3), SearchKind::Album);
        assert_eq!(SearchKind::from_index(99), SearchKind::Song);
    }
}

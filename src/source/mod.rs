//! 音乐源抽象（P1 第一步：先把网易云从 `api.rs` 里拆出来，行为不变）。
//!
//! 目标结构：每个平台一个 adapter 实现 [`Source`]，App 以后只认 trait，
//! 不在 UI 里写死某一家的接口。这一刀只搬代码 + 建骨架，不引新依赖、不改行为。

pub mod netease;

use crate::models::{Lyrics, SearchResult};
use anyhow::Result;
use std::sync::Arc;

/// 一个可搜索、可出流、可出歌词的音乐源。
///
/// P1 只用网易云；YouTube Music / SoundCloud（走 yt-dlp）与 Jamendo / Audius
/// 会在后续提交里各自实现这个 trait。
// P1 骨架：App 目前仍直接调 NetEaseSource 的专有接口（榜单/歌单/专辑等还没进 trait），
// 所以这几个 trait 方法暂时没有调用点；接第二个源时从这里开始用。
#[allow(dead_code)]
pub trait Source: Send + Sync {
    /// 稳定的内部标识，配置与 UI 上都用它（如 `"netease"`）
    fn id(&self) -> &'static str;
    /// 给人看的名字
    fn name(&self) -> &'static str;
    /// 搜索；`kind` 沿用网易云的 1/100/1000/10 语义（单曲/歌手/歌单/专辑）
    fn search(&self, keyword: &str, limit: i32, kind: i32) -> Result<SearchResult>;
    /// 单曲歌词
    fn lyric(&self, id: i64) -> Result<Lyrics>;
    /// 取播放地址；拿不到（无版权/需会员）时返回 Ok(None)
    fn stream_url(&self, id: i64, quality: &str) -> Result<Option<String>>;
}

/// 源注册表：按 id 取源，App 以后从这儿拿。
///
/// 目前只登记了网易云，`with_defaults` 之外没有调用点，所以标了 dead_code；
/// 接第二个源时它就会变成实际的入口。
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

    /// 目前只有网易云；`base` 是它的 API 地址
    pub fn with_defaults(netease_base: String) -> Self {
        let mut reg = SourceRegistry::new();
        reg.register(Arc::new(netease::NetEaseSource::new(netease_base)));
        reg
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn netease_registered_by_default() {
        let reg = SourceRegistry::with_defaults("http://127.0.0.1:3000".to_string());
        assert_eq!(reg.len(), 1);
        let s = reg.get("netease").expect("默认应注册网易云");
        assert_eq!(s.id(), "netease");
        assert_eq!(s.name(), "网易云音乐");
        assert!(reg.get("youtube").is_none());
    }

    #[test]
    fn duplicate_register_is_ignored() {
        let mut reg = SourceRegistry::with_defaults("http://127.0.0.1:3000".to_string());
        reg.register(Arc::new(netease::NetEaseSource::new(
            "http://127.0.0.1:3000".to_string(),
        )));
        assert_eq!(reg.len(), 1);
    }
}

//! 音乐源抽象。
//!
//! 每个平台一个 adapter 实现 [`Source`]，App 只认 trait。
//! 这一版先把接口定型：ID 用字符串包装（YouTube 是 `dQw4w9WgXcQ` 这种）、
//! 搜索类型用枚举（不再把网易云的 1/100/1000/10 漏到上层）。

pub mod agg;
pub mod archive;
pub mod audius;
pub mod ccmixter;
pub mod health;
pub mod http;
pub mod jamendo;
pub mod meting;
pub mod migu;
pub mod netease;
pub mod opengameart;
pub mod qq;
pub mod wikimedia;
pub mod ytdlp;

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
    /// 能不能出流。聚合只把 `true` 的源并进播放队列；
    /// 「能搜不能播」的源（例如没配 cookie 的 Meting）列表可见但标「仅搜索」。
    pub can_stream: bool,
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
    /// 分组（国内 / 国际 / 公开免费），S 循环与 UI 显示都按它来
    fn group(&self) -> &'static str {
        "其它"
    }
    fn capabilities(&self) -> Capabilities;
    fn search(&self, keyword: &str, limit: i32, kind: SearchKind) -> Result<SearchResult>;
    fn lyric(&self, id: &TrackId) -> Result<Lyrics>;
    /// 取播放地址；拿不到（无版权/需会员/流过期）时返回 Ok(None)
    fn stream_url(&self, id: &TrackId, quality: &str) -> Result<Option<String>>;
    /// 会不会 fork 子进程（目前只有 yt-dlp 系）：聚合时单独限流，避免 fork 风暴
    fn is_subprocess(&self) -> bool {
        false
    }

    /// 懒补时长：搜索接口不带时长的源（Meting）在选中/播放时问一次，默认不支持
    fn duration_ms(&self, _id: &TrackId) -> Result<Option<i64>> {
        Ok(None)
    }
}

/// 源优先级：数字小的排前面。`S` 循环顺序、聚合结果顺序、去重代表源都按它。
///
/// 网易云永远是第一（用户要求），其余按「国内 → 国际 → 公开免费」，
/// 国内内部顺序：QQ 直连 > Meting 系 > 咪咕。
pub fn source_priority(id: &str) -> u8 {
    match id {
        "netease" => 0,
        "qq" => 1,
        "meting:tencent" => 2,
        "meting:kugou" => 3,
        "meting:baidu" => 4,
        "meting:kuwo" => 5,
        "migu" => 6,
        "youtube" => 7,
        "soundcloud" => 8,
        "audius" => 9,
        "ccmixter" => 10,
        "jamendo" => 11,
        "wikimedia" => 12,
        "opengameart" => 13,
        "archive" => 14,
        _ => 99,
    }
}

/// 这个源在国内直连不通、必须走配置里的代理：搜索用 `http::` client 的
/// adapter（archive）自己会经桥走代理，mpv 播放时也得挂同一个出口。
pub fn needs_outbound_proxy(id: &str) -> bool {
    matches!(id, "youtube" | "soundcloud" | "archive")
}

/// 按优先级排序两个源 id（同优先级保持原有先后）
pub fn by_source_priority(a: &str, b: &str) -> std::cmp::Ordering {
    source_priority(a).cmp(&source_priority(b))
}

/// 内部源 id → 给人看的名字。列表/正在播放/i 弹窗都走它，未知 id 原样返回。
/// 全部内置源 id，顺序 = 注册顺序 = `S` 循环顺序 = 去重代表源优先级。
pub const ALL_SOURCE_IDS: &[&str] = &[
    "netease",
    "qq",
    "meting:tencent",
    "meting:kugou",
    "meting:baidu",
    "meting:kuwo",
    "migu",
    "youtube",
    "soundcloud",
    "audius",
    "ccmixter",
    "jamendo",
    "wikimedia",
    "opengameart",
    "archive",
];

/// 把用户输入展开成具体 id：支持 `meting`（四个 meting 源）和 `all`。
/// 返回空 Vec 表示有无法识别的 id（调用方负责报错）。
pub fn expand_source_ids(args: &[String]) -> (Vec<String>, Vec<String>) {
    let mut ok: Vec<String> = Vec::new();
    let mut bad: Vec<String> = Vec::new();
    for a in args {
        let a = a.trim();
        match a {
            "meting" => {
                for id in ALL_SOURCE_IDS.iter().filter(|i| i.starts_with("meting:")) {
                    if !ok.contains(&id.to_string()) {
                        ok.push(id.to_string());
                    }
                }
            }
            "all" => {
                for id in ALL_SOURCE_IDS {
                    if !ok.contains(&id.to_string()) {
                        ok.push(id.to_string());
                    }
                }
            }
            other if ALL_SOURCE_IDS.contains(&other) => {
                if !ok.contains(&other.to_string()) {
                    ok.push(other.to_string());
                }
            }
            other => bad.push(other.to_string()),
        }
    }
    (ok, bad)
}

pub fn source_display(id: &str) -> String {
    source_display_lang(id, !crate::i18n::en())
}

/// 同 [`source_display`]，但语言由参数给（测试要一次性验中英两版）
pub fn source_display_lang(id: &str, zh: bool) -> String {
    match id {
        "netease" => if zh { "网易云" } else { "NetEase" }.to_string(),
        "qq" => if zh { "QQ(直连)" } else { "QQ (direct)" }.to_string(),
        "meting:tencent" => if zh { "QQ(Meting)" } else { "QQ (Meting)" }.to_string(),
        "meting:kugou" => if zh { "酷狗" } else { "Kugou" }.to_string(),
        "meting:baidu" => if zh { "千度" } else { "Baidu" }.to_string(),
        "meting:kuwo" => if zh { "酷我" } else { "Kuwo" }.to_string(),
        "youtube" => "YouTube".to_string(),
        "soundcloud" => "SoundCloud".to_string(),
        "migu" => if zh { "咪咕" } else { "Migu" }.to_string(),
        "audius" => "Audius".to_string(),
        "ccmixter" => "ccMixter".to_string(),
        "wikimedia" => "Wikimedia".to_string(),
        "jamendo" => "Jamendo".to_string(),
        "opengameart" => "OpenGameArt".to_string(),
        "archive" => "Internet Archive".to_string(),
        other => other.to_string(),
    }
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
    /// 探活结果缓存：网络型源探一次要几百毫秒，绝不能每帧/每次按键都探
    avail: std::sync::Mutex<std::collections::HashMap<String, (std::time::Instant, bool)>>,
}

#[allow(dead_code)]
impl SourceRegistry {
    pub fn new() -> Self {
        SourceRegistry {
            sources: Vec::new(),
            avail: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// 带 60s 缓存的可用性判断
    pub fn available(&self, id: &str) -> bool {
        const TTL: std::time::Duration = std::time::Duration::from_secs(60);
        if let Some((at, ok)) = self.avail.lock().unwrap().get(id) {
            if at.elapsed() < TTL {
                return *ok;
            }
        }
        let ok = self.get(id).map(|s| s.is_available()).unwrap_or(false);
        self.avail
            .lock()
            .unwrap()
            .insert(id.to_string(), (std::time::Instant::now(), ok));
        ok
    }

    /// 默认注册全部源；配置（config.toml / 环境变量）由调用方传进来
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    pub fn with_defaults(
        netease_base: String,
        proxy: Option<String>,
        meting_base: String,
        ytdlp_bin: String,
        meting_cookie: Option<String>,
        jamendo_client_id: Option<String>,
        enabled: &[String],
    ) -> Self {
        let mut reg = SourceRegistry::new();
        // 只注册启用的源：默认配置下只有网易云，其余由 `zmuse source add` 打开。
        let on = |id: &str| enabled.iter().any(|e| e == id);
        let want = |id: &str| {
            if on(id) {
                Some(())
            } else {
                None
            }
        };
        let _ = &want;
        // 注册顺序 = `S` 循环顺序 = 去重代表源的优先级：网易云钉死第一，
        // 然后国内（QQ 直连 → Meting 系 → 咪咕）→ 国际 → 公开免费
        if on("netease") {
            reg.register(Arc::new(netease::NetEaseSource::new(netease_base)));
        }
        if on("qq") {
            reg.register(Arc::new(qq::QqSource::new()));
        }
        for server in ["tencent", "kugou", "baidu", "kuwo"] {
            if on(&format!("meting:{server}")) {
                reg.register(Arc::new(meting::MetingSource::with_cookie(
                    meting_base.clone(),
                    server,
                    meting_cookie.clone(),
                )));
            }
        }
        if on("migu") {
            reg.register(Arc::new(migu::MiguSource::new()));
        }
        // 国际
        // 走 yt-dlp 的一组站点（同一套逻辑按 Site 参数化）
        for (site, id) in [
            (ytdlp::Site::YouTube, "youtube"),
            (ytdlp::Site::SoundCloud, "soundcloud"),
        ] {
            if on(id) {
                let src = match &proxy {
                    Some(p) => ytdlp::YtDlpSource::with_proxy(site, &ytdlp_bin, Some(p.clone())),
                    None => ytdlp::YtDlpSource::with_proxy(site, &ytdlp_bin, None),
                };
                reg.register(Arc::new(src));
            }
        }
        // 公开免费
        if on("audius") {
            reg.register(Arc::new(audius::AudiusSource::new()));
        }
        if on("ccmixter") {
            reg.register(Arc::new(ccmixter::CcMixterSource::new()));
        }
        if on("jamendo") {
            reg.register(Arc::new(jamendo::JamendoSource::with_client_id(
                jamendo_client_id,
            )));
        }
        if on("wikimedia") {
            reg.register(Arc::new(wikimedia::WikimediaSource::new()));
        }
        if on("opengameart") {
            reg.register(Arc::new(opengameart::OpenGameArtSource::new()));
        }
        if on("archive") {
            reg.register(Arc::new(archive::ArchiveSource::new(proxy.clone())));
        }
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

    /// 按 source 标识补时长（拿不到就 None，不报错）
    pub fn duration_ms(&self, source: &str, id: &TrackId) -> Result<Option<i64>> {
        match self.get(source) {
            Some(s) => s.duration_ms(id),
            None => Ok(None),
        }
    }

    /// 按 source 标识取流；未知源给明确错误，别让上层拿到 None 猜
    pub fn stream_url(&self, source: &str, id: &TrackId, quality: &str) -> Result<Option<String>> {
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
        let reg = SourceRegistry::with_defaults(
            "http://127.0.0.1:3000".to_string(),
            None,
            "http://127.0.0.1:8123".to_string(),
            "yt-dlp".to_string(),
            None,
            None,
            &all_ids(),
        );
        assert!(reg.len() >= 2);
        let s = reg.get("netease").expect("默认应注册网易云");
        assert_eq!(s.id(), "netease");
        assert_eq!(s.name(), "网易云音乐");
        assert!(reg.get("youtube").is_some(), "YouTube 也应默认注册");
        assert!(reg.get("soundcloud").is_some(), "SoundCloud 应默认注册");
        assert!(reg.get("migu").is_some(), "咪咕应默认注册");
        assert!(reg.get("bandcamp").is_none(), "Bandcamp 无搜索入口，不注册");
        assert!(reg.get("spotify").is_none());
    }

    #[test]
    fn duplicate_register_is_ignored() {
        let mut reg = SourceRegistry::with_defaults(
            "http://127.0.0.1:3000".to_string(),
            None,
            "http://127.0.0.1:8123".to_string(),
            "yt-dlp".to_string(),
            None,
            None,
            &all_ids(),
        );
        reg.register(Arc::new(netease::NetEaseSource::new(
            "http://127.0.0.1:3000".to_string(),
        )));
        assert!(reg.len() >= 2);
    }

    #[test]
    fn unknown_source_stream_url_errors_clearly() {
        let reg = SourceRegistry::with_defaults(
            "http://127.0.0.1:3000".to_string(),
            None,
            "http://127.0.0.1:8123".to_string(),
            "yt-dlp".to_string(),
            None,
            None,
            &all_ids(),
        );
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
        let mut song = Song {
            id: 42,
            ..Default::default()
        };
        assert_eq!(track_of(&song).as_str(), "42");
        song.s_id = "dQw4w9WgXcQ".to_string();
        assert_eq!(track_of(&song).as_str(), "dQw4w9WgXcQ");
    }

    #[test]
    fn registry_order_puts_netease_first_and_follows_groups() {
        let reg = SourceRegistry::with_defaults(
            "http://127.0.0.1:3000".to_string(),
            None,
            "http://127.0.0.1:8123".to_string(),
            "yt-dlp".to_string(),
            None,
            None,
            &all_ids(),
        );
        let ids: Vec<&str> = reg.all().iter().map(|s| s.id()).collect();
        assert_eq!(ids[0], "netease", "S 循环第一个必须是网易云");
        // 注册顺序和优先级表一致
        let mut sorted = ids.clone();
        sorted.sort_by(|a, b| by_source_priority(a, b));
        assert_eq!(ids, sorted, "注册顺序要就是优先级顺序");
    }

    #[test]
    fn priorities_follow_the_agreed_table() {
        let order = [
            "netease",
            "qq",
            "meting:tencent",
            "meting:kugou",
            "migu",
            "youtube",
            "soundcloud",
            "audius",
            "ccmixter",
            "jamendo",
            "wikimedia",
            "opengameart",
            "archive",
        ];
        // 只断言相对顺序：表里还有 baidu/kuwo 两个不在 Chief 列表里的源
        for w in order.windows(2) {
            assert!(
                source_priority(w[0]) < source_priority(w[1]),
                "{} 应排在 {} 前面",
                w[0],
                w[1]
            );
        }
        assert_eq!(source_priority("netease"), 0);
        assert_eq!(source_priority("whatever-new"), 99);
    }

    /// 默认配置只启用网易云：其它源不注册（探测/列表/计数都跟着它走）
    #[test]
    fn default_config_registers_netease_only() {
        let reg = SourceRegistry::with_defaults(
            "http://127.0.0.1:3000".to_string(),
            None,
            "http://127.0.0.1:8123".to_string(),
            "yt-dlp".to_string(),
            None,
            None,
            &["netease".to_string()],
        );
        assert_eq!(reg.len(), 1, "默认只该有网易云");
        assert!(reg.get("netease").is_some());
        assert!(reg.get("youtube").is_none());
        assert!(reg.get("meting:tencent").is_none());
    }

    /// `meting` 分组能一次展开成四个源；`all` 是全部；未知 id 报回来
    #[test]
    fn source_id_aliases_expand() {
        let (ok, bad) = expand_source_ids(&["meting".to_string()]);
        assert_eq!(ok.len(), 4, "{ok:?}");
        assert!(bad.is_empty());

        let (ok, _) = expand_source_ids(&["all".to_string()]);
        assert_eq!(ok.len(), ALL_SOURCE_IDS.len());

        let (ok, bad) = expand_source_ids(&["netease".to_string(), "nope".to_string()]);
        assert_eq!(ok, vec!["netease".to_string()]);
        assert_eq!(bad, vec!["nope".to_string()]);
    }

    /// 测试用：全部源都启用（改名后默认只开网易云，行为测试要按全量跑）
    fn all_ids() -> Vec<String> {
        crate::source::ALL_SOURCE_IDS
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn source_display_covers_every_registered_source() {
        let cases = [
            ("netease", "网易云"),
            ("qq", "QQ(直连)"),
            ("meting:tencent", "QQ(Meting)"),
            ("meting:kugou", "酷狗"),
            ("meting:baidu", "千度"),
            ("meting:kuwo", "酷我"),
            ("youtube", "YouTube"),
            ("soundcloud", "SoundCloud"),
            ("migu", "咪咕"),
            ("audius", "Audius"),
            ("ccmixter", "ccMixter"),
            ("wikimedia", "Wikimedia"),
            ("jamendo", "Jamendo"),
            ("opengameart", "OpenGameArt"),
            ("archive", "Internet Archive"),
        ];
        for (id, want) in cases {
            assert_eq!(source_display(id), want, "id={id}");
        }
        assert_eq!(source_display("some-new-source"), "some-new-source");
        assert_eq!(source_display(""), "");

        // 英文版不许出现汉字，且每个源都得有名字
        for id in [
            "netease",
            "qq",
            "meting:tencent",
            "meting:kugou",
            "meting:baidu",
            "meting:kuwo",
            "youtube",
            "soundcloud",
            "migu",
            "audius",
            "ccmixter",
            "wikimedia",
            "jamendo",
            "opengameart",
            "archive",
        ] {
            let en = source_display_lang(id, false);
            assert!(!en.is_empty(), "{id} 英文名为空");
            assert!(
                !en.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)),
                "{id} 英文名混了中文: {en}"
            );
        }
    }

    #[test]
    fn outbound_proxy_needed_only_for_blocked_sources() {
        assert!(needs_outbound_proxy("archive"));
        assert!(needs_outbound_proxy("youtube"));
        assert!(needs_outbound_proxy("soundcloud"));
        assert!(!needs_outbound_proxy("netease"));
        assert!(!needs_outbound_proxy("audius"));
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

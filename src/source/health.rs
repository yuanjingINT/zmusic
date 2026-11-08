//! 源健康探测：状态机、并发探测、聚合结果合并。
//!
//! 约束（来自验收要求）：
//! - 探测全在后台线程，**绝不进渲染/按键路径**；UI 只读缓存状态
//! - 状态四态：`Unknown / Probing / Ok / Down / NeedsCreds`（缺 key、侧车没起单独区分）
//! - 结果带 TTL（默认 10 分钟），过期回到 `Unknown`，`R` 键手动重探
//! - 聚合搜索并发上限 4、每源 5s；超时的源记进「失败列表」，不影响其它源

use std::collections::{HashMap, VecDeque};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::source::Source;

/// 探测结果 TTL
pub const DEFAULT_TTL: Duration = Duration::from_secs(600);
/// 聚合/探测的并发上限
pub const CONCURRENCY: usize = 4;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SourceState {
    Unknown,
    Probing,
    Ok,
    /// 网络/接口挂了
    Down,
    /// 缺凭据：key 没配、侧车没起
    NeedsCreds,
}

impl SourceState {
    pub fn is_ok(self) -> bool {
        matches!(self, SourceState::Ok)
    }
}

/// 错误文本 → 状态。缺 key / 侧车没起归 `NeedsCreds`，其余统一 `Down`。
///
/// 这些字符串来自各 adapter 的报错（`Jamendo 需要 ZMUSIC_JAMENDO_CLIENT_ID`、
/// `meting 侧车未启动` 等），改文案时记得同步这里；匹配不到就按 `Down` 处理，宁严不宽。
pub fn classify_error(msg: &str) -> SourceState {
    let m = msg.to_ascii_lowercase();
    let needs = [
        "需要 zmusic_jamendo_client_id",
        "client_id",
        "侧车",
        "sidecar",
        "未启动",
        "token",
        "api key",
        "apikey",
        "需要登录",
        "cookie",
    ];
    if needs.iter().any(|k| m.contains(k)) {
        SourceState::NeedsCreds
    } else {
        SourceState::Down
    }
}

/// 探测一个源：先看 `is_available()`（各 adapter 已实现成最小请求），
/// 缺凭据的源按 id 兜底标 `NeedsCreds`。
pub fn probe_one(src: &dyn Source) -> SourceState {
    if src.is_available() {
        return SourceState::Ok;
    }
    if needs_creds_hint(src.id()) {
        SourceState::NeedsCreds
    } else {
        SourceState::Down
    }
}

/// 本地已知的「缺凭据」判据：只有确实要 key 的源才这么标
pub fn needs_creds_hint(id: &str) -> bool {
    match id {
        "jamendo" => std::env::var("ZMUSIC_JAMENDO_CLIENT_ID").is_err(),
        _ if id.starts_with("meting:") => true, // 侧车没起 / 需要 cookie
        _ => false,
    }
}

/// 带 TTL 的状态表。UI 每帧只读它，不做任何网络。
pub struct HealthTable {
    map: HashMap<String, (SourceState, Instant)>,
    ttl: Duration,
}

impl HealthTable {
    pub fn new(ttl: Duration) -> Self {
        HealthTable {
            map: HashMap::new(),
            ttl,
        }
    }

    pub fn set(&mut self, id: &str, state: SourceState) {
        self.map.insert(id.to_string(), (state, Instant::now()));
    }

    pub fn state(&self, id: &str) -> SourceState {
        match self.map.get(id) {
            Some((st, at)) if at.elapsed() < self.ttl => *st,
            Some(_) => SourceState::Unknown,
            None => SourceState::Unknown,
        }
    }

    pub fn ok_count(&self) -> usize {
        self.map
            .values()
            .filter(|(st, at)| *st == SourceState::Ok && at.elapsed() < self.ttl)
            .count()
    }

    pub fn probing_count(&self) -> usize {
        self.map
            .values()
            .filter(|(st, at)| *st == SourceState::Probing && at.elapsed() < self.ttl)
            .count()
    }
}

/// 并发探测：`concurrency` 个 worker 从同一个队列取源。返回全部结果。
///
/// 探测本身无法取消（reqwest 有自己的超时），所以调用方用 `recv_timeout` 卡总时长，
/// 超时没回来的源由调用方自己标 `Down`；迟到的结果仍可覆盖。
/// 并发探测，**每探到一个就回调**，给 UI 做「探测中 3/16」的进度。
pub fn probe_all_with<F: FnMut(String, SourceState) + Send>(
    sources: Vec<(String, Arc<dyn Source>)>,
    concurrency: usize,
    mut on_result: F,
) {
    let n = sources.len();
    let queue = Arc::new(Mutex::new(sources.into_iter().collect::<VecDeque<_>>()));
    let (tx, rx) = mpsc::channel::<(String, SourceState)>();
    let workers = concurrency.clamp(1, n.max(1));
    for _ in 0..workers {
        let queue = queue.clone();
        let tx = tx.clone();
        std::thread::spawn(move || loop {
            let item = { queue.lock().unwrap().pop_front() };
            let Some((id, src)) = item else { break };
            let st = probe_one(src.as_ref());
            if tx.send((id, st)).is_err() {
                break;
            }
        });
    }
    drop(tx);
    for _ in 0..n {
        match rx.recv() {
            Ok((id, st)) => on_result(id, st),
            Err(_) => break,
        }
    }
}

/// 聚合结果合并：`名称+主歌手` 去重，**代表源取优先级高的那条**（后到的网易云会顶掉先到的咪咕）。
///
/// `index` 是 key → dst 下标的映射，由调用方持有，便于增量合并。
pub fn merge_songs_priority(
    dst: &mut Vec<crate::models::Song>,
    index: &mut HashMap<String, usize>,
    part: &[crate::models::Song],
) {
    for s in part {
        let key = dedup_key(s);
        match index.get(&key) {
            Some(&i) => {
                if crate::source::source_priority(&s.source)
                    < crate::source::source_priority(&dst[i].source)
                {
                    dst[i] = s.clone();
                }
            }
            None => {
                index.insert(key, dst.len());
                dst.push(s.clone());
            }
        }
    }
}

/// 聚合结果按源优先级稳定排序：同源内保持接口返回顺序。
pub fn sort_by_priority(songs: &mut [crate::models::Song]) {
    songs.sort_by(|a, b| crate::source::by_source_priority(&a.source, &b.source));
}

/// 去重键：名称 + 主歌手，忽略大小写/空白/常见标点，避免 "晴天" 与 "晴天 " 各占一条
pub fn dedup_key(s: &crate::models::Song) -> String {
    let artist = s.ar.first().map(|a| a.name.as_str()).unwrap_or("");
    format!("{}|{}", normalize(&s.name), normalize(artist))
}

fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| {
            !c.is_whitespace()
                && !matches!(
                    c,
                    '（' | '）' | '(' | ')' | '[' | ']' | '【' | '】' | '-' | '·' | '・'
                )
        })
        .flat_map(|c| c.to_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Album, Artist, Lyrics, SearchResult, Song};
    use crate::source::{Capabilities, SearchKind, TrackId};

    struct Fake {
        id: &'static str,
        ok: bool,
        sleep_ms: u64,
        songs: Vec<Song>,
    }

    impl Source for Fake {
        fn id(&self) -> &'static str {
            self.id
        }
        fn name(&self) -> &'static str {
            self.id
        }
        fn is_available(&self) -> bool {
            self.ok
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities {
                kinds: vec![SearchKind::Song],
                lyrics: false,
                qualities: Vec::new(),
                can_stream: true,
            }
        }
        fn search(&self, _k: &str, _l: i32, _kind: SearchKind) -> anyhow::Result<SearchResult> {
            std::thread::sleep(Duration::from_millis(self.sleep_ms));
            Ok(SearchResult {
                songs: self.songs.clone(),
                ..Default::default()
            })
        }
        fn lyric(&self, _id: &TrackId) -> anyhow::Result<Lyrics> {
            Ok(Lyrics::default())
        }
        fn stream_url(&self, _id: &TrackId, _q: &str) -> anyhow::Result<Option<String>> {
            Ok(None)
        }
    }

    fn song(name: &str, artist: &str, source: &str) -> Song {
        Song {
            id: 0,
            name: name.to_string(),
            ar: vec![Artist {
                id: 0,
                name: artist.to_string(),
            }],
            al: Album {
                id: 0,
                name: source.to_string(),
                pic_url: None,
            },
            dt: 0,
            fee: 0,
            source: source.to_string(),
            s_id: name.to_string(),
        }
    }

    #[test]
    fn classifies_errors_into_creds_vs_down() {
        assert_eq!(
            classify_error("Jamendo 需要 ZMUSIC_JAMENDO_CLIENT_ID"),
            SourceState::NeedsCreds
        );
        assert_eq!(
            classify_error("meting 侧车未启动（http://127.0.0.1:8123）"),
            SourceState::NeedsCreds
        );
        assert_eq!(
            classify_error("请求失败: connection refused"),
            SourceState::Down
        );
        assert_eq!(classify_error(""), SourceState::Down);
    }

    #[test]
    fn ttl_expires_back_to_unknown() {
        let mut t = HealthTable::new(Duration::from_millis(30));
        t.set("netease", SourceState::Ok);
        assert_eq!(t.state("netease"), SourceState::Ok);
        assert_eq!(t.ok_count(), 1);
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(t.state("netease"), SourceState::Unknown);
        assert_eq!(t.ok_count(), 0);
    }

    #[test]
    fn probe_all_marks_each_source_and_tolerates_failures() {
        let srcs: Vec<(String, Arc<dyn Source>)> = vec![
            (
                "a".to_string(),
                Arc::new(Fake {
                    id: "a",
                    ok: true,
                    sleep_ms: 0,
                    songs: vec![],
                }),
            ),
            (
                "b".to_string(),
                Arc::new(Fake {
                    id: "b",
                    ok: false,
                    sleep_ms: 0,
                    songs: vec![],
                }),
            ),
        ];
        let mut got = Vec::new();
        probe_all_with(srcs, 4, |id, st| got.push((id, st)));
        got.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], ("a".to_string(), SourceState::Ok));
        assert_eq!(got[1], ("b".to_string(), SourceState::Down));
    }

    #[test]
    fn merge_keeps_highest_priority_representative() {
        let mut dst = Vec::new();
        let mut index = HashMap::new();
        // 咪咕先到
        merge_songs_priority(&mut dst, &mut index, &[song("晴天", "周杰伦", "migu")]);
        assert_eq!(dst[0].source, "migu");
        // 网易云后到：优先级更高 → 顶掉代表源
        merge_songs_priority(&mut dst, &mut index, &[song("晴天", "周杰伦", "netease")]);
        assert_eq!(dst.len(), 1, "同一首只留一行");
        assert_eq!(dst[0].source, "netease", "代表源取优先级高的");
        // 低优先级的后到不顶替
        merge_songs_priority(&mut dst, &mut index, &[song("晴天", "周杰伦", "audius")]);
        assert_eq!(dst[0].source, "netease");
        // 同名不同歌手各留一行
        merge_songs_priority(&mut dst, &mut index, &[song("晴天", "关诗敏", "qq")]);
        assert_eq!(dst.len(), 2);
    }

    #[test]
    fn sort_puts_netease_first_and_keeps_same_source_order() {
        let mut v = vec![
            song("a1", "x", "audius"),
            song("b1", "x", "bilibili"),
            song("n1", "x", "netease"),
            song("a2", "x", "audius"),
            song("n2", "x", "netease"),
            song("q1", "x", "qq"),
        ];
        sort_by_priority(&mut v);
        let order: Vec<&str> = v.iter().map(|s| s.source.as_str()).collect();
        assert_eq!(
            order,
            ["netease", "netease", "qq", "bilibili", "audius", "audius"],
            "网易云在最前，同源内保持原顺序"
        );
        assert_eq!(v[0].name, "n1");
        assert_eq!(v[1].name, "n2");
    }

    #[test]
    fn dedup_key_ignores_brackets_and_case() {
        let a = song("Get Lucky (Radio Edit)", "Daft Punk", "youtube");
        let b = song("get lucky radio edit", "DAFT PUNK", "soundcloud");
        assert_eq!(dedup_key(&a), dedup_key(&b));
    }
}

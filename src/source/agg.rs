//! 聚合搜索的执行器：并发拉满、按源限流、60s 结果缓存，并给出可测的耗时统计。
//!
//! UI 只负责把 `Part` 塞进 `AppMsg`；真正的调度在这里，方便单测/压测复用同一份代码。
//! **渲染路径不经过这里**——它只在用户按 Enter 时由后台线程调用。

use crate::models::SearchResult;
use crate::source::{SearchKind, Source};
use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// 一次聚合搜索的参数
#[derive(Debug, Clone, Copy)]
pub struct Config {
    /// 普通 HTTP 源的并发上限
    pub workers: usize,
    /// 要 fork 子进程的源（yt-dlp 系）单独限流，避免 fork 风暴
    pub subprocess_workers: usize,
    /// 到点就截止，没回来的算超时
    pub timeout: Duration,
    /// 是否启用 (source, query) 60s 缓存
    pub use_cache: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            workers: 24,
            subprocess_workers: 2,
            timeout: Duration::from_secs(5),
            use_cache: true,
        }
    }
}

#[derive(Debug)]
pub struct Part {
    pub id: String,
    /// 从开始到这条回来的耗时（UI 暂不用，压测/排查时看）
    #[allow(dead_code)]
    pub elapsed: Duration,
    pub result: Result<SearchResult, String>,
}

#[derive(Debug, Default)]
pub struct Stats {
    /// 第一个「有歌的」结果到达时间（TTFB）
    pub first_result: Option<Duration>,
    /// 截止/全部返回的时间
    pub finished: Duration,
    pub failed: usize,
    pub timed_out: usize,
    /// 超时没回来的源 id（UI 状态行要列出来）
    pub timed_out_ids: Vec<String>,
    pub failed_ids: Vec<String>,
}

const CACHE_TTL: Duration = Duration::from_secs(60);

/// (source, query) → 上次成功结果
pub struct Cache {
    map: Mutex<HashMap<(String, String), (Instant, SearchResult)>>,
}

impl Cache {
    fn new() -> Self {
        Cache {
            map: Mutex::new(HashMap::new()),
        }
    }

    pub fn get(&self, source: &str, query: &str) -> Option<SearchResult> {
        let map = self.map.lock().unwrap();
        let (at, v) = map.get(&(source.to_string(), query.to_string()))?;
        if at.elapsed() < CACHE_TTL {
            Some(v.clone())
        } else {
            None
        }
    }

    pub fn put(&self, source: &str, query: &str, v: &SearchResult) {
        self.map.lock().unwrap().insert(
            (source.to_string(), query.to_string()),
            (Instant::now(), v.clone()),
        );
    }

    pub fn clear(&self) {
        self.map.lock().unwrap().clear();
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.map.lock().unwrap().len()
    }
}

pub fn cache() -> &'static Cache {
    static C: OnceLock<Cache> = OnceLock::new();
    C.get_or_init(Cache::new)
}

pub fn clear_cache() {
    cache().clear();
}

/// 丢掉没有时长的曲目：列表里没有进度条的行，点了才知道能不能放，不如不显示。
/// （懒补时长补完仍为 0 的同样会被丢掉）
pub fn retain_timed(songs: &mut Vec<crate::models::Song>) {
    songs.retain(|s| s.dt > 0);
}

/// 播放候选排序：同一首歌在各源的条目按源优先级排好，去掉 (source, s_id) 重复。
pub fn order_candidates(list: &[crate::models::Song]) -> Vec<crate::models::Song> {
    let mut out: Vec<crate::models::Song> = Vec::with_capacity(list.len());
    for s in list {
        if !out.iter().any(|x| x.source == s.source && x.s_id == s.s_id) {
            out.push(s.clone());
        }
    }
    out.sort_by(|a, b| crate::source::by_source_priority(&a.source, &b.source));
    out
}

/// 按优先级逐个试流：返回第一个成功的 (url, 用的那条候选) 和所有失败原因。
///
/// `try_stream` 由调用方提供（UI 里是 `registry.stream_url`），方便单测不碰网络。
/// 试流结果：成功的 (url, 用到的候选)；失败原因列表 (显示名, 原因)
pub type PlayAttempt = (Option<(String, crate::models::Song)>, Vec<(String, String)>);

pub fn first_playable<F>(candidates: &[crate::models::Song], mut try_stream: F) -> PlayAttempt
where
    F: FnMut(&crate::models::Song) -> Result<Option<String>, String>,
{
    let mut failures = Vec::new();
    for cand in candidates {
        let label = crate::source::source_display(&cand.source);
        match try_stream(cand) {
            Ok(Some(url)) => return (Some((url, cand.clone())), failures),
            Ok(None) => failures.push((label, "需会员或无版权".to_string())),
            Err(e) => failures.push((label, e)),
        }
    }
    (None, failures)
}

/// 跑一次聚合搜索：每个源的结果到达就回调一次；到截止时间把没回来的记超时。
pub fn run<F: FnMut(Part)>(
    targets: Vec<(String, Arc<dyn Source>)>,
    query: &str,
    limit: i32,
    cfg: Config,
    mut on_part: F,
) -> Stats {
    let start = Instant::now();
    let mut stats = Stats::default();
    if targets.is_empty() {
        return stats;
    }
    let total = targets.len();
    let all_ids: Vec<String> = targets.iter().map(|(id, _)| id.clone()).collect();

    // 子进程源单独排队限流
    let (mut sub, mut plain): (Vec<_>, Vec<_>) =
        targets.into_iter().partition(|(_, s)| s.is_subprocess());

    let (tx, rx) = mpsc::channel::<(String, Result<SearchResult, String>)>();
    let spawn_pool = |items: &mut Vec<(String, Arc<dyn Source>)>, workers: usize| {
        if items.is_empty() {
            return;
        }
        let queue = Arc::new(Mutex::new(std::mem::take(items)));
        for _ in 0..workers.max(1) {
            let queue = queue.clone();
            let tx = tx.clone();
            let q = query.to_string();
            std::thread::spawn(move || loop {
                let item = { queue.lock().unwrap().pop() };
                let Some((id, src)) = item else { break };
                let r = match (cfg.use_cache, !src.is_subprocess()) {
                    (true, true) => match cache().get(&id, &q) {
                        Some(hit) => Ok(SearchResult {
                            songs: hit.songs,
                            ..Default::default()
                        }),
                        None => match src.search(&q, limit, SearchKind::Song) {
                            Ok(v) => {
                                cache().put(&id, &q, &v);
                                Ok(v)
                            }
                            Err(e) => Err(e.to_string()),
                        },
                    },
                    _ => src
                        .search(&q, limit, SearchKind::Song)
                        .map_err(|e| e.to_string()),
                };
                let mut r = r;
                if let Ok(v) = r.as_mut() {
                    retain_timed(&mut v.songs);
                }
                if tx.send((id, r)).is_err() {
                    break;
                }
            });
        }
    };
    let plain_workers = cfg.workers.min(plain.len()).max(1);
    let sub_workers = cfg.subprocess_workers.min(sub.len()).max(1);
    spawn_pool(&mut plain, plain_workers);
    spawn_pool(&mut sub, sub_workers);
    drop(tx);

    let deadline = start + cfg.timeout;
    let mut answered = 0usize;
    let mut answered_ids: Vec<String> = Vec::with_capacity(total);
    while answered < total {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        match rx.recv_timeout(left) {
            Ok((id, r)) => {
                answered += 1;
                answered_ids.push(id.clone());
                let elapsed = start.elapsed();
                if stats.first_result.is_none() {
                    if let Ok(ref v) = r {
                        if !v.songs.is_empty() {
                            stats.first_result = Some(elapsed);
                        }
                    }
                }
                if let Err(ref e) = r {
                    stats.failed += 1;
                    stats.failed_ids.push(id.clone());
                    // 注意：这里是 TUI 进程内的后台线程，绝不能 println/eprintln——
                    // 会直接写穿终端画面。要看明细用 AGG_TRACE 压测，不在正文打印。
                    let _ = e;
                }
                on_part(Part {
                    id,
                    elapsed,
                    result: r,
                });
            }
            Err(_) => break,
        }
    }
    stats.timed_out = total - answered;
    stats.timed_out_ids = all_ids
        .iter()
        .filter(|id| !answered_ids.contains(id))
        .cloned()
        .collect();
    stats.finished = start.elapsed();
    stats
}

#[cfg(test)]
mod bench {
    use super::*;
    use crate::source::SourceRegistry;
    use std::time::Duration;

    fn targets() -> Vec<(String, Arc<dyn Source>)> {
        let reg = SourceRegistry::with_defaults(
            "http://127.0.0.1:3000".to_string(),
            None,
            "http://127.0.0.1:8123".to_string(),
            "yt-dlp".to_string(),
            None,
            None,
        );
        reg.all()
            .iter()
            .filter(|s| s.capabilities().can_stream)
            .map(|s| (s.id().to_string(), s.clone()))
            .collect()
    }

    /// 只测网易云单个源（验证 TTFB 是不是缓存假象）：
    /// cargo test --release netease_only -- --ignored --nocapture
    #[test]
    #[ignore]
    fn netease_only() {
        let reg = SourceRegistry::with_defaults(
            "http://127.0.0.1:3000".to_string(),
            None,
            "http://127.0.0.1:8123".to_string(),
            "yt-dlp".to_string(),
            None,
            None,
        );
        let src = reg.get("netease").unwrap();
        for round in 1..=3 {
            clear_cache();
            let stats = run(
                vec![("netease".to_string(), src.clone())],
                "晴天",
                20,
                Config {
                    workers: 1,
                    subprocess_workers: 1,
                    timeout: Duration::from_secs(5),
                    use_cache: false,
                },
                |_| {},
            );
            println!(
                "netease round={round} ttfb_ms={:?} total_ms={}",
                stats.first_result.map(|d| d.as_millis()),
                stats.finished.as_millis()
            );
        }
    }

    /// 真实的聚合搜索压测（默认 ignore，需要本机 ncm-api / 侧车 / 网络）：
    /// cargo test --release agg_bench -- --ignored --nocapture
    #[test]
    #[ignore]
    fn agg_bench() {
        let cfgs = [
            (
                "baseline(workers=4,no-cache)",
                Config {
                    workers: 4,
                    subprocess_workers: 64,
                    timeout: Duration::from_secs(5),
                    use_cache: false,
                },
            ),
            (
                "optimized(workers=24,sub=2,cache)",
                Config {
                    workers: 24,
                    subprocess_workers: 2,
                    timeout: Duration::from_secs(5),
                    use_cache: true,
                },
            ),
        ];
        let only = std::env::var("AGG_ONLY").ok();
        let query = std::env::var("AGG_QUERY").unwrap_or_else(|_| "晴天".to_string());
        // 缓存命中演示：不 clear，紧接着再搜一次同样的词
        let cached_cfg = Config {
            workers: 24,
            subprocess_workers: 2,
            timeout: Duration::from_secs(5),
            use_cache: true,
        };
        let stats = run(targets(), &query, 20, cached_cfg, |_| {});
        println!(
            "cache-hit repeat ttfb_ms={:?} total_ms={} timedout={}",
            stats.first_result.map(|d| d.as_millis()),
            stats.finished.as_millis(),
            stats.timed_out
        );
        clear_cache();
        for (name, cfg) in cfgs {
            if let Some(ref o) = only {
                if !name.starts_with(o.as_str()) {
                    continue;
                }
            }
            for round in 1..=5 {
                clear_cache(); // 每轮都冷启动，避免用「改一个字再改回来」的命中掩盖真实耗时
                let mut n = 0;
                let mut first: Option<(String, u128)> = None;
                let trace = std::env::var("AGG_TRACE").is_ok() && round == 1;
                let stats = run(targets(), &query, 20, cfg, |p| {
                    n += 1;
                    if trace {
                        let n_songs = match &p.result {
                            Ok(v) => v.songs.len(),
                            Err(_) => 0,
                        };
                        let ok = p.result.is_ok();
                        println!(
                            "  part {} elapsed_ms={} ok={} songs={}",
                            p.id,
                            p.elapsed.as_millis(),
                            ok,
                            n_songs
                        );
                    }
                    if first.is_none() {
                        if let Ok(ref v) = p.result {
                            if !v.songs.is_empty() {
                                first = Some((p.id.clone(), p.elapsed.as_millis()));
                            }
                        }
                    }
                });
                println!(
                    "{name} round={round} ttfb_ms={:?} first_src={:?} total_ms={} parts={} failed={} timedout={}",
                    stats.first_result.map(|d| d.as_millis()),
                    first,
                    stats.finished.as_millis(),
                    n,
                    stats.failed,
                    stats.timed_out
                );
                if !stats.failed_ids.is_empty() {
                    println!("  failed_ids={:?}", stats.failed_ids);
                }
                if std::env::var("AGG_VERBOSE").is_ok() && round == 1 {
                    let mut parts: Vec<(String, u128, bool, usize)> = Vec::new();
                    // 重跑一遍拿明细太贵，这里只留占位：见下方 AGG_TRACE
                    parts.clear();
                    println!("  (明细见 AGG_TRACE 输出)");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Album, Artist, Lyrics, Song};
    use crate::source::{Capabilities, TrackId};
    use anyhow::Result;

    struct Fake {
        id: &'static str,
        ms: u64,
        fail: bool,
        subprocess: bool,
    }

    impl Source for Fake {
        fn id(&self) -> &'static str {
            self.id
        }
        fn name(&self) -> &'static str {
            self.id
        }
        fn is_subprocess(&self) -> bool {
            self.subprocess
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities {
                kinds: vec![SearchKind::Song],
                lyrics: false,
                qualities: Vec::new(),
                can_stream: true,
            }
        }
        fn search(&self, k: &str, _l: i32, _kind: SearchKind) -> Result<SearchResult> {
            std::thread::sleep(Duration::from_millis(self.ms));
            if self.fail {
                anyhow::bail!("boom");
            }
            Ok(SearchResult {
                songs: vec![Song {
                    id: 0,
                    name: format!("{k}-{}", self.id),
                    ar: vec![Artist {
                        id: 0,
                        name: "a".into(),
                    }],
                    al: Album {
                        id: 0,
                        name: "al".into(),
                        pic_url: None,
                    },
                    dt: 1,
                    fee: 0,
                    source: self.id.to_string(),
                    s_id: "1".into(),
                }],
                ..Default::default()
            })
        }
        fn lyric(&self, _id: &TrackId) -> Result<Lyrics> {
            Ok(Lyrics::default())
        }
        fn stream_url(&self, _id: &TrackId, _q: &str) -> Result<Option<String>> {
            Ok(None)
        }
    }

    fn t(id: &'static str, ms: u64) -> (String, Arc<dyn Source>) {
        (
            id.to_string(),
            Arc::new(Fake {
                id,
                ms,
                fail: false,
                subprocess: false,
            }),
        )
    }

    #[test]
    fn first_playable_falls_back_to_next_source() {
        let mk = |source: &str, s_id: &str| Song {
            id: 0,
            name: "晴天".into(),
            ar: vec![Artist {
                id: 0,
                name: "周杰伦".into(),
            }],
            al: Album {
                id: 0,
                name: "叶惠美".into(),
                pic_url: None,
            },
            dt: 1,
            fee: 0,
            source: source.to_string(),
            s_id: s_id.to_string(),
        };
        let cands = order_candidates(&[mk("migu", "m1"), mk("netease", "n1"), mk("qq", "q1")]);
        // 网易云会员取不到流 → 应该落到 QQ
        let (hit, fails) = first_playable(&cands, |s| match s.source.as_str() {
            "netease" => Ok(None),
            "qq" => Ok(Some("http://qq/stream".to_string())),
            _ => Ok(None),
        });
        let (url, used) = hit.expect("QQ 应能播");
        assert_eq!(url, "http://qq/stream");
        assert_eq!(used.source, "qq");
        assert_eq!(fails.len(), 1);
        assert_eq!(fails[0].0, "网易云");
        assert_eq!(fails[0].1, "需会员或无版权");
    }

    #[test]
    fn first_playable_reports_every_source_when_all_fail() {
        let mk = |source: &str| Song {
            id: 0,
            name: "x".into(),
            ar: vec![Artist {
                id: 0,
                name: "a".into(),
            }],
            al: Album {
                id: 0,
                name: "al".into(),
                pic_url: None,
            },
            dt: 1,
            fee: 0,
            source: source.to_string(),
            s_id: "1".into(),
        };
        let cands = order_candidates(&[mk("qq"), mk("netease"), mk("migu")]);
        let (hit, fails) = first_playable(&cands, |s| match s.source.as_str() {
            "netease" => Ok(None),
            "qq" => Err("无版权".to_string()),
            _ => Ok(None),
        });
        assert!(hit.is_none());
        assert_eq!(fails.len(), 3);
        // 顺序也按优先级：网易云 → QQ → 咪咕
        assert_eq!(fails[0].0, "网易云");
        assert_eq!(fails[1], ("QQ(直连)".to_string(), "无版权".to_string()));
        assert_eq!(fails[2].0, "咪咕");
    }

    #[test]
    fn retain_timed_drops_unknown_duration() {
        let mk = |name: &str, dt: i64| Song {
            id: 0,
            name: name.to_string(),
            ar: vec![Artist {
                id: 0,
                name: "a".into(),
            }],
            al: Album {
                id: 0,
                name: "al".into(),
                pic_url: None,
            },
            dt,
            fee: 0,
            source: "meting:tencent".into(),
            s_id: name.to_string(),
        };
        let mut v = vec![mk("有时长", 269_000), mk("没时长", 0), mk("负数", -1)];
        retain_timed(&mut v);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].name, "有时长");
    }

    #[test]
    fn order_candidates_sorts_by_priority_and_dedups() {
        let mk = |name: &str, source: &str, s_id: &str| Song {
            id: 0,
            name: name.to_string(),
            ar: vec![Artist {
                id: 0,
                name: "a".into(),
            }],
            al: Album {
                id: 0,
                name: "al".into(),
                pic_url: None,
            },
            dt: 1,
            fee: 0,
            source: source.to_string(),
            s_id: s_id.to_string(),
        };
        let list = vec![
            mk("晴天", "migu", "m1"),
            mk("晴天", "netease", "n1"),
            mk("晴天", "qq", "q1"),
            mk("晴天", "netease", "n1"), // 重复，丢掉
            mk("晴天", "meting:tencent", "t1"),
        ];
        let out = order_candidates(&list);
        let ids: Vec<&str> = out.iter().map(|s| s.source.as_str()).collect();
        assert_eq!(
            ids,
            ["netease", "qq", "meting:tencent", "migu"],
            "按优先级排序且去重"
        );
        assert_eq!(out.len(), 4);
    }

    #[test]
    fn streams_parts_and_counts_failures() {
        let targets = vec![
            t("fast", 10),
            t("slow", 60),
            (
                "bad".to_string(),
                Arc::new(Fake {
                    id: "bad",
                    ms: 5,
                    fail: true,
                    subprocess: false,
                }),
            ),
        ];
        let mut got = Vec::new();
        let stats = run(
            targets,
            "q",
            5,
            Config {
                workers: 8,
                subprocess_workers: 1,
                timeout: Duration::from_millis(300),
                use_cache: false,
            },
            |p| got.push(p.id),
        );
        got.sort();
        assert_eq!(got, ["bad", "fast", "slow"]);
        assert_eq!(stats.failed, 1);
        assert_eq!(stats.timed_out, 0);
        assert!(stats.first_result.is_some());
    }

    #[test]
    fn deadline_marks_timed_out_without_waiting_forever() {
        let targets = vec![t("slow", 500), t("ok", 5)];
        let stats = run(
            targets,
            "q",
            5,
            Config {
                workers: 4,
                subprocess_workers: 1,
                timeout: Duration::from_millis(80),
                use_cache: false,
            },
            |_| {},
        );
        assert_eq!(stats.timed_out, 1);
        assert!(!stats.timed_out_ids.is_empty());
        assert!(
            stats.finished < Duration::from_millis(300),
            "必须按截止时间返回"
        );
    }

    #[test]
    fn cache_serves_repeat_query_and_clear_works() {
        clear_cache();
        let cfg = Config {
            workers: 2,
            subprocess_workers: 1,
            timeout: Duration::from_millis(200),
            use_cache: true,
        };
        run(vec![t("a", 5)], "晴天", 5, cfg, |_| {});
        assert_eq!(cache().len(), 1);
        let mut hits = 0;
        let stats = run(vec![t("a", 5)], "晴天", 5, cfg, |p| {
            if p.elapsed < Duration::from_millis(3) {
                hits += 1;
            }
        });
        assert_eq!(stats.failed, 0);
        assert_eq!(hits, 1, "第二次应命中缓存（几乎 0 耗时）");
        clear_cache();
        assert_eq!(cache().len(), 0);
    }

    #[test]
    fn subprocess_sources_run_at_most_two_at_a_time() {
        // 6 个子进程源、每个 60ms、限流 2 → 总耗时应在 ~180ms 而不是 60ms
        let mut targets: Vec<(String, Arc<dyn Source>)> = Vec::new();
        for i in 0..6 {
            let id: &'static str = Box::leak(format!("sub{i}").into_boxed_str());
            targets.push((
                id.to_string(),
                Arc::new(Fake {
                    id,
                    ms: 60,
                    fail: false,
                    subprocess: true,
                }),
            ));
        }
        let cfg = Config {
            workers: 24,
            subprocess_workers: 2,
            timeout: Duration::from_millis(600),
            use_cache: false,
        };
        let stats = run(targets, "q", 5, cfg, |_| {});
        assert_eq!(stats.timed_out, 0);
        assert!(
            stats.finished >= Duration::from_millis(150),
            "限流 2 时不该 60ms 全跑完: {:?}",
            stats.finished
        );
    }
}

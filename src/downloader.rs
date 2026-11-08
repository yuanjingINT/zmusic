use crate::models::Song;
use crate::util;
use anyhow::Result;
use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;

const CHUNK: usize = 20;
const WORKERS: usize = 4;

#[derive(Debug, Clone)]
pub struct DownloadTask {
    pub id: i64,
    pub title: String,
    pub status: DlStatus,
}

#[derive(Debug, Clone)]
pub enum DlStatus {
    Queued,
    Resolving,
    Downloading(f64),
    Done,
    Skipped,
    Failed(String),
}

impl DownloadTask {
    pub fn label(&self) -> String {
        match &self.status {
            DlStatus::Queued => "排队中".to_string(),
            DlStatus::Resolving => "获取链接".to_string(),
            DlStatus::Downloading(p) => format!("{:.0}%", p),
            DlStatus::Done => "完成".to_string(),
            DlStatus::Skipped => "已存在, 跳过".to_string(),
            DlStatus::Failed(e) => format!("失败: {}", e),
        }
    }
}

#[derive(Debug)]
pub struct DownloadState {
    pub tasks: Vec<DownloadTask>,
}

#[derive(Debug, Clone)]
struct Job {
    songs: Vec<Song>,
    dir: PathBuf,
    base: String,
}

pub struct Downloader {
    shared: Arc<Mutex<DownloadState>>,
    tx: Sender<Job>,
    quality: Arc<RwLock<String>>,
    /// 工作线程首次入队时才建：空载时不占 4 个线程的栈和 malloc arena
    spawned: Arc<AtomicBool>,
    rx: Arc<Mutex<Receiver<Job>>>,
    dir: Arc<Mutex<PathBuf>>,
    base: String,
}

impl Downloader {
    pub fn new(dir: PathBuf, quality: Arc<RwLock<String>>, base: String) -> Self {
        let (tx, rx) = channel();
        let shared = Arc::new(Mutex::new(DownloadState { tasks: Vec::new() }));
        let dir = Arc::new(Mutex::new(dir));
        Downloader {
            shared,
            tx,
            spawned: Arc::new(AtomicBool::new(false)),
            rx: Arc::new(Mutex::new(rx)),
            dir,
            quality,
            base,
        }
    }

    pub fn set_dir(&self, dir: PathBuf) {
        *self.dir.lock().unwrap() = dir;
    }

    /// 首次真正要下载时才起工作线程（空载时省掉 4 个线程的栈与分配器 arena）
    fn ensure_workers(&self) {
        if self.spawned.swap(true, Ordering::SeqCst) {
            return;
        }
        for _ in 0..WORKERS {
            let rx = self.rx.clone();
            let sh = self.shared.clone();
            let q = self.quality.clone();
            thread::spawn(move || worker(rx, sh, q));
        }
    }

    pub fn enqueue(&self, songs: Vec<Song>) -> usize {
        self.ensure_workers();
        {
            let mut st = self.shared.lock().unwrap();
            let mut seen: HashSet<i64> = st.tasks.iter().map(|t| t.id).collect();
            let mut added = 0;
            for s in &songs {
                if seen.contains(&s.id) {
                    continue;
                }
                seen.insert(s.id);
                st.tasks.push(DownloadTask {
                    id: s.id,
                    title: format!("{} - {}", util::artists_of(&s.ar), s.name),
                    status: DlStatus::Queued,
                });
                added += 1;
            }
            drop(st);
            let cur_dir = self.dir.lock().unwrap().clone();
            for chunk in songs.chunks(CHUNK) {
                let job = Job {
                    songs: chunk.to_vec(),
                    dir: cur_dir.clone(),
                    base: self.base.clone(),
                };
                let _ = self.tx.send(job);
            }
            added
        }
    }

    pub fn snapshot(&self) -> Vec<DownloadTask> {
        self.shared.lock().unwrap().tasks.clone()
    }

    pub fn counts(&self) -> (usize, usize, usize, usize) {
        let st = self.shared.lock().unwrap();
        let total = st.tasks.len();
        let done = st
            .tasks
            .iter()
            .filter(|t| matches!(t.status, DlStatus::Done))
            .count();
        let skipped = st
            .tasks
            .iter()
            .filter(|t| matches!(t.status, DlStatus::Skipped))
            .count();
        let failed = st
            .tasks
            .iter()
            .filter(|t| matches!(t.status, DlStatus::Failed(_)))
            .count();
        (total, done, skipped, failed)
    }
}

fn set_status(shared: &Arc<Mutex<DownloadState>>, id: i64, status: DlStatus) {
    if let Some(t) = shared.lock().unwrap().tasks.iter_mut().find(|t| t.id == id) {
        t.status = status;
    }
}

fn worker(
    rx: Arc<Mutex<Receiver<Job>>>,
    shared: Arc<Mutex<DownloadState>>,
    quality: Arc<RwLock<String>>,
) {
    // 共享连接池：4 个 worker 共用一套连接，不再各自建 client/runtime
    let client = crate::source::http::client();
    loop {
        let job = rx.lock().unwrap().recv();
        match job {
            Ok(job) => {
                for song in &job.songs {
                    process(client, song, &job, &shared, &quality);
                }
            }
            Err(_) => break,
        }
    }
}

fn process(
    client: &reqwest::blocking::Client,
    song: &Song,
    job: &Job,
    shared: &Arc<Mutex<DownloadState>>,
    quality: &Arc<RwLock<String>>,
) {
    set_status(shared, song.id, DlStatus::Resolving);
    // 每个任务发起时读当前音质：队列里已排队的任务也会用上最新设置
    let level = quality.read().unwrap().clone();
    let url = match resolve_url(client, &job.base, song.id, &level) {
        Ok(Some(u)) => u,
        Ok(None) => {
            set_status(shared, song.id, DlStatus::Failed("无播放链接".to_string()));
            return;
        }
        Err(e) => {
            set_status(shared, song.id, DlStatus::Failed(e.to_string()));
            return;
        }
    };
    let ext = util::ext_from_url(&url).unwrap_or_else(|| "mp3".to_string());
    let fname = util::sanitize_filename(&format!(
        "{} - {}.{}",
        util::artists_of(&song.ar),
        song.name,
        ext
    ));
    let path = job.dir.join(fname);
    if path.exists() {
        set_status(shared, song.id, DlStatus::Skipped);
        return;
    }
    set_status(shared, song.id, DlStatus::Downloading(0.0));
    match download_to(client, &url, &path, |read, total| {
        set_status(
            shared,
            song.id,
            DlStatus::Downloading(read as f64 / total as f64 * 100.0),
        )
    }) {
        Ok(_) => set_status(shared, song.id, DlStatus::Done),
        Err(e) => set_status(shared, song.id, DlStatus::Failed(e.to_string())),
    }
}

fn resolve_url(
    client: &reqwest::blocking::Client,
    base: &str,
    id: i64,
    level: &str,
) -> Result<Option<String>> {
    let r = client
        .get(format!("{}/song/url/v1", base))
        .query(&[("id", id.to_string()), ("level", level.to_string())])
        .send()?;
    let v: serde_json::Value = r.json()?;
    let url = v["data"][0]["url"]
        .as_str()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty());
    Ok(url)
}

/// 所有下载路径共用的落盘实现：128KB 分块、先写 `.part` 再 rename。
/// 普通下载按块报进度，超级下载不关心进度（传 noop）。
pub(crate) fn download_to(
    client: &reqwest::blocking::Client,
    url: &str,
    path: &std::path::Path,
    mut on_progress: impl FnMut(u64, u64),
) -> Result<u64> {
    let mut resp = client.get(url).send()?;
    let total = resp.content_length().unwrap_or(0).max(1);
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = path.with_extension(format!("{}.part", ext));
    let mut f = File::create(&tmp)?;
    let mut buf = vec![0u8; 128 * 1024];
    let mut read: u64 = 0;
    loop {
        let n = resp.read(&mut buf)?;
        if n == 0 {
            break;
        }
        f.write_all(&buf[..n])?;
        read += n as u64;
        on_progress(read, total);
    }
    drop(f);
    std::fs::rename(&tmp, path)?;
    Ok(read)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::time::Duration;

    /// 起一个只认 /song/url/v1 和 /audio.mp3 的本地桩，记录每次请求的 level
    fn stub_api(levels: Arc<Mutex<Vec<String>>>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let base_for_thread = base.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut sock) = stream else { break };
                let mut reader = BufReader::new(sock.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                loop {
                    let mut h = String::new();
                    match reader.read_line(&mut h) {
                        Ok(0) | Err(_) => break,
                        Ok(_) if h == "\r\n" => break,
                        Ok(_) => {}
                    }
                }
                let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                let body = if path.starts_with("/song/url/v1") {
                    if let Some(rest) = path.split("level=").nth(1) {
                        levels
                            .lock()
                            .unwrap()
                            .push(rest.split('&').next().unwrap_or("").to_string());
                    }
                    format!(
                        "{{\"data\":[{{\"url\":\"{}/audio.mp3\"}}]}}",
                        base_for_thread
                    )
                } else {
                    "ID3fake-audio-bytes".to_string()
                };
                let _ = write!(
                    sock,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
            }
        });
        base
    }

    fn wait_done(d: &Downloader, expect: usize) {
        for _ in 0..100 {
            let (total, done, skipped, failed) = d.counts();
            if failed > 0 {
                panic!("有任务失败: {:?}", d.snapshot());
            }
            if done + skipped >= expect && total >= expect {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        panic!("超时：{:?}", d.snapshot());
    }

    fn song(id: i64, name: &str) -> Song {
        Song {
            id,
            name: name.to_string(),
            source: "netease".to_string(),
            s_id: id.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn changing_quality_after_enqueue_affects_next_request() {
        let levels = Arc::new(Mutex::new(Vec::new()));
        let base = stub_api(levels.clone());
        let dir = std::env::temp_dir().join(format!("zmusic-dl-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let quality = Arc::new(RwLock::new("exhigh".to_string()));
        let dl = Downloader::new(dir, quality.clone(), base);

        dl.enqueue(vec![song(1, "第一首")]);
        wait_done(&dl, 1);

        *quality.write().unwrap() = "hires".to_string();
        dl.enqueue(vec![song(2, "第二首")]);
        wait_done(&dl, 2);

        let got = levels.lock().unwrap().clone();
        assert_eq!(got.first().map(String::as_str), Some("exhigh"));
        assert_eq!(
            got.last().map(String::as_str),
            Some("hires"),
            "改音质后下载请求应跟着变"
        );
    }
}

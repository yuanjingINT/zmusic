use crate::models::Song;
use crate::util;
use anyhow::Result;
use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

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
    quality: String,
    base: String,
}

pub struct Downloader {
    shared: Arc<Mutex<DownloadState>>,
    tx: Sender<Job>,
    dir: Arc<Mutex<PathBuf>>,
    quality: String,
    base: String,
}

impl Downloader {
    pub fn new(dir: PathBuf, quality: String, base: String) -> Self {
        let (tx, rx) = channel();
        let shared = Arc::new(Mutex::new(DownloadState { tasks: Vec::new() }));
        let dir = Arc::new(Mutex::new(dir));
        let rx = Arc::new(Mutex::new(rx));
        for _ in 0..WORKERS {
            let rx = rx.clone();
            let sh = shared.clone();
            thread::spawn(move || worker(rx, sh));
        }
        Downloader {
            shared,
            tx,
            dir,
            quality,
            base,
        }
    }

    pub fn set_dir(&self, dir: PathBuf) {
        *self.dir.lock().unwrap() = dir;
    }

    pub fn enqueue(&self, songs: Vec<Song>) -> usize {
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
                    quality: self.quality.clone(),
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
        let done = st.tasks.iter().filter(|t| matches!(t.status, DlStatus::Done)).count();
        let skipped = st.tasks.iter().filter(|t| matches!(t.status, DlStatus::Skipped)).count();
        let failed = st.tasks.iter().filter(|t| matches!(t.status, DlStatus::Failed(_))).count();
        (total, done, skipped, failed)
    }
}

fn set_status(shared: &Arc<Mutex<DownloadState>>, id: i64, status: DlStatus) {
    if let Some(t) = shared.lock().unwrap().tasks.iter_mut().find(|t| t.id == id) {
        t.status = status;
    }
}

fn worker(rx: Arc<Mutex<Receiver<Job>>>, shared: Arc<Mutex<DownloadState>>) {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .unwrap();
    loop {
        let job = rx.lock().unwrap().recv();
        match job {
            Ok(job) => {
                for song in &job.songs {
                    process(&client, song, &job, &shared);
                }
            }
            Err(_) => break,
        }
    }
}

fn process(client: &reqwest::blocking::Client, song: &Song, job: &Job, shared: &Arc<Mutex<DownloadState>>) {
    set_status(shared, song.id, DlStatus::Resolving);
    let url = match resolve_url(client, &job.base, song.id, &job.quality) {
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
    match download_to(client, &url, &path, song.id, shared) {
        Ok(()) => set_status(shared, song.id, DlStatus::Done),
        Err(e) => set_status(shared, song.id, DlStatus::Failed(e.to_string())),
    }
}

fn resolve_url(client: &reqwest::blocking::Client, base: &str, id: i64, level: &str) -> Result<Option<String>> {
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

fn download_to(
    client: &reqwest::blocking::Client,
    url: &str,
    path: &std::path::Path,
    id: i64,
    shared: &Arc<Mutex<DownloadState>>,
) -> Result<()> {
    let mut resp = client.get(url).send()?;
    let total = resp.content_length().unwrap_or(0).max(1);
    let ext = path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
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
        set_status(shared, id, DlStatus::Downloading(read as f64 / total as f64 * 100.0));
    }
    drop(f);
    std::fs::rename(&tmp, path)?;
    Ok(())
}

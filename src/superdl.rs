#![allow(dead_code)]
use crate::api::ApiClient;
use crate::models::Song;
use crate::ui::AppMsg;
use crate::util;
use std::collections::HashSet;
use std::path::Path;
use std::process::Command;
use std::sync::mpsc::Sender;

#[derive(Clone)]
pub struct UsbDevice {
    pub name: String,
    pub path: String,
    pub part: Option<String>,
    pub size: u64,
    pub model: String,
    pub mount: Option<String>,
    pub fs: Option<String>,
}

impl UsbDevice {
    pub fn size_gb(&self) -> String {
        format!("{:.1}GB", self.size as f64 / 1e9)
    }
}

pub fn detect_usb() -> anyhow::Result<Vec<UsbDevice>> {
    let out = Command::new("lsblk")
        .args(["-J", "-b", "-o", "NAME,PATH,TYPE,TRAN,SIZE,MODEL,MOUNTPOINT,FSTYPE,RM"])
        .output()?;
    if !out.status.success() {
        anyhow::bail!("lsblk 执行失败");
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    let mut devs = Vec::new();
    let disks = v
        .get("blockdevices")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    for disk in &disks {
        let is_usb = disk
            .get("tran")
            .and_then(|t| t.as_str())
            .map(|t| t == "usb")
            .unwrap_or(false)
            || disk.get("rm").and_then(|r| r.as_bool()).unwrap_or(false);
        if !is_usb || disk.get("type").and_then(|t| t.as_str()) != Some("disk") {
            continue;
        }
        let path = disk.get("path").and_then(|p| p.as_str()).unwrap_or("").to_string();
        if path.is_empty() {
            continue;
        }
        let size = disk.get("size").and_then(|s| s.as_u64()).unwrap_or(0);
        let model = disk.get("model").and_then(|m| m.as_str()).unwrap_or("").trim().to_string();
        let name = disk.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
        let mut part = None;
        let mut mount = None;
        let mut fs = None;
        if let Some(children) = disk.get("children").and_then(|c| c.as_array()) {
            for ch in children {
                if ch.get("type").and_then(|t| t.as_str()) == Some("part") {
                    part = Some(ch.get("path").and_then(|p| p.as_str()).unwrap_or("").to_string());
                    mount = ch.get("mountpoint").and_then(|m| m.as_str()).map(|s| s.to_string());
                    fs = ch.get("fstype").and_then(|f| f.as_str()).map(|s| s.to_string());
                    break;
                }
            }
        }
        devs.push(UsbDevice {
            name,
            path,
            part,
            size,
            model,
            mount,
            fs,
        });
    }
    Ok(devs)
}

fn run(cmd: &str) -> (bool, String) {
    match Command::new("sh").args(["-c", cmd]).output() {
        Ok(o) => (
            o.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            ),
        ),
        Err(e) => (false, format!("{}", e)),
    }
}

fn free_space(dir: &str) -> u64 {
    if let Ok(out) = Command::new("df").args(["-B1", "--output=avail", dir]).output() {
        let s = String::from_utf8_lossy(&out.stdout);
        if let Some(line) = s.lines().nth(1) {
            if let Ok(n) = line.trim().parse::<u64>() {
                return n;
            }
        }
    }
    0
}

fn collect_hot_songs(client: &ApiClient, max_toplists: usize) -> Vec<Song> {
    let mut seen: HashSet<i64> = HashSet::new();
    let mut all = Vec::new();
    let toplists = client.toplist().unwrap_or_default();
    let mut count = 0;
    for tl in &toplists {
        if count >= max_toplists {
            break;
        }
        if let Ok(songs) = client.playlist_track_all(tl.id) {
            count += 1;
            for s in songs {
                if seen.insert(s.id) {
                    all.push(s);
                }
            }
        }
    }
    all
}

fn download_to(client: &reqwest::blocking::Client, url: &str, path: &Path) -> anyhow::Result<u64> {
    let mut resp = client.get(url).send()?;
    let mut f = std::fs::File::create(path)?;
    let n = std::io::copy(&mut resp, &mut f)?;
    Ok(n)
}

pub fn fill_with_music(
    client: &ApiClient,
    dir: &str,
    quality: &str,
    max_toplists: usize,
    log: &mut dyn FnMut(String),
) {
    let songs = collect_hot_songs(client, max_toplists);
    log(format!("获取到 {} 首热榜歌曲", songs.len()));
    let http = reqwest::blocking::Client::new();
    let mut done: usize = 0;
    let mut skipped: usize = 0;
    let min_free: u64 = 200 * 1024 * 1024;
    for song in &songs {
        let avail = free_space(dir);
        if avail < min_free {
            log("设备剩余空间不足 (<200MB), 停止填充".to_string());
            break;
        }
        let url = match client.song_url_v1(&[song.id], quality) {
            Ok(u) => u.into_iter().next().and_then(|x| x.url),
            Err(_) => None,
        };
        let Some(url) = url else {
            log(format!(
                "跳过(无播放链接): {} - {}",
                song.name,
                util::artists_of(&song.ar)
            ));
            continue;
        };
        let ext = util::ext_from_url(&url).unwrap_or_else(|| "mp3".to_string());
        let fname = util::sanitize_filename(&format!(
            "{} - {}.{}",
            util::artists_of(&song.ar),
            song.name,
            ext
        ));
        let path = Path::new(dir).join(fname);
        if path.exists() {
            skipped += 1;
            continue;
        }
        match download_to(&http, &url, &path) {
            Ok(n) => {
                done += 1;
                log(format!(
                    "[{}] 完成: {} - {} ({:.1}MB, 剩余 {:.1}GB)",
                    done,
                    song.name,
                    util::artists_of(&song.ar),
                    n as f64 / 1e6,
                    free_space(dir) as f64 / 1e9
                ));
            }
            Err(e) => log(format!(
                "失败: {} - {} ({})",
                song.name,
                util::artists_of(&song.ar),
                e
            )),
        }
    }
    log(format!(
        "填充结束: 下载 {} 首, 跳过 {} 首, 剩余空间 {:.1}MB",
        done,
        skipped,
        free_space(dir) as f64 / 1e6
    ));
}

fn mount_device<F: Fn(&str) -> (bool, String)>(target: &str, sudo_run: &F) -> Option<String> {
    let (ok, out) = run(&format!("udisksctl mount -b {}", target));
    if ok {
        if let Some(pos) = out.find(" at ") {
            let m = out[pos + 4..].trim().to_string();
            if !m.is_empty() {
                return Some(m);
            }
        }
    }
    let dir = "/mnt/zmusic";
    let _ = run(&format!("mkdir -p {}", dir));
    let (ok, _) = sudo_run(&format!("sudo mount {} {}", target, dir));
    if ok {
        Some(dir.to_string())
    } else {
        None
    }
}

pub fn run_super_download<F: Fn(&str) -> (bool, String)>(
    dev: &UsbDevice,
    api_base: &str,
    quality: &str,
    tx: Sender<AppMsg>,
    sudo_run: F,
) {
    let log = |s: String| {
        let _ = tx.send(AppMsg::SuperProgress(s));
    };
    log(format!(
        "==> 开始超级下载: {} ({}GB, {})",
        dev.path,
        dev.size as f64 / 1e9,
        dev.model
    ));
    let target = dev.part.clone().unwrap_or_else(|| dev.path.clone());
    let (ok, _) = run(&format!("udisksctl unmount -b {} -f", target));
    if !ok {
        let _ = sudo_run(&format!("sudo umount -f {}", target));
    }
    let fs = if dev.size < 32 * 1024 * 1024 * 1024 {
        "FAT32"
    } else {
        "exFAT"
    };
    log(format!("==> 格式化 {} 为 {} (会切换到终端输入 sudo 密码)...", target, fs));
    let cmd = if fs == "FAT32" {
        format!("sudo mkfs.vfat -F 32 -n ZMUSIC {}", target)
    } else {
        format!("sudo mkfs.exfat -n ZMUSIC {}", target)
    };
    let (ok, out) = sudo_run(&cmd);
    if !ok {
        let _ = tx.send(AppMsg::SuperDone(Err(format!("格式化失败: {}", out.trim()))));
        return;
    }
    log("格式化完成".to_string());
    let Some(mount) = mount_device(&target, &sudo_run) else {
        let _ = tx.send(AppMsg::SuperDone(Err("挂载失败".to_string())));
        return;
    };
    log(format!("已挂载到 {}", mount));
    let client = ApiClient::new(api_base.to_string());
    let mut log2 = |s: String| {
        let _ = tx.send(AppMsg::SuperProgress(s));
    };
    fill_with_music(&client, &mount, quality, 12, &mut log2);
    let _ = tx.send(AppMsg::SuperDone(Ok("超级下载完成".to_string())));
}


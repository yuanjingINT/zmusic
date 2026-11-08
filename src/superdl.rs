#![allow(dead_code)]
use crate::models::Song;
use crate::source::netease::NetEaseSource;
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
        .args([
            "-J",
            "-b",
            "-o",
            "NAME,PATH,TYPE,TRAN,SIZE,MODEL,MOUNTPOINT,FSTYPE,RM",
        ])
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
        let path = disk
            .get("path")
            .and_then(|p| p.as_str())
            .unwrap_or("")
            .to_string();
        if path.is_empty() {
            continue;
        }
        let size = disk.get("size").and_then(|s| s.as_u64()).unwrap_or(0);
        let model = disk
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let name = disk
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("")
            .to_string();
        let mut part = None;
        let mut mount = None;
        let mut fs = None;
        if let Some(children) = disk.get("children").and_then(|c| c.as_array()) {
            for ch in children {
                if ch.get("type").and_then(|t| t.as_str()) == Some("part") {
                    part = Some(
                        ch.get("path")
                            .and_then(|p| p.as_str())
                            .unwrap_or("")
                            .to_string(),
                    );
                    mount = ch
                        .get("mountpoint")
                        .and_then(|m| m.as_str())
                        .map(|s| s.to_string());
                    fs = ch
                        .get("fstype")
                        .and_then(|f| f.as_str())
                        .map(|s| s.to_string());
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
    if let Ok(out) = Command::new("df")
        .args(["-B1", "--output=avail", dir])
        .output()
    {
        let s = String::from_utf8_lossy(&out.stdout);
        if let Some(line) = s.lines().nth(1) {
            if let Ok(n) = line.trim().parse::<u64>() {
                return n;
            }
        }
    }
    0
}

fn collect_hot_songs(client: &NetEaseSource, max_toplists: usize) -> Vec<Song> {
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

pub fn fill_with_music(
    client: &NetEaseSource,
    dir: &str,
    quality: &str,
    max_toplists: usize,
    log: &mut dyn FnMut(String),
) {
    let songs = collect_hot_songs(client, max_toplists);
    log(format!("获取到 {} 首热榜歌曲", songs.len()));
    let http = crate::source::http::client();
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
            Ok(u) => u
                .into_iter()
                .next()
                .and_then(crate::source::netease::accept_stream),
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
        match crate::downloader::download_to(http, &url, &path, |_, _| {}) {
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
    // 无盘/自动化场景：直接往指定目录下歌，跳过卸载与格式化（不碰任何设备）。
    // 没有可移动盘时也能验证「取流失败不写盘」这类行为。
    if let Some(dir) = std::env::var("ZMUSIC_SUPERDL_DIR")
        .ok()
        .filter(|s| !s.trim().is_empty())
    {
        log(format!(
            "==> ZMUSIC_SUPERDL_DIR={dir}，跳过格式化，直接写入该目录"
        ));
        let _ = std::fs::create_dir_all(&dir);
        let client = NetEaseSource::new(api_base.to_string());
        let mut log2 = |s: String| {
            let _ = tx.send(AppMsg::SuperProgress(s));
        };
        fill_with_music(&client, &dir, quality, 12, &mut log2);
        let _ = tx.send(AppMsg::SuperDone(Ok("超级下载完成".to_string())));
        return;
    }
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
    log(format!(
        "==> 格式化 {} 为 {} (会切换到终端输入 sudo 密码)...",
        target, fs
    ));
    let cmd = if fs == "FAT32" {
        format!("sudo mkfs.vfat -F 32 -n ZMUSIC {}", target)
    } else {
        format!("sudo mkfs.exfat -n ZMUSIC {}", target)
    };
    let (ok, out) = sudo_run(&cmd);
    if !ok {
        let _ = tx.send(AppMsg::SuperDone(Err(format!(
            "格式化失败: {}",
            out.trim()
        ))));
        return;
    }
    log("格式化完成".to_string());
    let Some(mount) = mount_device(&target, &sudo_run) else {
        let _ = tx.send(AppMsg::SuperDone(Err("挂载失败".to_string())));
        return;
    };
    log(format!("已挂载到 {}", mount));
    let client = NetEaseSource::new(api_base.to_string());
    let mut log2 = |s: String| {
        let _ = tx.send(AppMsg::SuperProgress(s));
    };
    fill_with_music(&client, &mount, quality, 12, &mut log2);
    let _ = tx.send(AppMsg::SuperDone(Ok("超级下载完成".to_string())));
}

#[cfg(test)]
mod merge_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// 极简桩：/toplist、/playlist/track/all、/song/url/v1、/audio
    fn stub_api(dead_url: bool) -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for s in l.incoming() {
                let mut s = match s {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                let mut buf = [0u8; 2048];
                let n = s.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                let (ctype, body): (&str, Vec<u8>) = if path.starts_with("/toplist") {
                    (
                        "application/json",
                        br#"{"list":[{"id":1,"name":"t"}]}"#.to_vec(),
                    )
                } else if path.starts_with("/playlist/track/all") {
                    (
                        "application/json",
                        "{\"code\":200,\"songs\":[{\"id\":11,\"name\":\"测\",\"ar\":[{\"id\":1,\"name\":\"人\"}],\"al\":{\"id\":1,\"name\":\"专\"},\"dt\":180000,\"fee\":0}]}".as_bytes().to_vec(),
                    )
                } else if path.starts_with("/song/url/v1") && dead_url {
                    // 404 兜底死链：url 非空但指向打不通的端口，旧实现会把它写进盘
                    (
                        "application/json",
                        br#"{"data":[{"id":11,"url":"http://127.0.0.1:9/dead.mp3","code":404,"br":0,"size":0,"fee":1}]}"#
                            .to_vec(),
                    )
                } else if path.starts_with("/song/url/v1") {
                    (
                        "application/json",
                        format!(
                            r#"{{"data":[{{"id":11,"url":"http://127.0.0.1:{port}/audio.mp3","code":200,"br":320000,"size":19,"fee":0}}]}}"#
                        )
                        .into_bytes(),
                    )
                } else {
                    ("audio/mpeg", b"ID3fake-audio-bytes".to_vec())
                };
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = s.write_all(head.as_bytes());
                let _ = s.write_all(&body);
            }
        });
        port
    }

    /// 超级下载走的是 downloader 的共用 download_to：真下文件并逐字节比对
    #[test]
    fn super_download_lands_exact_bytes() {
        let port = stub_api(false);
        let src = crate::source::netease::NetEaseSource::new(format!("http://127.0.0.1:{port}"));
        let dir = std::env::temp_dir().join(format!("zm-superdl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut logs: Vec<String> = Vec::new();
        fill_with_music(&src, dir.to_str().unwrap(), "exhigh", 1, &mut |l| {
            logs.push(l)
        });
        let got = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .find(|e| e.path().extension().map(|x| x == "mp3").unwrap_or(false))
            .expect("超级下载应该落一个 mp3");
        assert_eq!(std::fs::read(got.path()).unwrap(), b"ID3fake-audio-bytes");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 取流返回 404 死链时必须跳过该曲：既不落 .mp3，也不留 .part
    #[test]
    fn dead_404_link_is_skipped_without_writing() {
        let port = stub_api(true);
        let src = crate::source::netease::NetEaseSource::new(format!("http://127.0.0.1:{port}"));
        let dir = std::env::temp_dir().join(format!("zm-superdl-dead-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut logs: Vec<String> = Vec::new();
        fill_with_music(&src, dir.to_str().unwrap(), "exhigh", 1, &mut |l| {
            logs.push(l)
        });
        let files: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(files.is_empty(), "死链不该落盘，实际有 {files:?}");
        assert!(
            logs.iter().any(|l| l.contains("跳过(无播放链接)")),
            "应该跳过并记日志，实际日志: {logs:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}

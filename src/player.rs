use crate::util;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default)]
pub struct MpvState {
    pub time_pos: Option<f64>,
    pub duration: Option<f64>,
    pub paused: bool,
    pub eof: bool,
    pub idle: bool,
    pub volume: u8,
}

#[allow(dead_code)]
#[derive(Debug)]
pub enum MpvCmd {
    /// 加载并播放（强制解暂停）：用于用户主动播放、自动连播
    LoadPlay(String),
    /// 只加载，保持当前暂停状态：用于手动切歌、切音质重取流
    Load(String),
    Pause(bool),
    Toggle,
    SeekRel(f64),
    SeekAbs(f64),
    Volume(i32),
    Quit,
}

pub struct Player {
    state: Arc<Mutex<MpvState>>,
    tx: Sender<MpvCmd>,
    child: Option<Child>,
    socket: String,
}

#[derive(Clone, Copy)]
enum Poll {
    TimePos,
    Duration,
    Pause,
    Eof,
    Idle,
    Volume,
}

impl Player {
    pub fn start() -> Result<Self> {
        let socket = util::mpv_socket();
        let _ = std::fs::remove_file(&socket);
        let out = std::fs::File::create("/tmp/ncm_mpv.log").map_err(|e| anyhow!("无法创建 mpv 日志: {}", e))?;
        let ipc = format!("--input-ipc-server={}", socket);
        let child = Command::new("mpv")
            .args([
                "--idle",
                "--no-video",
                "--no-terminal",
                &ipc,
                "--volume=70",
                "--cache=yes",
                // eof-reached 只在 keep-open 时可靠为 yes；没有它，播完就回到 idle，
                // 自动下一首永远触发不了
                "--keep-open=yes",
                "--demuxer-max-bytes=512MiB",
                "--log-file=/tmp/ncm_mpv.log",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::from(out))
            .stderr(Stdio::from(
                std::fs::File::create("/tmp/ncm_mpv_err.log").map_err(|e| anyhow!("无法创建 mpv 日志: {}", e))?,
            ))
            .spawn()
            .map_err(|e| anyhow!("无法启动 mpv: {}", e))?;
        let mut stream = None;
        for _ in 0..60 {
            match UnixStream::connect(&socket) {
                Ok(s) => {
                    stream = Some(s);
                    break;
                }
                Err(_) => std::thread::sleep(Duration::from_millis(100)),
            }
        }
        let stream = stream.ok_or_else(|| anyhow!("mpv IPC 连接失败 (socket: {})", socket))?;
        let (tx, rx) = channel();
        let state = Arc::new(Mutex::new(MpvState {
            volume: 70,
            ..Default::default()
        }));
        let st = state.clone();
        std::thread::spawn(move || ipc_loop(stream, rx, st));
        Ok(Player {
            state,
            tx,
            child: Some(child),
            socket,
        })
    }

    pub fn load(&self, url: &str) {
        let _ = self.tx.send(MpvCmd::Load(url.to_string()));
    }

    /// 加载并强制播放（清掉 keep-open 留下的暂停态）
    pub fn load_play(&self, url: &str) {
        let _ = self.tx.send(MpvCmd::LoadPlay(url.to_string()));
    }

    pub fn toggle(&self) {
        let _ = self.tx.send(MpvCmd::Toggle);
    }

    pub fn seek_rel(&self, s: f64) {
        let _ = self.tx.send(MpvCmd::SeekRel(s));
    }

    pub fn seek_abs(&self, s: f64) {
        let _ = self.tx.send(MpvCmd::SeekAbs(s));
    }

    pub fn volume(&self, delta: i32) {
        let _ = self.tx.send(MpvCmd::Volume(delta));
    }

    pub fn state(&self) -> MpvState {
        self.state.lock().unwrap().clone()
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.tx.send(MpvCmd::Quit);
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        let _ = std::fs::remove_file(&self.socket);
    }
}

fn ipc_loop(mut stream: UnixStream, rx: Receiver<MpvCmd>, state: Arc<Mutex<MpvState>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
    let mut rid: u64 = 0;
    loop {
        let mut quit = false;
        loop {
            match rx.try_recv() {
                Ok(cmd) => match cmd {
                    MpvCmd::Quit => {
                        quit = true;
                        let _ = send_cmd(&mut stream, &["quit"], &mut rid);
                        break;
                    }
                    MpvCmd::Load(u) => {
                        // 只 loadfile，不碰 pause：手动暂停时切歌保持暂停语义
                        let _ = send_cmd(&mut stream, &["loadfile", &u, "replace"], &mut rid);
                    }
                    MpvCmd::LoadPlay(u) => {
                        let _ = send_cmd(&mut stream, &["loadfile", &u, "replace"], &mut rid);
                        // keep-open 下播完一首会停在暂停态，自动连播只 loadfile 不会出声，
                        // 这里显式解暂停
                        let _ = send_cmd(&mut stream, &["set_property", "pause", "no"], &mut rid);
                    }
                    MpvCmd::Pause(b) => {
                        let _ = send_cmd(
                            &mut stream,
                            &["set_property", "pause", if b { "yes" } else { "no" }],
                            &mut rid,
                        );
                    }
                    MpvCmd::Toggle => {
                        let _ = send_cmd(&mut stream, &["cycle", "pause"], &mut rid);
                    }
                    MpvCmd::SeekRel(s) => {
                        let _ = send_cmd(
                            &mut stream,
                            &["seek", &format!("{}", s), "relative"],
                            &mut rid,
                        );
                    }
                    MpvCmd::SeekAbs(s) => {
                        let _ = send_cmd(
                            &mut stream,
                            &["seek", &format!("{}", s), "absolute"],
                            &mut rid,
                        );
                    }
                    MpvCmd::Volume(d) => {
                        let _ = send_cmd(
                            &mut stream,
                            &["add", "volume", &format!("{}", d)],
                            &mut rid,
                        );
                    }
                },
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    quit = true;
                    break;
                }
            }
        }
        if quit {
            break;
        }
        let mut outstanding: Vec<(u64, Poll)> = Vec::new();
        for (p, name) in [
            (Poll::TimePos, "time-pos"),
            (Poll::Duration, "duration"),
            (Poll::Pause, "pause"),
            (Poll::Eof, "eof-reached"),
            (Poll::Idle, "idle-active"),
            (Poll::Volume, "volume"),
        ] {
            rid += 1;
            let id = rid;
            if send_prop(&mut stream, name, id).is_ok() {
                outstanding.push((id, p));
            }
        }
        let deadline = Instant::now() + Duration::from_millis(500);
        let mut buf: Vec<u8> = Vec::new();
        while !outstanding.is_empty() && Instant::now() < deadline {
            let mut tmp = [0u8; 8192];
            match stream.read(&mut tmp) {
                Ok(0) => break,
                Ok(n) => buf.extend_from_slice(&tmp[..n]),
                Err(_) => break,
            }
            let mut i = 0;
            while let Some(pos) = buf[i..].iter().position(|&b| b == b'\n') {
                let line = buf[i..i + pos].to_vec();
                i += pos + 1;
                if let Ok(v) = serde_json::from_slice::<Value>(&line) {
                    if let Some(rr) = v.get("request_id").and_then(|x| x.as_u64()) {
                        if let Some(idx) = outstanding.iter().position(|(id, _)| *id == rr) {
                            let p = outstanding.remove(idx).1;
                            apply_poll(&v, p, &state);
                        }
                    }
                }
            }
            buf.drain(..i);
        }
        std::thread::sleep(Duration::from_millis(60));
    }
}

fn apply_poll(v: &Value, p: Poll, state: &Arc<Mutex<MpvState>>) {
    let data = v.get("data");
    let mut st = state.lock().unwrap();
    match p {
        Poll::TimePos => st.time_pos = data.and_then(|d| d.as_f64()),
        Poll::Duration => st.duration = data.and_then(|d| d.as_f64()),
        Poll::Pause => st.paused = data.and_then(|d| d.as_bool()).unwrap_or(false),
        Poll::Eof => st.eof = data.and_then(|d| d.as_bool()).unwrap_or(false),
        Poll::Idle => st.idle = data.and_then(|d| d.as_bool()).unwrap_or(true),
        Poll::Volume => {
            if let Some(v) = data.and_then(|d| d.as_f64()) {
                st.volume = v as u8;
            }
        }
    }
}

fn send_cmd(stream: &mut UnixStream, args: &[&str], rid: &mut u64) -> std::io::Result<()> {
    *rid += 1;
    let v = json!({ "command": args, "request_id": *rid });
    write_json(stream, &v)
}

fn send_prop(stream: &mut UnixStream, name: &str, rid: u64) -> std::io::Result<()> {
    let v = json!({ "command": ["get_property", name], "request_id": rid });
    write_json(stream, &v)
}

fn write_json(stream: &mut UnixStream, v: &Value) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(v).unwrap();
    line.push(b'\n');
    stream.write_all(&line)
}

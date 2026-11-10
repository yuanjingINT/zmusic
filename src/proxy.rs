//! 代理支持。
//!
//! 背景：YouTube 的 googlevideo 直链把签名绑到出口 IP，zmuse 和 mpv 必须走同一个出口。
//! yt-dlp 自己认 `--proxy socks5h://…`，但 mpv 的 `--http-proxy` 只支持 HTTP 代理，
//! 所以这是给 mpv 用的最小桥：在本机起一个 HTTP CONNECT 代理，转发到配置里的 SOCKS5。

use anyhow::{anyhow, Result};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::sync::OnceLock;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProxyKind {
    /// http:// 或 https:// 开头的 HTTP 代理：mpv 直接能用
    Http(String),
    /// socks5:// 或 socks5h://：mpv 不认，要走内置桥
    Socks(String),
}

/// 解析配置里的代理串；空串/None 视为没配
pub fn parse(proxy: Option<&str>) -> Option<ProxyKind> {
    let p = proxy?.trim();
    if p.is_empty() {
        return None;
    }
    let lower = p.to_ascii_lowercase();
    if lower.starts_with("socks5h://") || lower.starts_with("socks5://") {
        Some(ProxyKind::Socks(p.to_string()))
    } else {
        Some(ProxyKind::Http(p.to_string()))
    }
}

/// 给 mpv 用的代理地址。HTTP 代理原样返回；SOCKS 代理起一次内置桥，返回桥的地址。
pub fn mpv_proxy(proxy: Option<&str>) -> Result<Option<String>> {
    http_proxy(proxy)
}

/// 任何只认 HTTP 代理的客户端（mpv、reqwest）都能用的地址：
/// SOCKS 代理会起一次内置桥转成 `http://127.0.0.1:<port>`。
pub fn http_proxy(proxy: Option<&str>) -> Result<Option<String>> {
    match parse(proxy) {
        None => Ok(None),
        Some(ProxyKind::Http(p)) => Ok(Some(p)),
        Some(ProxyKind::Socks(p)) => Ok(Some(socks_bridge(&p)?)),
    }
}

static BRIDGE: OnceLock<Result<String, String>> = OnceLock::new();

/// 进程内单例：同一个 SOCKS 代理只起一个桥
fn socks_bridge(socks: &str) -> Result<String> {
    let r = BRIDGE.get_or_init(|| start_bridge(socks).map_err(|e| e.to_string()));
    match r {
        Ok(url) => Ok(url.clone()),
        Err(e) => Err(anyhow!("启动本地代理桥失败: {}", e)),
    }
}

/// 起一个 HTTP CONNECT 代理，转发到 `socks`，监听 127.0.0.1 的随机端口
pub fn start_bridge(socks: &str) -> Result<String> {
    let target = socks.to_string();
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(stream) = conn else { break };
            let target = target.clone();
            std::thread::spawn(move || {
                let _ = handle_client(stream, &target);
            });
        }
    });
    Ok(format!("http://127.0.0.1:{}", port))
}

fn handle_client(mut client: TcpStream, socks: &str) -> Result<()> {
    client.set_read_timeout(Some(Duration::from_secs(15)))?;
    let mut head = Vec::new();
    let mut buf = [0u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = client.read(&mut buf)?;
        if n == 0 {
            return Ok(());
        }
        head.extend_from_slice(&buf[..n]);
        if head.len() > 16 * 1024 {
            return Err(anyhow!("请求头过大"));
        }
    }
    let text = String::from_utf8_lossy(&head).to_string();
    let first = text.lines().next().unwrap_or("");
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("");
    if !method.eq_ignore_ascii_case("CONNECT") {
        let _ = client.write_all(b"HTTP/1.1 405 Method Not Allowed\r\n\r\n");
        return Ok(());
    }
    let (host, port) = target
        .rsplit_once(':')
        .ok_or_else(|| anyhow!("CONNECT 目标格式不对: {}", target))?;
    let port: u16 = port.parse().map_err(|_| anyhow!("端口不对: {}", target))?;

    let mut upstream = socks5_connect(socks, host, port)?;
    client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")?;
    client.set_read_timeout(None)?;

    let mut c2 = client.try_clone()?;
    let mut u2 = upstream.try_clone()?;
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut c2, &mut u2);
        let _ = u2.shutdown(std::net::Shutdown::Write);
    });
    let _ = std::io::copy(&mut upstream, &mut client);
    let _ = client.shutdown(std::net::Shutdown::Write);
    Ok(())
}

/// 最小 SOCKS5（无认证）客户端
fn socks5_connect(socks: &str, host: &str, port: u16) -> Result<TcpStream> {
    let addr = socks
        .trim_start_matches("socks5h://")
        .trim_start_matches("socks5://");
    let (phost, pport) = addr
        .rsplit_once(':')
        .ok_or_else(|| anyhow!("SOCKS 地址格式不对: {}", socks))?;
    let pport: u16 = pport
        .parse()
        .map_err(|_| anyhow!("SOCKS 端口不对: {}", socks))?;
    let sock_addr = (phost, pport)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| anyhow!("解析 SOCKS 地址失败: {}", socks))?;
    let mut s = TcpStream::connect_timeout(&sock_addr, Duration::from_secs(10))?;
    s.set_read_timeout(Some(Duration::from_secs(10)))?;

    s.write_all(&[0x05, 0x01, 0x00])?;
    let mut resp = [0u8; 2];
    s.read_exact(&mut resp)?;
    if resp != [0x05, 0x00] {
        return Err(anyhow!("SOCKS 握手失败: {:?}", resp));
    }

    let mut req = vec![0x05, 0x01, 0x00, 0x03, host.len() as u8];
    req.extend_from_slice(host.as_bytes());
    req.extend_from_slice(&port.to_be_bytes());
    s.write_all(&req)?;

    let mut head = [0u8; 4];
    s.read_exact(&mut head)?;
    if head[1] != 0x00 {
        return Err(anyhow!("SOCKS 连接被拒，code={}", head[1]));
    }
    match head[3] {
        0x01 => {
            let mut skip = [0u8; 4];
            s.read_exact(&mut skip)?;
        }
        0x04 => {
            let mut skip = [0u8; 16];
            s.read_exact(&mut skip)?;
        }
        0x03 => {
            let mut len = [0u8; 1];
            s.read_exact(&mut len)?;
            let mut skip = vec![0u8; len[0] as usize];
            s.read_exact(&mut skip)?;
        }
        _ => return Err(anyhow!("SOCKS 回复地址类型未知")),
    }
    let mut port_buf = [0u8; 2];
    s.read_exact(&mut port_buf)?;
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn parses_proxy_strings() {
        assert_eq!(parse(None), None);
        assert_eq!(parse(Some("  ")), None);
        assert_eq!(
            parse(Some("socks5h://127.0.0.1:11080")),
            Some(ProxyKind::Socks("socks5h://127.0.0.1:11080".into()))
        );
        assert_eq!(
            parse(Some("http://127.0.0.1:3128")),
            Some(ProxyKind::Http("http://127.0.0.1:3128".into()))
        );
    }

    /// 起一个假 SOCKS5 服务：只完成握手+CONNECT，然后把收到的字节原样回写
    fn fake_socks() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(mut s) = conn else { break };
                std::thread::spawn(move || {
                    let mut buf = [0u8; 3];
                    if s.read_exact(&mut buf).is_err() {
                        return;
                    }
                    let _ = s.write_all(&[0x05, 0x00]);
                    let mut head = [0u8; 4];
                    if s.read_exact(&mut head).is_err() {
                        return;
                    }
                    let mut addr = [0u8; 1];
                    let _ = s.read_exact(&mut addr);
                    let mut host = vec![0u8; addr[0] as usize];
                    let _ = s.read_exact(&mut host);
                    let mut port = [0u8; 2];
                    let _ = s.read_exact(&mut port);
                    let _ = s.write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0]);
                    // 回声
                    let mut echo = [0u8; 64];
                    loop {
                        match s.read(&mut echo) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                if s.write_all(&echo[..n]).is_err() {
                                    break;
                                }
                            }
                        }
                    }
                });
            }
        });
        port
    }

    #[test]
    fn bridge_tunnels_through_socks() {
        let socks_port = fake_socks();
        let bridge = start_bridge(&format!("socks5h://127.0.0.1:{socks_port}")).unwrap();
        let addr = bridge.trim_start_matches("http://");
        let mut c = TcpStream::connect(addr).unwrap();
        c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        c.write_all(b"CONNECT example.com:443 HTTP/1.1\r\nHost: example.com\r\n\r\n")
            .unwrap();
        let mut head = [0u8; 39];
        c.read_exact(&mut head).unwrap();
        assert!(
            String::from_utf8_lossy(&head).contains("200"),
            "应返回 200: {:?}",
            String::from_utf8_lossy(&head)
        );
        c.write_all(b"ping").unwrap();
        let mut echo = [0u8; 4];
        c.read_exact(&mut echo).unwrap();
        assert_eq!(&echo, b"ping", "隧道应把数据原样转发");
    }
}

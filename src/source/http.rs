//! 公开 API 源共用的小工具：一个带超时的 blocking client + JSON 读取。

use anyhow::{anyhow, Context, Result};
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::OnceLock;
use std::time::Duration;

pub fn client() -> &'static Client {
    static CLI: OnceLock<Client> = OnceLock::new();
    CLI.get_or_init(|| {
        Client::builder()
            // 全进程唯一的 client：搜索/取流/歌词/下载共用一套连接池
            // （以前每个调用方自己建，一个 blocking client 就带一个 tokio runtime + 线程池）
            // 全局 120s 给下载；搜索这类快请求在 get_json/get_text 里压到 6s
            .timeout(Duration::from_secs(120))
            .connect_timeout(Duration::from_secs(2))
            .pool_max_idle_per_host(16)
            .tcp_nodelay(true)
            .user_agent("zmusic/0.1 (+https://git.tuxai.org/personal/zmusic)")
            .build()
            .expect("构建 HTTP client 失败")
    })
}

pub fn get_json(url: &str) -> Result<Value> {
    let resp = client()
        .get(url)
        .timeout(Duration::from_secs(6))
        .send()
        .with_context(|| format!("请求失败: {url}"))?;
    if !resp.status().is_success() {
        return Err(anyhow!("HTTP {}: {url}", resp.status()));
    }
    resp.json::<Value>()
        .with_context(|| format!("JSON 解析失败: {url}"))
}

pub fn get_text(url: &str) -> Result<String> {
    let resp = client()
        .get(url)
        .timeout(Duration::from_secs(6))
        .send()
        .with_context(|| format!("请求失败: {url}"))?;
    if !resp.status().is_success() {
        return Err(anyhow!("HTTP {}: {url}", resp.status()));
    }
    Ok(resp.text()?)
}

/// "4:35" / "1:02:03" / "275" -> 毫秒；认不出来就 0
pub fn duration_ms(s: &str) -> i64 {
    let s = s.trim();
    let parts: Vec<&str> = s.split(':').collect();
    // h:mm:ss / mm:ss / ss
    let secs = match parts.len() {
        3 => match (
            parts[0].trim().parse::<i64>(),
            parts[1].trim().parse::<i64>(),
            parts[2].trim().parse::<f64>(),
        ) {
            (Ok(h), Ok(m), Ok(sec)) => Some(h as f64 * 3600.0 + m as f64 * 60.0 + sec),
            _ => None,
        },
        2 => match (
            parts[0].trim().parse::<i64>(),
            parts[1].trim().parse::<f64>(),
        ) {
            (Ok(m), Ok(sec)) => Some(m as f64 * 60.0 + sec),
            _ => None,
        },
        _ => s.parse::<f64>().ok(),
    };
    secs.map(|v| (v * 1000.0).round() as i64).unwrap_or(0)
}

/// 最小的 percent-encode（查询串用）
pub fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mm_ss_h_mm_ss_and_bare_seconds() {
        assert_eq!(duration_ms("4:35"), 275_000);
        assert_eq!(duration_ms("1:02:03"), 3_723_000);
        assert_eq!(duration_ms("275"), 275_000);
        assert_eq!(duration_ms("246.381"), 246_381);
        assert_eq!(duration_ms(""), 0);
        assert_eq!(duration_ms("--:--"), 0);
    }

    #[test]
    fn percent_encode_keeps_unreserved_bytes() {
        assert_eq!(enc("aZ0-_.~"), "aZ0-_.~");
        assert_eq!(enc("晴天 a"), "%E6%99%B4%E5%A4%A9%20a");
    }
}

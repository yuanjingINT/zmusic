use crate::models::Artist;

pub fn sanitize_filename(name: &str) -> String {
    let mut s = String::new();
    for c in name.chars() {
        if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
            s.push(' ');
        } else {
            s.push(c);
        }
    }
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let s = s.trim();
    if s.is_empty() {
        "unnamed".to_string()
    } else if s.chars().count() > 120 {
        s.chars().take(120).collect()
    } else {
        s.to_string()
    }
}

pub fn fmt_time(secs: Option<f64>) -> String {
    match secs {
        Some(s) if s.is_finite() && s > 0.0 => {
            let total = s as i64;
            format!("{:02}:{:02}", total / 60, total % 60)
        }
        _ => "--:--".to_string(),
    }
}

pub fn year_of_ms(ms: i64) -> String {
    let secs = ms / 1000;
    let days = secs / 86400;
    let y = 1970 + days / 365;
    format!("{}年", y)
}

pub fn fmt_count(n: i64) -> String {
    if n >= 100_000_000 {
        format!("{:.1}亿", n as f64 / 1e8)
    } else if n >= 10_000 {
        format!("{:.1}万", n as f64 / 1e4)
    } else {
        n.to_string()
    }
}

pub fn artists_of(ar: &[Artist]) -> String {
    ar.iter()
        .map(|a| a.name.clone())
        .collect::<Vec<_>>()
        .join(" & ")
}

pub fn ext_from_url(url: &str) -> Option<String> {
    let path = url.split('?').next()?;
    let seg = path.rsplit('/').next()?;
    if seg.contains('.') {
        let ext = seg.rsplit('.').next()?;
        if !ext.is_empty() && ext.len() <= 6 {
            return Some(ext.to_lowercase());
        }
    }
    None
}

pub fn mpv_socket() -> String {
    let user = std::env::var("USER").unwrap_or_else(|_| "user".to_string());
    format!("/tmp/ncm_mpv_{}.sock", user)
}

pub fn parse_lrc(text: &str) -> Vec<(u64, String)> {
    let mut out = Vec::new();
    let mut offset: i64 = 0;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let mut rest = line;
        let mut times = Vec::new();
        loop {
            if let Some(stripped) = rest.strip_prefix('[') {
                if let Some(end) = stripped.find(']') {
                    let tag = &stripped[..end];
                    if let Some(t) = parse_lrc_time(tag) {
                        times.push(t);
                        rest = &stripped[end + 1..];
                        continue;
                    }
                }
            }
            break;
        }
        if rest.starts_with("offset:") {
            if let Some(v) = rest["offset:".len()..].trim().parse::<i64>().ok() {
                offset = v;
            }
            continue;
        }
        if times.is_empty() {
            continue;
        }
        let text = rest.trim().to_string();
        if text.is_empty() {
            continue;
        }
        for t in times {
            let shifted = ((t as i64) + offset).max(0) as u64;
            out.push((shifted, text.clone()));
        }
    }
    out
}

fn parse_lrc_time(tag: &str) -> Option<u64> {
    let (m, s) = tag.split_once(':')?;
    let m: f64 = m.parse().ok()?;
    let s: f64 = s.parse().ok()?;
    Some(((m * 60.0 + s) * 1000.0) as u64)
}

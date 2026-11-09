//! 搜索页的「源连通性」视图：纯逻辑，渲染只读探活缓存，**不发网络请求**。

use crate::source::health::SourceState;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// 内部源 id（查原因、判断是不是当前源都靠它）
    pub id: String,
    pub label: String,
    pub state: SourceState,
    /// 当前搜索用的源：加粗/反显
    pub current: bool,
    /// 能搜不能播（Ok 但 can_stream=false）：标黄
    pub only_search: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Good,
    Warn,
    Dim,
}

pub fn tone_of(it: &Item) -> Tone {
    match it.state {
        SourceState::Ok if it.only_search => Tone::Warn,
        SourceState::Ok => Tone::Good,
        SourceState::NeedsCreds => Tone::Warn,
        SourceState::Down | SourceState::Probing | SourceState::Unknown => Tone::Dim,
    }
}

/// 状态点：连通 ● / 不连通 ○
pub fn dot(it: &Item) -> &'static str {
    if it.state.is_ok() {
        "●"
    } else {
        "○"
    }
}

/// 末尾计数：探测中显示进度，其余显示可用数
pub fn suffix(probing: bool, done: usize, total: usize, ok: usize) -> String {
    if probing {
        format!("探测中 {done}/{total}")
    } else {
        format!("{ok}/{total} 可用")
    }
}

/// 按终端显示宽度截断，超宽用 `…` 收尾（中文/emoji 不会错位）
pub fn truncate(s: &str, max_width: usize) -> String {
    if s.width() <= max_width {
        return s.to_string();
    }
    if max_width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut w = 0usize;
    for ch in s.chars() {
        let cw = ch.to_string().width();
        if w + cw + 1 > max_width {
            break;
        }
        out.push(ch);
        w += cw;
    }
    out.push('…');
    out
}

/// 一行源明细：`● 网易云音乐   ok`
pub fn detail_line(it: &Item, reason: &str) -> String {
    format!("{} {}  {}", dot(it), it.label, reason)
}

/// 按终端显示宽度左补空格（右对齐）：中文/emoji 不会把列撑歪
pub fn pad_left(s: &str, width: usize) -> String {
    let w = s.width();
    if w >= width {
        return truncate(s, width);
    }
    let mut out = String::with_capacity(s.len() + (width - w));
    for _ in 0..(width - w) {
        out.push(' ');
    }
    out.push_str(s);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(label: &str, state: SourceState) -> Item {
        Item {
            id: label.to_string(),
            label: label.to_string(),
            state,
            current: false,
            only_search: false,
        }
    }

    #[test]
    fn tone_marks_only_search_and_creds_as_warn() {
        let mut it = item("x", SourceState::Ok);
        assert_eq!(tone_of(&it), Tone::Good);
        it.only_search = true;
        assert_eq!(tone_of(&it), Tone::Warn, "仅搜不能播标黄");
        it.state = SourceState::NeedsCreds;
        assert_eq!(tone_of(&it), Tone::Warn);
        it.state = SourceState::Down;
        assert_eq!(tone_of(&it), Tone::Dim);
    }

    #[test]
    fn dots_follow_connectivity() {
        assert_eq!(dot(&item("a", SourceState::Ok)), "●");
        assert_eq!(dot(&item("b", SourceState::Down)), "○");
        assert_eq!(dot(&item("c", SourceState::NeedsCreds)), "○");
        assert_eq!(dot(&item("d", SourceState::Probing)), "○");
    }

    #[test]
    fn suffix_switches_between_probing_and_count() {
        assert_eq!(suffix(true, 7, 16, 0), "探测中 7/16");
        assert_eq!(suffix(false, 0, 16, 14), "14/16 可用");
    }

    #[test]
    fn detail_line_has_dot_label_reason() {
        let it = item("YouTube", SourceState::Down);
        assert_eq!(detail_line(&it, "代理未配置"), "○ YouTube  代理未配置");
    }

    #[test]
    fn truncate_keeps_display_width_and_adds_ellipsis() {
        for w in [4usize, 8, 12, 20] {
            let out = truncate("网易云音乐 侧车未启动 很长的一行", w);
            assert!(out.width() <= w, "宽度 {} 超了: {out}", w);
        }
        assert_eq!(truncate("abc", 10), "abc");
        assert_eq!(truncate("", 5), "");
        assert_eq!(truncate("abcdef", 0), "");
        // 中文占两列也按显示宽度算
        assert_eq!(truncate("网易云音乐", 7), "网易云…");
        assert_eq!(truncate("网易云音乐", 8), "网易云…");
    }
}

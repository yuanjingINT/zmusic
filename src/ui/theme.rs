//! 统一调色板：界面里**所有**颜色都从这里取，screens.rs 不许再出现裸 `Color::`。
//!
//! 三层语义：
//! - 前景/次级/边框：结构和层次（靠明度分层，不靠花哨颜色）
//! - 强调（accent）：标题、进度条、焦点边框
//! - 状态（success/warn/error）：连通性、错误、提示
//!
//! 三套调色板，按终端能力自动降级：
//! - `truecolor`：`COLORTERM=truecolor|24bit`
//! - `256`：其余彩色终端
//! - `mono`：`NO_COLOR` 存在或 `TERM=dumb`（只靠 Dim/Bold/反显分层）

use ratatui::style::{Color, Modifier, Style};
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Palette {
    Truecolor,
    Ansi256,
    Mono,
}

#[derive(Debug, Clone)]
pub struct Theme {
    pub palette: Palette,
    /// 正文
    pub fg: Style,
    /// 次级信息（歌手/来源/时长/快捷键）
    pub secondary: Style,
    /// 标题栏、块标题
    pub header: Style,
    /// 强调：焦点、数值、图标
    pub accent: Style,
    pub success: Style,
    pub warn: Style,
    pub error: Style,
    /// 普通边框
    pub border: Style,
    /// 焦点边框（当前页/弹窗）
    pub border_focus: Style,
    /// 选中行（整行反显）
    pub selection: Style,
    /// 正在播放的那一首
    pub playing: Style,
    /// 来源标签
    pub source: Style,
    /// 进度条
    pub gauge: Style,
    pub lyric_current: Style,
    pub lyric: Style,
    /// 正在播放页的大标题
    pub song_title: Style,
}

impl Theme {
    /// 按环境变量选调色板。`get` 注入便于单测（`std::env::var` 在测试里会被并行污染）。
    pub fn from_env(get: &dyn Fn(&str) -> Option<String>) -> Theme {
        let no_color = get("NO_COLOR").is_some();
        let dumb = get("TERM").map(|t| t == "dumb").unwrap_or(false);
        if no_color || dumb {
            return Theme::mono();
        }
        // 配置里选了单色（[ui] theme = "mono" → ZMUSE_THEME），走单色调色板
        if get("ZMUSE_THEME")
            .map(|v| v.eq_ignore_ascii_case("mono"))
            .unwrap_or(false)
        {
            return Theme::mono();
        }
        let truecolor = get("COLORTERM")
            .map(|v| {
                let v = v.to_ascii_lowercase();
                v == "truecolor" || v == "24bit"
            })
            .unwrap_or(false);
        if truecolor {
            Theme::md3_truecolor()
        } else {
            Theme::md3_256()
        }
    }

    pub fn detect() -> Theme {
        Theme::from_env(&|k| std::env::var(k).ok())
    }

    /// MD3 暗底：紫（primary）强调 + 青色来源标签，层次靠明度
    fn md3_truecolor() -> Theme {
        let c = |hex: u32| Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8);
        Theme {
            palette: Palette::Truecolor,
            fg: Style::new().fg(c(0xE6E1E5)),
            secondary: Style::new().fg(c(0x938F99)),
            header: Style::new().fg(c(0xD0BCFF)).add_modifier(Modifier::BOLD),
            accent: Style::new().fg(c(0xD0BCFF)),
            success: Style::new().fg(c(0xA6D189)),
            warn: Style::new().fg(c(0xF2CC60)),
            error: Style::new().fg(c(0xF28B82)),
            border: Style::new().fg(c(0x49454F)),
            border_focus: Style::new().fg(c(0xD0BCFF)),
            selection: Style::new()
                .bg(c(0x4A4458))
                .fg(c(0xE8DEF8))
                .add_modifier(Modifier::BOLD),
            playing: Style::new().fg(c(0xD0BCFF)).add_modifier(Modifier::BOLD),
            source: Style::new().fg(c(0x8FD3E8)),
            gauge: Style::new().fg(c(0xD0BCFF)).bg(c(0x36343B)),
            lyric_current: Style::new().fg(c(0xD0BCFF)).add_modifier(Modifier::BOLD),
            lyric: Style::new().fg(c(0xB0ABB4)),
            song_title: Style::new().fg(c(0xFFFFFF)).add_modifier(Modifier::BOLD),
        }
    }

    /// 256 色近似：没有 truecolor 的终端也别掉回 8 色
    fn md3_256() -> Theme {
        let c = Color::Indexed;
        Theme {
            palette: Palette::Ansi256,
            fg: Style::new().fg(c(252)),
            secondary: Style::new().fg(c(245)),
            header: Style::new().fg(c(183)).add_modifier(Modifier::BOLD),
            accent: Style::new().fg(c(183)),
            success: Style::new().fg(c(114)),
            warn: Style::new().fg(c(220)),
            error: Style::new().fg(c(203)),
            border: Style::new().fg(c(239)),
            border_focus: Style::new().fg(c(183)),
            selection: Style::new()
                .bg(c(237))
                .fg(c(189))
                .add_modifier(Modifier::BOLD),
            playing: Style::new().fg(c(183)).add_modifier(Modifier::BOLD),
            source: Style::new().fg(c(117)),
            gauge: Style::new().fg(c(183)).bg(c(236)),
            lyric_current: Style::new().fg(c(183)).add_modifier(Modifier::BOLD),
            lyric: Style::new().fg(c(250)),
            song_title: Style::new().fg(c(231)).add_modifier(Modifier::BOLD),
        }
    }

    /// 单色：只靠 Dim / Bold / 反显
    fn mono() -> Theme {
        Theme {
            palette: Palette::Mono,
            fg: Style::new(),
            secondary: Style::new().add_modifier(Modifier::DIM),
            header: Style::new().add_modifier(Modifier::BOLD),
            accent: Style::new().add_modifier(Modifier::BOLD),
            success: Style::new().add_modifier(Modifier::BOLD),
            warn: Style::new().add_modifier(Modifier::BOLD),
            error: Style::new()
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::UNDERLINED),
            border: Style::new().add_modifier(Modifier::DIM),
            border_focus: Style::new().add_modifier(Modifier::BOLD),
            selection: Style::new().add_modifier(Modifier::REVERSED),
            playing: Style::new().add_modifier(Modifier::BOLD),
            source: Style::new().add_modifier(Modifier::DIM),
            gauge: Style::new().add_modifier(Modifier::REVERSED),
            lyric_current: Style::new().add_modifier(Modifier::BOLD),
            lyric: Style::new().add_modifier(Modifier::DIM),
            song_title: Style::new().add_modifier(Modifier::BOLD),
        }
    }
}

fn current() -> &'static Theme {
    static T: OnceLock<Theme> = OnceLock::new();
    T.get_or_init(Theme::detect)
}

// ---- 兼容旧调用点的语义访问器（screens.rs 只认这些名字） ----

pub fn primary() -> Style {
    current().accent
}

pub fn title() -> Style {
    current().header
}

pub fn highlight() -> Style {
    current().selection
}

pub fn dim() -> Style {
    current().secondary
}

pub fn good() -> Style {
    current().success
}

pub fn warn() -> Style {
    current().warn
}

pub fn err() -> Style {
    current().error
}

pub fn lyric_current() -> Style {
    current().lyric_current
}

pub fn lyric_normal() -> Style {
    current().lyric
}

pub fn song_title() -> Style {
    current().song_title
}

pub fn playing() -> Style {
    current().playing
}

pub fn source_tag() -> Style {
    current().source
}

pub fn gauge_style() -> Style {
    current().gauge
}

/// 当前调色板（设置页显示用，也方便排「为什么颜色不对」）
pub fn palette_name() -> &'static str {
    match current().palette {
        Palette::Truecolor => "truecolor",
        Palette::Ansi256 => "256 色",
        Palette::Mono => "单色（NO_COLOR/TERM=dumb）",
    }
}

/// 列表/详情块的边框：`focused = true` 用强调色
pub fn border_style(focused: bool) -> Style {
    if focused {
        current().border_focus
    } else {
        current().border
    }
}

pub fn fg() -> Style {
    current().fg
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_with(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| {
            pairs
                .iter()
                .find(|(name, _)| *name == k)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn truecolor_when_colorterm_says_so() {
        let t = Theme::from_env(&env_with(&[
            ("COLORTERM", "truecolor"),
            ("TERM", "xterm-256color"),
        ]));
        assert_eq!(t.palette, Palette::Truecolor);
        assert!(matches!(t.accent.fg, Some(Color::Rgb(..))));
    }

    #[test]
    fn truecolor_also_for_24bit() {
        let t = Theme::from_env(&env_with(&[("COLORTERM", "24bit")]));
        assert_eq!(t.palette, Palette::Truecolor);
    }

    #[test]
    fn ansi256_without_colorterm() {
        let t = Theme::from_env(&env_with(&[("TERM", "xterm-256color")]));
        assert_eq!(t.palette, Palette::Ansi256);
        assert!(matches!(t.accent.fg, Some(Color::Indexed(_))));
    }

    #[test]
    fn mono_when_no_color() {
        // NO_COLOR 只要存在就生效，空值也算
        let t = Theme::from_env(&env_with(&[("NO_COLOR", ""), ("COLORTERM", "truecolor")]));
        assert_eq!(t.palette, Palette::Mono);
    }

    #[test]
    fn mono_when_term_dumb() {
        let t = Theme::from_env(&env_with(&[("TERM", "dumb")]));
        assert_eq!(t.palette, Palette::Mono);
    }

    /// 单色模式不许带任何颜色，只靠修饰符分层
    #[test]
    fn mono_theme_has_no_colors() {
        let t = Theme::mono();
        let styles = [
            t.fg,
            t.secondary,
            t.header,
            t.accent,
            t.success,
            t.warn,
            t.error,
            t.border,
            t.border_focus,
            t.selection,
            t.playing,
            t.source,
            t.gauge,
            t.lyric_current,
            t.lyric,
            t.song_title,
        ];
        for s in styles {
            assert!(s.fg.is_none(), "mono 不应有前景色: {s:?}");
            assert!(s.bg.is_none(), "mono 不应有背景色: {s:?}");
        }
        assert!(t.secondary.add_modifier.contains(Modifier::DIM));
        assert!(t.selection.add_modifier.contains(Modifier::REVERSED));
    }

    /// 两套彩色调色板里，错误色必须和成功色不同（语义不能塌成一坨）
    #[test]
    fn semantic_colors_are_distinct() {
        for t in [Theme::md3_truecolor(), Theme::md3_256()] {
            assert_ne!(t.error.fg, t.success.fg);
            assert_ne!(t.warn.fg, t.success.fg);
            assert_ne!(t.secondary.fg, t.fg.fg);
        }
    }
}

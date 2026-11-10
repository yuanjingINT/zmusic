//! `zmuse config` 的交互设置界面：选外观、开关波形、改下载目录。
//!
//! 设计取向：用户要的是「选样式」，不是「编辑一个 TOML」。所以这里是一屏可键盘
//! 操作的设置列表，改完按 s 写回 `~/.config/zmuse/config.toml` 的 `[general]` /
//! `[ui]`；想直接编辑文件仍可用 `zmuse config --edit`。

use crate::config::Config;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, List, ListItem, ListState, Paragraph};
use unicode_width::UnicodeWidthStr;
use ratatui::Terminal;
use std::io::stdout;
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq)]
enum Row {
    Sources,
    Theme,
    Wave,
    WaveSpacing,
    WavePad,
    Animations,
    Spinner,
    DownloadDir,
    SuperDlDir,
    Quality,
}

const ROWS: [Row; 10] = [
    Row::Sources,
    Row::Theme,
    Row::Wave,
    Row::WaveSpacing,
    Row::WavePad,
    Row::Animations,
    Row::Spinner,
    Row::DownloadDir,
    Row::SuperDlDir,
    Row::Quality,
];

struct State {
    sel: usize,
    /// 启用的源 id（和 config.toml 的 [sources] enabled 同步）
    sources: Vec<String>,
    /// 正在编辑源列表：true 时 j/k 在源之间移动、空格切换
    picking: bool,
    pick_sel: usize,
    theme: usize,
    wave: bool,
    spacing: u8,
    pad: u8,
    animations: bool,
    spinner: bool,
    download_dir: String,
    superdl_dir: String,
    quality: usize,
    editing: Option<String>,
    dirty: bool,
    status: Option<(String, bool)>,
}

const THEMES: [&str; 2] = ["md3", "mono"];

/// 主题名按界面语言显示（值本身仍是 md3 / mono）
fn theme_label(i: usize) -> &'static str {
    match (i, crate::i18n::en()) {
        (0, false) => "MD3 暗色（自动降级 truecolor/256/单色）",
        (0, true) => "MD3 dark (auto-degrades to truecolor/256/mono)",
        (_, false) => "单色（只用明暗/反显）",
        (_, true) => "Mono (brightness and reverse video only)",
    }
}
const QUALITIES: [&str; 5] = ["standard", "higher", "exhigh", "lossless", "hires"];

impl State {
    fn from_cfg(cfg: &Config) -> Self {
        let theme = match std::env::var("ZMUSE_THEME").as_deref() {
            Ok("mono") => 1,
            _ => 0,
        };
        let quality = QUALITIES
            .iter()
            .position(|q| *q == cfg.quality)
            .unwrap_or(2);
        State {
            sel: 0,
            sources: cfg.enabled_sources.clone(),
            picking: false,
            pick_sel: 0,
            theme,
            wave: cfg.animations.equalizer,
            spacing: cfg.animations.wave_spacing.clamp(1, 3),
            pad: cfg.animations.wave_vpad,
            animations: cfg.animations.animations,
            spinner: cfg.animations.spinner,
            download_dir: cfg.download_dir.display().to_string(),
            superdl_dir: std::env::var("ZMUSE_SUPERDL_DIR").unwrap_or_default(),
            quality,
            editing: None,
            dirty: false,
            status: None,
        }
    }

    fn value(&self, row: Row) -> String {
        match row {
            Row::Sources => {
                if self.sources.is_empty() {
                    crate::tr!("（没有启用任何源，回车选择）", "(none enabled, Enter to choose)")
                } else {
                    crate::tr!(
                        "{} 个（回车选择）",
                        "{} (Enter to choose)",
                        self.sources.len()
                    )
                }
            }
            Row::Theme => theme_label(self.theme).to_string(),
            Row::Wave => yes_no(self.wave),
            Row::WaveSpacing => crate::tr!(
                "{}（1 密 / 2 默认 / 3 疏）",
                "{} (1 dense / 2 default / 3 airy)",
                self.spacing
            ),
            Row::WavePad => crate::tr!("{} 行", "{} rows", self.pad),
            Row::Animations => yes_no(self.animations),
            Row::Spinner => yes_no(self.spinner),
            Row::DownloadDir => self.download_dir.clone(),
            Row::SuperDlDir => {
                if self.superdl_dir.is_empty() {
                    crate::tr!(
                        "（未设置：超级下载只认可移动设备）",
                        "(unset: super download only accepts removable drives)"
                    )
                } else {
                    self.superdl_dir.clone()
                }
            }
            Row::Quality => QUALITIES[self.quality].to_string(),
        }
    }

    fn cycle(&mut self, dir: i32) {
        let row = ROWS[self.sel];
        match row {
            Row::Sources => {
                self.picking = true;
                self.pick_sel = 0;
            }
            Row::Theme => {
                self.theme = wrap(self.theme, THEMES.len(), dir);
            }
            Row::Wave => self.wave = !self.wave,
            Row::WaveSpacing => {
                let n = ((self.spacing as i32 - 1 + dir).rem_euclid(3)) as u8 + 1;
                self.spacing = n;
            }
            Row::WavePad => self.pad = ((self.pad as i32 + dir).rem_euclid(3)) as u8,
            Row::Animations => self.animations = !self.animations,
            Row::Spinner => self.spinner = !self.spinner,
            Row::Quality => {
                self.quality = wrap(self.quality, QUALITIES.len(), dir);
            }
            Row::DownloadDir | Row::SuperDlDir => {
                self.editing = Some(self.value(row));
                return;
            }
        }
        self.dirty = true;
    }

    /// 把改动写回 config.toml 的 [general] / [ui]
    fn save(&mut self, cfg: &Config) -> anyhow::Result<()> {
        let path = &cfg.config_path;
        let raw = std::fs::read_to_string(path).unwrap_or_default();
        let mut doc: toml::Value = raw
            .parse()
            .unwrap_or_else(|_| toml::Value::Table(Default::default()));
        if !doc.is_table() {
            doc = toml::Value::Table(Default::default());
        }
        let table = doc.as_table_mut().unwrap();
        {
            let general = table
                .entry("general")
                .or_insert_with(|| toml::Value::Table(Default::default()));
            if let Some(t) = general.as_table_mut() {
                t.insert(
                    "download_dir".into(),
                    toml::Value::String(self.download_dir.clone()),
                );
                t.insert(
                    "quality".into(),
                    toml::Value::String(QUALITIES[self.quality].to_string()),
                );
            }
        }
        {
            let ui = table
                .entry("ui")
                .or_insert_with(|| toml::Value::Table(Default::default()));
            if let Some(t) = ui.as_table_mut() {
                t.insert(
                    "wave_vpad".into(),
                    toml::Value::Integer(self.pad as i64),
                );
                t.insert("animations".into(), toml::Value::Boolean(self.animations));
                t.insert("spinner".into(), toml::Value::Boolean(self.spinner));
                t.insert(
                    "equalizer".into(),
                    toml::Value::Boolean(self.wave && self.animations),
                );
                t.insert(
                    "wave_spacing".into(),
                    toml::Value::Integer(self.spacing as i64),
                );
                t.insert("theme".into(), toml::Value::String(THEMES[self.theme].into()));
            }
        }
        if !self.superdl_dir.trim().is_empty() {
            let t = table
                .entry("superdl")
                .or_insert_with(|| toml::Value::Table(Default::default()));
            if let Some(t) = t.as_table_mut() {
                t.insert(
                    "dir".into(),
                    toml::Value::String(self.superdl_dir.trim().to_string()),
                );
            }
        }
        if let Some(dir) = PathBuf::from(&self.download_dir).parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(path, toml::to_string_pretty(&doc)?)?;
        // 源列表单独写（这一段的结构由 Config::save_enabled_sources 统一维护）
        cfg.save_enabled_sources(&self.sources)?;
        self.dirty = false;
        self.status = Some((
            crate::tr!("已写入 {}", "Written to {}", path.display()),
            true,
        ));
        Ok(())
    }
}

fn yes_no(v: bool) -> String {
    if v {
        crate::tr!("开", "on")
    } else {
        crate::tr!("关", "off")
    }
}

fn wrap(cur: usize, len: usize, dir: i32) -> usize {
    ((cur as i32 + dir).rem_euclid(len as i32)) as usize
}

pub fn run(cfg: &Config) -> anyhow::Result<()> {
    let mut st = State::from_cfg(cfg);
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(out);
    let mut term = Terminal::new(backend)?;
    let res = loop_ui(&mut term, &mut st, cfg);
    disable_raw_mode()?;
    execute!(term.backend_mut(), LeaveAlternateScreen)?;
    term.show_cursor()?;
    res
}

fn loop_ui(
    term: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    st: &mut State,
    cfg: &Config,
) -> anyhow::Result<()> {
    loop {
        term.draw(|f| draw(f, st))?;
        if let Event::Key(k) = event::read()? {
            if k.kind != KeyEventKind::Press {
                continue;
            }
            // 源选择模式：j/k 移动，空格切换，Enter/Esc 回到设置列表
            if st.picking {
                match k.code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        st.pick_sel = st.pick_sel.saturating_sub(1)
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        st.pick_sel = (st.pick_sel + 1).min(crate::source::ALL_SOURCE_IDS.len() - 1)
                    }
                    KeyCode::Char(' ') => {
                        let id = crate::source::ALL_SOURCE_IDS[st.pick_sel].to_string();
                        if let Some(pos) = st.sources.iter().position(|x| *x == id) {
                            st.sources.remove(pos);
                        } else {
                            st.sources.push(id);
                            st.sources.sort_by_key(|x| {
                                crate::source::ALL_SOURCE_IDS
                                    .iter()
                                    .position(|y| y == x)
                                    .unwrap_or(usize::MAX)
                            });
                        }
                        st.dirty = true;
                    }
                    KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') => st.picking = false,
                    _ => {}
                }
                continue;
            }
            // 路径编辑模式：先吃字符
            if let Some(buf) = st.editing.as_mut() {
                match k.code {
                    KeyCode::Char('c') if k.modifiers.contains(event::KeyModifiers::CONTROL) => {
                        st.editing = None;
                    }
                    KeyCode::Enter => {
                        let v = buf.trim().to_string();
                        let row = ROWS[st.sel];
                        match row {
                            Row::DownloadDir => st.download_dir = v,
                            Row::SuperDlDir => st.superdl_dir = v,
                            _ => {}
                        }
                        st.dirty = true;
                        st.editing = None;
                    }
                    KeyCode::Esc => st.editing = None,
                    KeyCode::Backspace => {
                        buf.pop();
                    }
                    KeyCode::Char(c) => buf.push(c),
                    _ => {}
                }
                continue;
            }
            match k.code {
                KeyCode::Char('q') | KeyCode::Esc => {
                    if st.dirty {
                        let _ = st.save(cfg);
                    }
                    return Ok(());
                }
                KeyCode::Char('s') => st.save(cfg)?,
                KeyCode::Up | KeyCode::Char('k') => st.sel = st.sel.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => {
                    st.sel = (st.sel + 1).min(ROWS.len() - 1)
                }
                KeyCode::Left | KeyCode::Char('h') => st.cycle(-1),
                KeyCode::Right | KeyCode::Char('l') | KeyCode::Enter => st.cycle(1),
                _ => {}
            }
        }
    }
}

fn draw(f: &mut ratatui::Frame, st: &State) {
    let area = f.area();
    let fill = area;
    f.render_widget(ratatui::widgets::Clear, fill);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Span::styled(
            format!(" {} ", crate::i18n::t("set.title")),
            Style::default()
                .fg(Color::Rgb(208, 188, 255))
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let [list_area, help_area] =
        Layout::vertical([Constraint::Min(6), Constraint::Length(2)]).areas(inner);
    let row_area = list_area;
    // 两列对齐：标签列宽按显示宽度算（中文占 2 格），否则一列中文一列英文永远对不齐
    let labels: Vec<&str> = ROWS
        .iter()
        .map(|r| match r {
            Row::Sources => crate::i18n::t("set.sources"),
            Row::Theme => crate::i18n::t("set.theme"),
            Row::Wave => crate::i18n::t("set.wave"),
            Row::WaveSpacing => crate::i18n::t("set.spacing"),
            Row::WavePad => crate::i18n::t("set.pad"),
            Row::Animations => crate::i18n::t("set.anim"),
            Row::Spinner => crate::i18n::t("set.spinner"),
            Row::DownloadDir => crate::i18n::t("set.download"),
            Row::SuperDlDir => crate::i18n::t("set.superdl"),
            Row::Quality => crate::i18n::t("set.quality"),
        })
        .collect();
    let label_w = labels
        .iter()
        .map(|l| UnicodeWidthStr::width(*l))
        .max()
        .unwrap_or(10)
        + 2;
    let rows: Vec<ListItem> = ROWS
        .iter()
        .zip(labels.iter())
        .map(|(r, label)| {
            let editing = st.editing.is_some() && ROWS[st.sel] == *r;
            let value = if editing {
                format!("{}_", st.editing.clone().unwrap_or_default())
            } else {
                st.value(*r)
            };
            let pad = label_w.saturating_sub(UnicodeWidthStr::width(*label));
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{label}{}", " ".repeat(pad)),
                    Style::default().fg(Color::Rgb(230, 225, 229)),
                ),
                Span::styled(
                    value,
                    if editing {
                        Style::default().fg(Color::Rgb(242, 231, 255))
                    } else {
                        Style::default().fg(Color::Rgb(208, 188, 255))
                    },
                ),
            ]))
        })
        .collect();
    if st.picking {
        // 源选择：一屏列出全部内置源，[x] 表示启用
        let rows: Vec<ListItem> = crate::source::ALL_SOURCE_IDS
            .iter()
            .map(|id| {
                let on = st.sources.iter().any(|e| e == id);
                ListItem::new(Line::from(vec![
                    Span::styled(
                        if on { "[x] " } else { "[ ] " },
                        Style::default().fg(if on {
                            Color::Rgb(166, 209, 137)
                        } else {
                            Color::Rgb(147, 143, 153)
                        }),
                    ),
                    Span::styled(
                        format!("{:<16} {}", id, crate::source::source_display(id)),
                        Style::default().fg(Color::Rgb(230, 225, 229)),
                    ),
                ]))
            })
            .collect();
        let mut state = ListState::default();
        state.select(Some(st.pick_sel));
        f.render_stateful_widget(
            List::new(rows)
                .highlight_style(
                    Style::default()
                        .bg(Color::Rgb(74, 68, 88))
                        .fg(Color::Rgb(242, 231, 255))
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol("▸ "),
            row_area,
            &mut state,
        );
        let help = Paragraph::new(Line::from(Span::styled(
            crate::tr!(
                "j/k 移动   空格 启用/停用   Enter 返回   s 保存（保存后按 R 重新探测）",
                "j/k move   Space toggle   Enter back   s save (press R in the player afterwards)"
            ),
            Style::default().fg(Color::Rgb(147, 143, 153)),
        )));
        f.render_widget(help, help_area);
        return;
    }
    let mut state = ListState::default();
    state.select(Some(st.sel));
    f.render_stateful_widget(
        List::new(rows)
            .highlight_style(
                Style::default()
                    .bg(Color::Rgb(74, 68, 88))
                    .fg(Color::Rgb(242, 231, 255))
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▸ "),
        row_area,
        &mut state,
    );
    let status_ok = st.status.as_ref().map(|s| s.1).unwrap_or(true);
    let status = match &st.status {
        Some((msg, _ok)) => msg.clone(),
        None => {
            if st.dirty {
                "未保存（q 会自动保存，Esc 退出）".to_string()
            } else {
                String::new()
            }
        }
    };
    let help = vec![
        Line::from(Span::styled(
            crate::i18n::t("set.help"),
            Style::default().fg(Color::Rgb(147, 143, 153)),
        )),
        Line::from(Span::styled(
            status,
            Style::default().fg(if status_ok {
                Color::Rgb(120, 200, 150)
            } else {
                Color::Rgb(230, 120, 120)
            }),
        )),
    ];
    f.render_widget(Paragraph::new(help), help_area);
}

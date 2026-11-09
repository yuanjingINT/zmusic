//! 首次启动向导：语言 → 主题 → 波形 → 下载目录 → 音质。
//!
//! 只在第一次运行（`config.toml` 里没有 `first_run_done = true`）出现；
//! 升级不会再弹——判据是配置文件里的标记，不是「有没有配置文件」。
//! 想重跑：`zmusic config --wizard`，或删掉 config.toml 里那行标记。

use crate::config::Config;
use crate::i18n::{self, Lang};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};
use ratatui::Terminal;
use std::io::stdout;

const QUALITIES: [&str; 5] = ["standard", "higher", "exhigh", "lossless", "hires"];
const THEMES: [&str; 2] = ["MD3 dark", "mono"];

struct State {
    step: usize,
    lang: Lang,
    theme: usize,
    wave: bool,
    dir: String,
    quality: usize,
    editing: bool,
}

impl State {
    fn new(cfg: &Config) -> Self {
        State {
            step: 0,
            lang: i18n::lang(),
            theme: 0,
            wave: true,
            dir: cfg.download_dir.display().to_string(),
            quality: QUALITIES
                .iter()
                .position(|q| *q == cfg.quality)
                .unwrap_or(2),
            editing: false,
        }
    }
}

pub fn run(cfg: &Config) -> anyhow::Result<()> {
    let mut st = State::new(cfg);
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
        let Event::Key(k) = event::read()? else {
            continue;
        };
        if k.kind != KeyEventKind::Press {
            continue;
        }
        if st.editing {
            match k.code {
                KeyCode::Enter => {
                    st.editing = false;
                    st.step += 1;
                }
                KeyCode::Esc => {
                    st.editing = false;
                    st.step += 1;
                }
                KeyCode::Backspace => {
                    st.dir.pop();
                }
                KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    st.editing = false;
                }
                KeyCode::Char(c) => st.dir.push(c),
                _ => {}
            }
            continue;
        }
        match k.code {
            KeyCode::Esc => {
                // 跳过向导：保留默认值，直接标记完成
                finish(cfg, st)?;
                return Ok(());
            }
            KeyCode::Enter => {
                if st.step >= 4 {
                    finish(cfg, st)?;
                    return Ok(());
                }
                if st.step == 3 {
                    st.editing = true;
                    continue;
                }
                st.step += 1;
            }
            KeyCode::Left | KeyCode::Char('h') => match st.step {
                0 => {
                    st.lang = Lang::Zh;
                    i18n::set_lang(Lang::Zh);
                }
                1 => st.theme = (st.theme + 1) % THEMES.len(),
                2 => st.wave = !st.wave,
                4 => st.quality = (st.quality + QUALITIES.len() - 1) % QUALITIES.len(),
                _ => {}
            },
            KeyCode::Right | KeyCode::Char('l') => match st.step {
                0 => {
                    st.lang = Lang::En;
                    i18n::set_lang(Lang::En);
                }
                1 => st.theme = (st.theme + 1) % THEMES.len(),
                2 => st.wave = !st.wave,
                4 => st.quality = (st.quality + 1) % QUALITIES.len(),
                _ => {}
            },
            KeyCode::Char('j') | KeyCode::Down => st.step = (st.step + 1).min(4),
            KeyCode::Char('k') | KeyCode::Up => st.step = st.step.saturating_sub(1),
            _ => {}
        }
    }
}

/// 把向导结果写进 config.toml，并打上「已完成首次设置」的标记
fn finish(cfg: &Config, st: &State) -> anyhow::Result<()> {
    let path = &cfg.config_path;
    let raw = std::fs::read_to_string(path).unwrap_or_default();
    let mut doc: toml::Value = raw
        .parse()
        .unwrap_or_else(|_| toml::Value::Table(Default::default()));
    if !doc.is_table() {
        doc = toml::Value::Table(Default::default());
    }
    let table = doc.as_table_mut().unwrap();
    let general = table
        .entry("general")
        .or_insert_with(|| toml::Value::Table(Default::default()));
    if let Some(g) = general.as_table_mut() {
        g.insert("first_run_done".into(), toml::Value::Boolean(true));
        g.insert(
            "language".into(),
            toml::Value::String(match st.lang {
                Lang::Zh => "zh",
                Lang::En => "en",
            }
            .into()),
        );
        g.insert("download_dir".into(), toml::Value::String(st.dir.clone()));
        g.insert(
            "quality".into(),
            toml::Value::String(QUALITIES[st.quality].to_string()),
        );
    }
    let ui = table
        .entry("ui")
        .or_insert_with(|| toml::Value::Table(Default::default()));
    if let Some(u) = ui.as_table_mut() {
        u.insert("theme".into(), toml::Value::String(THEMES[st.theme].into()));
        u.insert("equalizer".into(), toml::Value::Boolean(st.wave));
    }
    if let Some(dir) = std::path::Path::new(&st.dir).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(path, toml::to_string_pretty(&doc)?)?;
    Ok(())
}

fn draw(f: &mut ratatui::Frame, st: &State) {
    let area = f.area();
    f.render_widget(ratatui::widgets::Clear, area);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Span::styled(
            format!(" {} ", i18n::t("wizard.title")),
            Style::default()
                .fg(Color::Rgb(208, 188, 255))
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let rows = 10u16; // 5 项 × (标签 + 取值)
    let top = inner.y + inner.height.saturating_sub(rows + 3) / 2;
    let body = Rect::new(inner.x + 4, top, inner.width.saturating_sub(8), rows);

    let label = |key: &str| -> Line<'static> {
        Line::from(Span::styled(
            i18n::t(key).to_string(),
            Style::default()
                .fg(Color::Rgb(230, 225, 229))
                .add_modifier(Modifier::BOLD),
        ))
    };
    let val = |text: String, sel: bool| -> Line<'static> {
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                text,
                if sel {
                    Style::default()
                        .fg(Color::Rgb(242, 231, 255))
                        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
                } else {
                    Style::default().fg(Color::Rgb(208, 188, 255))
                },
            ),
        ])
    };
    let lang_text = "中文  /  English".to_string();
    let theme_text = format!("{}  /  {}", THEMES[st.theme], THEMES[(st.theme + 1) % 2]);
    let wave_text = if st.wave {
        format!("{}  /  {}", i18n::t("common.on"), i18n::t("common.off"))
    } else {
        format!("{}  /  {}", i18n::t("common.off"), i18n::t("common.on"))
    };
    let dir_text = if st.editing {
        format!("{}_", st.dir)
    } else {
        st.dir.clone()
    };
    let lines = vec![
        label("wizard.lang"),
        val(lang_text, st.step == 0),
        label("wizard.theme"),
        val(theme_text, st.step == 1),
        label("wizard.wave"),
        val(wave_text, st.step == 2),
        label("wizard.dir"),
        val(dir_text, st.step == 3),
        label("wizard.quality"),
        val(
            format!("{}  ({}/5)", QUALITIES[st.quality], st.quality + 1),
            st.step == 4,
        ),
    ];
    f.render_widget(Paragraph::new(lines), body);
    let help = if st.step >= 3 {
        Line::from(Span::styled(
            format!(
                " {}   {}",
                i18n::t("wizard.next"),
                if st.step == 4 { i18n::t("wizard.done") } else { "" }
            ),
            Style::default().fg(Color::Rgb(147, 143, 153)),
        ))
    } else {
        Line::from(Span::styled(
            format!(" {}", i18n::t("wizard.next")),
            Style::default().fg(Color::Rgb(147, 143, 153)),
        ))
    };
    let help_area = Rect::new(
        inner.x + 2,
        inner.y + inner.height.saturating_sub(2),
        inner.width.saturating_sub(4),
        1,
    );
    f.render_widget(Paragraph::new(help).alignment(Alignment::Left), help_area);
}

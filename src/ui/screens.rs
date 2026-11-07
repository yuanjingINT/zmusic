use crate::downloader::DlStatus;
use crate::ui::theme;
use crate::ui::App;
use crate::util;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Cell, Gauge, List, ListItem, ListState, Paragraph, Row, Table, TableState,
};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

pub fn render_all(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let [top, content, bottom] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0), Constraint::Length(3)])
            .areas(area);
    render_top(f, top, app);
    match app.screen {
        crate::ui::ScreenKind::Menu => render_menu(f, content, app),
        crate::ui::ScreenKind::Search => render_search(f, content, app),
        crate::ui::ScreenKind::Songs => render_songs(f, content, app),
        crate::ui::ScreenKind::Albums => render_albums(f, content, app),
        crate::ui::ScreenKind::Playlists => render_playlists(f, content, app),
        crate::ui::ScreenKind::Library => render_library(f, content, app),
        crate::ui::ScreenKind::NowPlaying => render_now_playing(f, content, app),
        crate::ui::ScreenKind::Downloads => render_downloads(f, content, app),
        crate::ui::ScreenKind::Favorites => render_favorites(f, content, app),
        crate::ui::ScreenKind::SuperDl => render_superdl(f, content, app),
        crate::ui::ScreenKind::Settings => render_settings(f, content, app),
    }
    render_ctx_menu(f, app);
    render_bottom(f, bottom, app);
}

fn render_settings(f: &mut Frame, area: Rect, app: &mut App) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Span::styled(" 设置 ", theme::title()));
    let inner = block.inner(area);
    let usb = app.settings.is_usb();
    let mut items: Vec<ListItem> = Vec::new();
    let loc_line = Line::from(vec![
        Span::styled("下载位置:  ", theme::song_title()),
        Span::styled(
            "[ 本地电脑 ]",
            if !usb {
                theme::primary().add_modifier(ratatui::style::Modifier::BOLD)
            } else {
                theme::dim()
            },
        ),
        Span::styled("   /   ", theme::dim()),
        Span::styled(
            "[ U盘 ]",
            if usb {
                theme::primary().add_modifier(ratatui::style::Modifier::BOLD)
            } else {
                theme::dim()
            },
        ),
        Span::styled(
            format!(
                "     当前: {}",
                if usb {
                    app.settings.usb_mount.clone()
                } else {
                    app.cfg.download_dir.display().to_string()
                }
            ),
            theme::dim(),
        ),
    ]);
    items.push(ListItem::new(loc_line));
    if app.settings_usb_loading && app.settings_usb.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            "  (正在检测 U盘...)",
            theme::dim(),
        ))));
    } else if app.settings_usb.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            "  (未检测到 U盘, 插入后重新进入本界面)",
            theme::dim(),
        ))));
    } else {
        for (i, d) in app.settings_usb.iter().enumerate() {
            let mark = if usb && app.settings.usb_device == d.path {
                "☑"
            } else {
                "  "
            };
            items.push(ListItem::new(Line::from(vec![
                Span::styled(format!(" {} ", mark), theme::primary()),
                Span::styled(d.path.clone(), theme::song_title()),
                Span::styled(
                    format!(
                        "   {}  {}  {}",
                        d.size_gb(),
                        d.model,
                        d.mount.as_deref().unwrap_or("未挂载")
                    ),
                    theme::dim(),
                ),
            ])));
            let _ = i;
        }
    }
    let hint = Line::from(Span::styled(
        " 操作: ↑↓选择  行1(下载位置)按Enter/←→切换本地或U盘  选中U盘按Enter下载到该设备",
        theme::dim(),
    ));
    items.push(ListItem::new(hint));
    let (top, count) = window(app.settings_sel, items.len(), inner.height as usize);
    let list_items: Vec<ListItem> = items.iter().skip(top).take(count).cloned().collect();
    let sel = if count > 0 { Some(app.settings_sel - top) } else { None };
    let mut state = ListState::default();
    state.select(sel);
    let list = List::new(list_items)
        .block(block)
        .highlight_style(theme::highlight())
        .highlight_symbol("▸ ");
    f.render_stateful_widget(list, area, &mut state);
}

fn render_ctx_menu(f: &mut Frame, app: &mut App) {
    if !app.ctx_open {
        return;
    }
    let Some(rect) = app.ctx_rect() else { return };
    let items = app.ctx_items();
    let list_items: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let style = if i == app.ctx_sel {
                theme::highlight()
            } else {
                Style::default()
            };
            ListItem::new(Line::from(Span::styled(*s, style)))
        })
        .collect();
    let menu = List::new(list_items).block(
        Block::bordered()
            .border_type(BorderType::Rounded)
            .title(Span::styled(" 右键菜单 ", theme::title())),
    );
    f.render_widget(menu, rect);
}

fn render_top(f: &mut Frame, area: Rect, app: &mut App) {
    let mut line = Line::from(Span::styled(
        format!(" zmusic · {}", app.screen.name()),
        theme::title(),
    ));
    if let Some((msg, _)) = &app.toast {
        line.push_span(Span::styled(format!("   {}", msg), theme::warn()));
    }
    f.render_widget(Paragraph::new(line), area);
}

fn render_bottom(f: &mut Frame, area: Rect, app: &mut App) {
    let st = app.player.state();
    let (song, dur) = match &app.now_playing {
        Some(s) => (
            format!("{} - {}", s.name, util::artists_of(&s.ar)),
            st.duration,
        ),
        None => ("未在播放".to_string(), st.duration),
    };
    let play = if st.paused { "⏸" } else { "▶" };
    let mut line1 = Line::from(vec![
        Span::styled(format!(" {} ", play), theme::primary()),
        Span::styled(song, theme::song_title()),
        Span::styled(
            format!(
                "  {} / {}  vol {}",
                util::fmt_time(st.time_pos),
                util::fmt_time(dur),
                st.volume
            ),
            theme::dim(),
        ),
    ]);
    if app.now_playing.is_some() {
        if let Some(idx) = app.queue_idx {
            line1.push_span(Span::styled(
                format!("   [{} / {}]", idx + 1, app.queue.len()),
                theme::dim(),
            ));
        }
    }
    let ratio = match (st.time_pos, dur) {
        (Some(t), Some(d)) if d > 0.0 => (t / d).clamp(0.0, 1.0),
        _ => 0.0,
    };
    let gauge = Gauge::default()
        .gauge_style(Style::new().fg(ratatui::style::Color::Cyan))
        .ratio(ratio)
        .label(format!("{:.1}%", ratio * 100.0));
    let sel_n = app.selected.len();
    let sel_hint = if sel_n > 0 {
        format!(" 已选 {} 首 ", sel_n)
    } else {
        String::new()
    };
    let hints = Line::from(Span::styled(
        format!(
            " h菜单 s搜索 v歌词 Alt+Enter选择 d下载(选中) l收藏 x移除收藏 Esc返回 n/p切歌 空格暂停 q退出{}",
            sel_hint
        ),
        theme::dim(),
    ));
    let [r1, r2, r3] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Length(1)])
            .areas(area);
    f.render_widget(Paragraph::new(line1), r1);
    f.render_widget(gauge, r2);
    f.render_widget(Paragraph::new(hints), r3);
}

fn render_center(f: &mut Frame, area: Rect, text: &str) {
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(text, theme::dim())).alignment(Alignment::Center))
            .alignment(Alignment::Center),
        area,
    );
}

fn window(sel: usize, len: usize, h: usize) -> (usize, usize) {
    let h = h.max(1);
    let top = if len == 0 {
        0
    } else if sel >= h {
        (sel - h + 1).min(len.saturating_sub(h))
    } else {
        0
    };
    let count = len.saturating_sub(top).min(h);
    (top, count)
}

fn render_menu(f: &mut Frame, area: Rect, app: &mut App) {
    let items: Vec<ListItem> = crate::ui::MENU_ITEMS
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let num = Span::styled(format!(" {} ", i + 1), theme::primary());
            let text = Span::styled(*name, theme::song_title());
            ListItem::new(Line::from(vec![num, text]))
        })
        .collect();
    let mut state = ListState::default();
    state.select(Some(app.menu_sel));
    let list = List::new(items)
        .highlight_style(theme::highlight())
        .highlight_symbol("▸ ")
        .block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(Span::styled(" 功能模块 ", theme::title())),
        );
    f.render_stateful_widget(list, area, &mut state);
}

fn render_playlists(f: &mut Frame, area: Rect, app: &mut App) {
    if app.playlists_loading && app.playlists.is_empty() {
        render_center(f, area, "歌单推荐加载中...");
        return;
    }
    if app.playlists.is_empty() {
        render_center(f, area, "暂无歌单 (按 d 刷新?)");
        return;
    }
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Span::styled("歌单推荐", theme::title()));
    let inner = block.inner(area);
    let (top, count) = window(app.playlists_sel, app.playlists.len(), inner.height as usize);
    let items: Vec<ListItem> = app
        .playlists
        .iter()
        .skip(top)
        .take(count)
        .map(|p| {
            let meta = format!(
                "  {}播放 · {}首",
                util::fmt_count(p.play_count),
                p.track_count
            );
            ListItem::new(Line::from(vec![
                Span::raw(p.name.clone()),
                Span::styled(meta, theme::dim()),
            ]))
        })
        .collect();
    let sel = if count > 0 { Some(app.playlists_sel - top) } else { None };
    let mut state = ListState::default();
    state.select(sel);
    let list = List::new(items)
        .block(block)
        .highlight_style(theme::highlight())
        .highlight_symbol("▸ ");
    f.render_stateful_widget(list, area, &mut state);
}

fn render_songs(f: &mut Frame, area: Rect, app: &mut App) {
    if app.songs_loading && app.songs.is_empty() {
        render_center(f, area, "歌曲推荐加载中...");
        return;
    }
    if app.songs.is_empty() {
        render_center(f, area, "暂无推荐歌曲");
        return;
    }
    render_song_table(f, area, &app.songs, app.songs_sel, &app.selected, "歌曲推荐");
}

fn render_albums(f: &mut Frame, area: Rect, app: &mut App) {
    if app.albums_loading && app.albums.is_empty() {
        render_center(f, area, "专辑推荐加载中...");
        return;
    }
    if app.albums.is_empty() {
        render_center(f, area, "暂无推荐专辑");
        return;
    }
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Span::styled("专辑推荐", theme::title()));
    let inner = block.inner(area);
    let (top, count) = window(app.albums_sel, app.albums.len(), inner.height as usize);
    let items: Vec<ListItem> = app
        .albums
        .iter()
        .skip(top)
        .take(count)
        .map(|a| {
            let artist = a
                .artists
                .iter()
                .map(|x| x.name.clone())
                .collect::<Vec<_>>()
                .join(" & ");
            let artist = if artist.is_empty() {
                a.artist.as_ref().map(|x| x.name.clone()).unwrap_or_default()
            } else {
                artist
            };
            let year = a
                .publish_time
                .map(util::year_of_ms)
                .unwrap_or_default();
            ListItem::new(Line::from(vec![
                Span::raw(a.name.clone()),
                Span::styled(format!("  {}  {}", artist, year), theme::dim()),
            ]))
        })
        .collect();
    let sel = if count > 0 { Some(app.albums_sel - top) } else { None };
    let mut state = ListState::default();
    state.select(sel);
    let list = List::new(items)
        .block(block)
        .highlight_style(theme::highlight())
        .highlight_symbol("▸ ");
    f.render_stateful_widget(list, area, &mut state);
}

fn render_search(f: &mut Frame, area: Rect, app: &mut App) {
    let [input_area, type_area, list_area] =
        Layout::vertical([Constraint::Length(3), Constraint::Length(1), Constraint::Min(0)])
            .areas(area);
    let style = if app.search_input_focus {
        theme::primary()
    } else {
        Style::default()
    };
    let input = Paragraph::new(Line::from(Span::styled(
        format!(" 🔍 {}", app.search_input),
        style,
    )))
    .block(
        Block::bordered()
            .border_type(BorderType::Rounded)
            .title(Line::from("搜索")),
    );
    f.render_widget(input, input_area);
    if app.search_input_focus {
        let x = (input_area.x + 1 + app.search_input.width() as u16)
            .min(input_area.right().saturating_sub(1));
        f.set_cursor_position((x, input_area.y + 1));
    }
    let types = ["0歌曲", "1歌手", "2歌单", "3专辑"];
    let line = Line::from(
        types
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let s = if i == app.search_type {
                    theme::primary().add_modifier(ratatui::style::Modifier::BOLD)
                } else {
                    theme::dim()
                };
                Span::styled(format!("  {} ", t), s)
            })
            .collect::<Vec<_>>(),
    );
    f.render_widget(Paragraph::new(line), type_area);
    if app.search_loading {
        render_center(f, list_area, "搜索中...");
        return;
    }
    match app.search_type {
        0 => {
            if app.search.is_empty() {
                render_center(f, list_area, "输入关键词回车搜索歌曲, Alt+Enter 多选, d 下载, l 收藏");
                return;
            }
            render_song_table(f, list_area, &app.search, app.search_sel, &app.selected, "搜索结果");
        }
        1 => {
            if app.search_artists.is_empty() {
                render_center(f, list_area, "输入关键词回车搜索歌手");
                return;
            }
            render_artists(f, list_area, &app.search_artists, app.search_sel);
        }
        2 => {
            if app.search_playlists.is_empty() {
                render_center(f, list_area, "输入关键词回车搜索歌单");
                return;
            }
            render_playlists_list(f, list_area, &app.search_playlists, app.search_sel);
        }
        3 => {
            if app.search_albums.is_empty() {
                render_center(f, list_area, "输入关键词回车搜索专辑");
                return;
            }
            render_albums_list(f, list_area, &app.search_albums, app.search_sel);
        }
        _ => {}
    }
}

fn render_artists(
    f: &mut Frame,
    area: Rect,
    artists: &[crate::models::ArtistInfo],
    sel: usize,
) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Span::styled("搜索歌手", theme::title()));
    let inner = block.inner(area);
    let (top, count) = window(sel, artists.len(), inner.height as usize);
    let items: Vec<ListItem> = artists
        .iter()
        .skip(top)
        .take(count)
        .map(|a| {
            let meta = format!("  歌曲 {} · 专辑 {}", a.music_size, a.album_size);
            ListItem::new(Line::from(vec![
                Span::raw(a.name.clone()),
                Span::styled(meta, theme::dim()),
            ]))
        })
        .collect();
    let sel = if count > 0 { Some(sel - top) } else { None };
    let mut state = ListState::default();
    state.select(sel);
    let list = List::new(items)
        .block(block)
        .highlight_style(theme::highlight())
        .highlight_symbol("▸ ");
    f.render_stateful_widget(list, area, &mut state);
}

fn render_playlists_list(f: &mut Frame, area: Rect, pls: &[crate::models::Playlist], sel: usize) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Span::styled("搜索歌单", theme::title()));
    let inner = block.inner(area);
    let (top, count) = window(sel, pls.len(), inner.height as usize);
    let items: Vec<ListItem> = pls
        .iter()
        .skip(top)
        .take(count)
        .map(|p| {
            let meta = format!(
                "  {}播放 · {}首",
                util::fmt_count(p.play_count),
                p.track_count
            );
            ListItem::new(Line::from(vec![
                Span::raw(p.name.clone()),
                Span::styled(meta, theme::dim()),
            ]))
        })
        .collect();
    let sel = if count > 0 { Some(sel - top) } else { None };
    let mut state = ListState::default();
    state.select(sel);
    let list = List::new(items)
        .block(block)
        .highlight_style(theme::highlight())
        .highlight_symbol("▸ ");
    f.render_stateful_widget(list, area, &mut state);
}

fn render_albums_list(f: &mut Frame, area: Rect, albs: &[crate::models::RecAlbum], sel: usize) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Span::styled("搜索专辑", theme::title()));
    let inner = block.inner(area);
    let (top, count) = window(sel, albs.len(), inner.height as usize);
    let items: Vec<ListItem> = albs
        .iter()
        .skip(top)
        .take(count)
        .map(|a| {
            let artist = a
                .artists
                .iter()
                .map(|x| x.name.clone())
                .collect::<Vec<_>>()
                .join(" & ");
            let artist = if artist.is_empty() {
                a.artist.as_ref().map(|x| x.name.clone()).unwrap_or_default()
            } else {
                artist
            };
            let year = a.publish_time.map(util::year_of_ms).unwrap_or_default();
            ListItem::new(Line::from(vec![
                Span::raw(a.name.clone()),
                Span::styled(format!("  {}  {}", artist, year), theme::dim()),
            ]))
        })
        .collect();
    let sel = if count > 0 { Some(sel - top) } else { None };
    let mut state = ListState::default();
    state.select(sel);
    let list = List::new(items)
        .block(block)
        .highlight_style(theme::highlight())
        .highlight_symbol("▸ ");
    f.render_stateful_widget(list, area, &mut state);
}

fn render_library(f: &mut Frame, area: Rect, app: &mut App) {
    if app.lib_loading {
        render_center(f, area, "加载中...");
        return;
    }
    if app.lib.is_empty() {
        render_center(f, area, "暂无内容");
        return;
    }
    render_song_table(f, area, &app.lib, app.lib_sel, &app.selected, &app.lib_title);
}

fn render_favorites(f: &mut Frame, area: Rect, app: &mut App) {
    let title = if app.fav_view == 1 {
        format!("收藏的歌单 ({} 个)", app.favorites.playlist_len())
    } else {
        format!("收藏的歌曲 ({} 首, 按 t 切换歌单)", app.favorites.len())
    };
    if app.fav_view == 1 {
        let pls = app.favorites.playlists();
        if pls.is_empty() {
            render_center(f, area, "暂无收藏歌单 (在歌单界面按 Alt+Enter/右键收藏, t 切换)");
            return;
        }
        render_playlists_list(f, area, pls, app.fav_sel);
        return;
    }
    if app.favorites.len() == 0 {
        render_center(f, area, "暂无收藏歌曲 (在歌曲列表按 Alt+Enter 多选, l 收藏, 右键菜单也可收藏)");
        return;
    }
    render_song_table(f, area, app.favorites.list(), app.fav_sel, &app.selected, &title);
}

fn render_song_table(
    f: &mut Frame,
    area: Rect,
    songs: &[crate::models::Song],
    sel: usize,
    selected: &std::collections::HashSet<i64>,
    title: &str,
) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Span::styled(title, theme::title()));
    let inner = block.inner(area);
    let (top, count) = window(sel, songs.len(), inner.height as usize);
    let rows: Vec<Row> = songs
        .iter()
        .skip(top)
        .take(count)
        .enumerate()
        .map(|(i, s)| {
            let mark = if selected.contains(&s.id) {
                Span::styled("☑", theme::primary())
            } else {
                Span::styled("  ", theme::dim())
            };
            Row::new(vec![
                Cell::from(mark),
                Cell::from(Span::styled(
                    format!("{:>3}", top + i + 1),
                    theme::dim(),
                )),
                Cell::from(s.name.clone()),
                Cell::from(util::artists_of(&s.ar)),
                Cell::from(s.al.name.clone()),
                Cell::from(util::fmt_time(Some(s.dt as f64 / 1000.0))),
            ])
        })
        .collect();
    let header = Row::new(["选", "#", "歌名", "歌手", "专辑", "时长"].map(Cell::from))
        .style(theme::title());
    let widths = [
        Constraint::Length(3),
        Constraint::Length(4),
        Constraint::Min(14),
        Constraint::Min(10),
        Constraint::Min(10),
        Constraint::Length(6),
    ];
    let mut ts = TableState::default();
    ts.select(Some(sel.saturating_sub(top)));
    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .row_highlight_style(theme::highlight());
    f.render_stateful_widget(table, area, &mut ts);
}

fn render_now_playing(f: &mut Frame, area: Rect, app: &mut App) {
    let Some(song) = app.now_playing.clone() else {
        render_center(f, area, "未在播放");
        return;
    };
    let st = app.player.state();
    let [info, lyric_area] =
        Layout::vertical([Constraint::Length(4), Constraint::Min(0)]).areas(area);
    let title = format!("  {} ", song.name);
    let sub = format!("  {} · {} ", util::artists_of(&song.ar), song.al.name);
    let ratio = match (st.time_pos, st.duration) {
        (Some(t), Some(d)) if d > 0.0 => (t / d).clamp(0.0, 1.0),
        _ => 0.0,
    };
    let gauge = Gauge::default()
        .gauge_style(Style::new().fg(ratatui::style::Color::Cyan))
        .ratio(ratio)
        .label(format!(
            "{} / {}",
            util::fmt_time(st.time_pos),
            util::fmt_time(st.duration)
        ));
    let [ti, ar, ga] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Length(1)])
            .areas(info);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(title, theme::song_title())).alignment(Alignment::Center)),
        ti,
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(sub, theme::dim())).alignment(Alignment::Center)),
        ar,
    );
    f.render_widget(gauge, ga);
    render_lyrics(f, lyric_area, app, st.time_pos);
}

fn render_lyrics(f: &mut Frame, area: Rect, app: &mut App, time_pos: Option<f64>) {
    let Some(lyr) = &app.lyric else {
        render_center(f, area, "暂无歌词");
        return;
    };
    if lyr.lines.is_empty() {
        render_center(f, area, "暂无歌词");
        return;
    }
    let cur = match time_pos {
        Some(t) => {
            let target = t * 1000.0;
            lyr.lines
                .partition_point(|(ms, _)| *ms as f64 <= target)
                .saturating_sub(1)
        }
        None => 0,
    };
    let h = area.height as usize;
    let half = h / 2;
    let start = cur.saturating_sub(half);
    let end = (start + h).min(lyr.lines.len());
    let mut lines: Vec<Line> = Vec::new();
    for i in start..end {
        let (ms, text) = &lyr.lines[i];
        let style = if i == cur {
            theme::lyric_current()
        } else {
            theme::lyric_normal()
        };
        let mut line = Line::from(Span::styled(text.clone(), style));
        if i == cur {
            if let Some((_, t)) = lyr
                .trans
                .iter()
                .find(|(tm, _)| (*tm as i64 - *ms as i64).abs() < 300)
            {
                line.push_span(Span::styled(format!("  {}", t), theme::dim()));
            }
        }
        line.alignment = Some(Alignment::Center);
        lines.push(line);
    }
    f.render_widget(Paragraph::new(lines), area);
}

fn render_superdl(f: &mut Frame, area: Rect, app: &mut App) {
    if app.super_running {
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .title(Span::styled(" 超级下载进行中 ", theme::warn()));
        let inner = block.inner(area);
        let lines: Vec<Line> = app
            .super_lines
            .iter()
            .rev()
            .take(inner.height as usize)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|s| Line::from(Span::styled(s.clone(), theme::dim())))
            .collect();
        f.render_widget(Paragraph::new(lines).block(block), area);
        return;
    }
    if app.super_loading {
        render_center(f, area, "正在检测 USB 设备...");
        return;
    }
    if app.super_devices.is_empty() {
        render_center(f, area, "未检测到 USB 设备 (插入 U 盘后重新进入本界面, 格式化需要 sudo 密码)");
        return;
    }
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Span::styled(" 选择 USB 设备 (Enter = 格式化并下载热榜音乐, 数据将丢失!) ", theme::err()));
    let inner = block.inner(area);
    let (top, count) = window(app.super_sel, app.super_devices.len(), inner.height as usize);
    let items: Vec<ListItem> = app
        .super_devices
        .iter()
        .skip(top)
        .take(count)
        .map(|d| {
            let info = format!(
                "  {}  {}  {}  {}",
                d.size_gb(),
                d.model,
                d.fs.as_deref().unwrap_or("无文件系统"),
                d.mount.as_deref().unwrap_or("未挂载")
            );
            ListItem::new(Line::from(vec![
                Span::styled(d.path.clone(), theme::primary()),
                Span::styled(info, theme::dim()),
            ]))
        })
        .collect();
    let sel = if count > 0 { Some(app.super_sel - top) } else { None };
    let mut state = ListState::default();
    state.select(sel);
    let list = List::new(items)
        .block(block)
        .highlight_style(theme::highlight())
        .highlight_symbol("▸ ");
    f.render_stateful_widget(list, area, &mut state);
}

fn render_downloads(f: &mut Frame, area: Rect, app: &mut App) {
    let tasks = app.downloader.snapshot();
    let (total, done, skipped, failed) = app.downloader.counts();
    let title = format!(
        "下载任务  (完成 {} / 跳过 {} / 失败 {} / 共 {})",
        done, skipped, failed, total
    );
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Span::styled(title, theme::title()));
    if tasks.is_empty() {
        render_center(f, area, "暂无下载任务 (在歌曲列表按 d 下载单曲, D 批量下载)");
        return;
    }
    let inner = block.inner(area);
    let (top, count) = window(0, tasks.len(), inner.height as usize);
    let items: Vec<ListItem> = tasks
        .iter()
        .skip(top)
        .take(count)
        .map(|t| {
            let (icon, style) = match &t.status {
                DlStatus::Queued => ("·", theme::dim()),
                DlStatus::Resolving => ("↻", theme::warn()),
                DlStatus::Downloading(_) => ("↓", theme::primary()),
                DlStatus::Done => ("✓", theme::good()),
                DlStatus::Skipped => ("⇥", theme::dim()),
                DlStatus::Failed(_) => ("✗", theme::err()),
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!(" {} ", icon), style),
                Span::raw(t.title.clone()),
                Span::styled(format!("  {}", t.label()), style),
            ]))
        })
        .collect();
    f.render_widget(List::new(items).block(block), area);
}

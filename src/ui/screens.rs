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
    let [top, content, bottom] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(3),
    ])
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
    render_song_cfg(f, app);
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
    items.push(ListItem::new(Line::from(vec![
        Span::styled("配色:      ", theme::song_title()),
        Span::styled(theme::palette_name(), theme::dim()),
    ])));
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
    let sel = if count > 0 {
        Some(app.settings_sel - top)
    } else {
        None
    };
    let mut state = ListState::default();
    state.select(sel);
    let list = List::new(list_items)
        .block(block)
        .highlight_style(theme::highlight())
        .highlight_symbol("▸ ");
    f.render_stateful_widget(list, area, &mut state);
}

fn render_song_cfg(f: &mut Frame, app: &mut App) {
    if !app.song_cfg_open {
        return;
    }
    let items = app.song_cfg_items();
    let area = f.area();
    let w = 52u16.min(area.width);
    let h = (items.len() as u16 + 2).min(area.height);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let rect = Rect::new(x, y, w, h);
    f.render_widget(ratatui::widgets::Clear, rect);

    let rows: Vec<ListItem> = items
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let style = if i == app.song_cfg_sel {
                theme::highlight()
            } else {
                Style::default()
            };
            ListItem::new(Line::from(Span::styled(format!(" {} ", s), style)))
        })
        .collect();
    let menu = List::new(rows).block(Block::bordered().border_type(BorderType::Rounded).title(
        Span::styled(" 当前歌曲 · Enter 应用 / Esc 关闭 ", theme::title()),
    ));
    f.render_widget(menu, rect);
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
    let mut spans = Vec::with_capacity(4);
    spans.push(Span::styled(" zmusic · ", theme::title()));
    spans.push(Span::styled(app.screen.name(), theme::title()));
    if let Some(st) = app.probe_status() {
        spans.push(Span::styled("   ", theme::dim()));
        spans.push(Span::styled(st, theme::dim()));
    }
    if let Some((msg, _)) = &app.toast {
        spans.push(Span::styled("   ", theme::warn()));
        spans.push(Span::styled(msg.as_str(), theme::warn()));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_bottom(f: &mut Frame, area: Rect, app: &mut App) {
    let st = app.player.state();
    let dur = st.duration;
    // 渲染热路径：文本拼进两块复用缓冲区，避免每帧 6 次 format! 的堆分配
    let mut artists_buf = String::with_capacity(64);
    let mut meta_buf = String::with_capacity(64);
    let mut spans: Vec<Span> = Vec::with_capacity(5);
    let elapsed_ms = app.started.elapsed().as_millis() as u64;
    let spin = crate::ui::anim::spinner(app.anim.spinner_on(), app.is_loading(), elapsed_ms);
    if let Some(frame) = spin {
        spans.push(Span::styled(format!(" {frame} "), theme::primary()));
    }
    spans.push(Span::styled(
        if st.paused { " ⏸ " } else { " ▶ " },
        if st.paused {
            theme::dim()
        } else {
            theme::playing()
        },
    ));
    match &app.now_playing {
        Some(s) => {
            spans.push(Span::styled(s.name.as_str(), theme::song_title()));
            artists_buf.push_str(" - ");
            util::artists_of_into(&mut artists_buf, &s.ar);
            spans.push(Span::styled(artists_buf.as_str(), theme::song_title()));
        }
        None => spans.push(Span::styled("未在播放", theme::song_title())),
    }
    use std::fmt::Write as _;
    let mut tbuf = String::with_capacity(8);
    util::fmt_time_into(&mut tbuf, st.time_pos);
    meta_buf.push_str("  ");
    meta_buf.push_str(&tbuf);
    meta_buf.push_str(" / ");
    util::fmt_time_into(&mut tbuf, dur);
    meta_buf.push_str(&tbuf);
    let _ = write!(meta_buf, "  vol {}", st.volume);
    if let Some(idx) = app.queue_idx {
        let _ = write!(meta_buf, "   [{} / {}]", idx + 1, app.queue.len());
    }
    let line1 = Line::from(spans);
    let ratio = match (st.time_pos, dur) {
        (Some(t), Some(d)) if d > 0.0 => (t / d).clamp(0.0, 1.0),
        _ => 0.0,
    };
    let gauge = Gauge::default()
        .gauge_style(theme::gauge_style())
        .ratio(ratio)
        .label(format!("{:.1}%", ratio * 100.0));
    let mut hint_spans = Vec::with_capacity(2);
    hint_spans.push(Span::styled(
        crate::ui::hints::hints(app.screen, app.search_input_focus),
        theme::dim(),
    ));
    let sel_n = app.selected.len();
    let sel_buf = if sel_n > 0 {
        let mut b = String::with_capacity(16);
        let _ = write!(b, "  已选 {} 首 ", sel_n);
        b
    } else {
        String::new()
    };
    let hints = Line::from(hint_spans);
    let [r1, r2, r3] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);
    // 状态栏分区：左=播放状态/歌名，右=时间·音量·队列（右对齐，不被长歌名挤掉）
    let meta_w = (UnicodeWidthStr::width(meta_buf.as_str()) as u16).min(r1.width);
    let [left, right] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(meta_w)]).areas(r1);
    f.render_widget(Paragraph::new(line1), left);
    f.render_widget(
        Paragraph::new(
            Line::from(Span::styled(meta_buf.as_str(), theme::dim())).alignment(Alignment::Right),
        ),
        right,
    );
    // 正在播放页自己有一条带时间/总长的进度条，底栏再画一条就重复了
    let render_hint_row = |f: &mut Frame, row: Rect| {
        if sel_buf.is_empty() {
            f.render_widget(Paragraph::new(hints), row);
            return;
        }
        let sel_w = (UnicodeWidthStr::width(sel_buf.as_str()) as u16).min(row.width);
        let [l, r] = Layout::horizontal([Constraint::Min(0), Constraint::Length(sel_w)]).areas(row);
        f.render_widget(Paragraph::new(hints), l);
        f.render_widget(
            Paragraph::new(
                Line::from(Span::styled(sel_buf.as_str(), theme::primary()))
                    .alignment(Alignment::Right),
            ),
            r,
        );
    };
    let progress = |f: &mut Frame, row: Rect| {
        if app.anim.smooth_progress_on() {
            let ratio = match (st.time_pos, dur) {
                (Some(t), Some(d)) if d > 0.0 => (t / d).clamp(0.0, 1.0),
                _ => 0.0,
            };
            f.render_widget(
                Paragraph::new(smooth_progress_line(ratio, row.width as usize)),
                row,
            );
        } else {
            f.render_widget(gauge.clone(), row);
        }
    };
    if app.screen == crate::ui::ScreenKind::NowPlaying {
        render_hint_row(f, r2);
    } else {
        progress(f, r2);
        render_hint_row(f, r3);
    }
}

/// 平滑进度条：已播主题色 █、播放头 ▶、未播低对比度 ░（按 time-pos 每帧更新）
fn smooth_progress_line(ratio: f64, width: usize) -> Line<'static> {
    let w = width.max(8);
    let filled = ((w as f64 * ratio.clamp(0.0, 1.0)).round() as usize).min(w.saturating_sub(1));
    let mut played = String::with_capacity(filled);
    for _ in 0..filled {
        played.push('█');
    }
    let mut rest = String::with_capacity(w - filled);
    rest.push('░');
    for _ in 0..(w - filled - 1) {
        rest.push('░');
    }
    Line::from(vec![
        Span::styled(played, theme::primary()),
        Span::styled("▶", theme::playing()),
        Span::styled(rest, theme::dim()),
    ])
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
            let num = Span::styled(format!(" {i} "), theme::primary());
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
    let (top, count) = window(
        app.playlists_sel,
        app.playlists.len(),
        inner.height as usize,
    );
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
                Span::raw(p.name.as_str()),
                Span::styled(meta, theme::dim()),
            ]))
        })
        .collect();
    let sel = if count > 0 {
        Some(app.playlists_sel - top)
    } else {
        None
    };
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
    let tag = |s: &crate::models::Song| app.source_badge(s);
    render_song_table(
        f,
        area,
        &app.songs,
        app.songs_sel,
        &app.selected,
        "歌曲推荐",
        Some(&tag),
    );
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
                a.artist
                    .as_ref()
                    .map(|x| x.name.clone())
                    .unwrap_or_default()
            } else {
                artist
            };
            let year = a.publish_time.map(util::year_of_ms).unwrap_or_default();
            ListItem::new(Line::from(vec![
                Span::raw(a.name.as_str()),
                Span::styled(format!("  {}  {}", artist, year), theme::dim()),
            ]))
        })
        .collect();
    let sel = if count > 0 {
        Some(app.albums_sel - top)
    } else {
        None
    };
    let mut state = ListState::default();
    state.select(sel);
    let list = List::new(items)
        .block(block)
        .highlight_style(theme::highlight())
        .highlight_symbol("▸ ");
    f.render_stateful_widget(list, area, &mut state);
}

fn render_search(f: &mut Frame, area: Rect, app: &mut App) {
    let [input_area, type_area, list_area] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(area);
    let style = if app.search_input_focus {
        theme::primary()
    } else {
        Style::default()
    };
    let input = Paragraph::new(Line::from(Span::styled(
        format!(" {} ", app.search_input),
        style,
    )))
    .block(
        Block::bordered()
            .border_type(BorderType::Rounded)
            .title(Line::from("搜索")),
    );
    f.render_widget(input, input_area);
    if app.search_input_focus {
        let x = (input_area.x + 2 + app.search_input.width() as u16)
            .min(input_area.right().saturating_sub(1));
        f.set_cursor_position((x, input_area.y + 1));
    }
    let (src_name, src_ok) = app.current_source();
    let src_name = src_name.as_str();
    let types = ["0歌曲", "1歌手", "2歌单", "3专辑"];
    let mut spans: Vec<Span> = Vec::with_capacity(6);
    spans.push(Span::styled(
        format!(" 源:{} ", src_name),
        if src_ok {
            theme::primary().add_modifier(ratatui::style::Modifier::BOLD)
        } else {
            theme::dim()
        },
    ));
    spans.extend(
        types
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let supported = app
                    .source_caps()
                    .map(|c| c.supports(crate::source::SearchKind::from_index(i)))
                    .unwrap_or(true);
                let s = if i == app.search_type {
                    theme::primary().add_modifier(ratatui::style::Modifier::BOLD)
                } else if !supported {
                    theme::dim().add_modifier(ratatui::style::Modifier::CROSSED_OUT)
                } else {
                    theme::dim()
                };
                Span::styled(format!("  {} ", t), s)
            })
            .collect::<Vec<_>>(),
    );
    if app.source_id == crate::ui::AGGREGATE_ID && !app.agg_failed.is_empty() {
        let mut names = String::new();
        for (i, f) in app.agg_failed.iter().take(4).enumerate() {
            if i > 0 {
                names.push('，');
            }
            names.push_str(f.as_str());
        }
        if app.agg_failed.len() > 4 {
            names.push('…');
        }
        spans.push(Span::styled(format!("   失败源: {names}"), theme::warn()));
    }
    spans.push(Span::styled(
        if app.search_input_focus {
            "   Tab→列表 · F1-F4/Alt+数字 换类型"
        } else {
            "   Tab→输入栏 · 直接打字即回输入 · Enter 播放所选"
        },
        theme::dim(),
    ));
    f.render_widget(Paragraph::new(Line::from(spans)), type_area);
    if app.search_loading {
        // 探测还在跑时把进度也带上，别只显示一个「搜索中」
        let msg = format!("{} → 搜索中…", app.source_status_suffix());
        render_center(f, list_area, &msg);
        return;
    }
    // 结果为空、或按 V 切过来时，整块显示逐行的源状态明细
    if app.source_view_active() {
        render_source_details(f, list_area, app);
        return;
    }
    match app.search_type {
        0 => {
            if app.search.is_empty() {
                render_center(
                    f,
                    list_area,
                    "输入关键词回车搜索歌曲；Tab 到列表；Alt+Enter 多选, d 下载, l 收藏",
                );
                return;
            }
            let tag = |s: &crate::models::Song| app.source_badge(s);
            // 源连通性摘要收在列表块标题行右侧
            let title = format!(
                "搜索结果   源 {}/{}",
                app.health_ok_count(),
                app.registry.len()
            );
            render_song_table(
                f,
                list_area,
                &app.search,
                app.search_sel,
                &app.selected,
                &title,
                Some(&tag),
            );
        }
        1 => {
            if app.search_artists.is_empty() {
                render_center(
                    f,
                    list_area,
                    "输入关键词回车搜索歌手；Tab 到列表，F2/Alt+1 换类型",
                );
                return;
            }
            let tag = |a: &crate::models::ArtistInfo| {
                app.item_sources(&crate::source::health::artist_key(a))
            };
            render_artists(
                f,
                list_area,
                &app.search_artists,
                app.search_sel,
                Some(&tag),
            );
        }
        2 => {
            if app.search_playlists.is_empty() {
                render_center(
                    f,
                    list_area,
                    "输入关键词回车搜索歌单；Tab 到列表，F3/Alt+2 换类型",
                );
                return;
            }
            let tag = |p: &crate::models::Playlist| {
                app.item_sources(&crate::source::health::playlist_key(p))
            };
            render_playlists_list(
                f,
                list_area,
                &app.search_playlists,
                app.search_sel,
                Some(&tag),
            );
        }
        3 => {
            if app.search_albums.is_empty() {
                render_center(
                    f,
                    list_area,
                    "输入关键词回车搜索专辑；Tab 到列表，F4/Alt+3 换类型",
                );
                return;
            }
            let tag = |a: &crate::models::RecAlbum| {
                app.item_sources(&crate::source::health::album_key(a))
            };
            render_albums_list(f, list_area, &app.search_albums, app.search_sel, Some(&tag));
        }
        _ => {}
    }
}

type ArtistTag<'a> = Option<&'a dyn Fn(&crate::models::ArtistInfo) -> Option<String>>;
type PlaylistTag<'a> = Option<&'a dyn Fn(&crate::models::Playlist) -> Option<String>>;
type AlbumTag<'a> = Option<&'a dyn Fn(&crate::models::RecAlbum) -> Option<String>>;

fn render_artists(
    f: &mut Frame,
    area: Rect,
    artists: &[crate::models::ArtistInfo],
    sel: usize,
    tag: ArtistTag<'_>,
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
            let mut spans = vec![Span::raw(a.name.clone()), Span::styled(meta, theme::dim())];
            if let Some(src) = tag.and_then(|t| t(a)) {
                spans.push(Span::styled(format!("  {src}"), theme::dim()));
            }
            ListItem::new(Line::from(spans))
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

fn render_playlists_list(
    f: &mut Frame,
    area: Rect,
    pls: &[crate::models::Playlist],
    sel: usize,
    tag: PlaylistTag<'_>,
) {
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
            let meta = if p.creator_name().is_empty() {
                format!(
                    "  {}播放 · {}首",
                    util::fmt_count(p.play_count),
                    p.track_count
                )
            } else {
                format!(
                    "  {} · {}播放 · {}首",
                    p.creator_name(),
                    util::fmt_count(p.play_count),
                    p.track_count
                )
            };
            let mut spans = vec![Span::raw(p.name.clone()), Span::styled(meta, theme::dim())];
            if let Some(src) = tag.and_then(|t| t(p)) {
                spans.push(Span::styled(format!("  {src}"), theme::dim()));
            }
            ListItem::new(Line::from(spans))
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

fn render_albums_list(
    f: &mut Frame,
    area: Rect,
    albs: &[crate::models::RecAlbum],
    sel: usize,
    tag: AlbumTag<'_>,
) {
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
                a.artist
                    .as_ref()
                    .map(|x| x.name.clone())
                    .unwrap_or_default()
            } else {
                artist
            };
            let year = a.publish_time.map(util::year_of_ms).unwrap_or_default();
            let mut spans = vec![
                Span::raw(a.name.clone()),
                Span::styled(format!("  {}  {}", artist, year), theme::dim()),
            ];
            if let Some(src) = tag.and_then(|t| t(a)) {
                spans.push(Span::styled(format!("  {src}"), theme::dim()));
            }
            ListItem::new(Line::from(spans))
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
    let tag = |s: &crate::models::Song| app.source_badge(s);
    render_song_table(
        f,
        area,
        &app.lib,
        app.lib_sel,
        &app.selected,
        &app.lib_title,
        Some(&tag),
    );
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
            render_center(
                f,
                area,
                "暂无收藏歌单 (在歌单界面按 Alt+Enter/右键收藏, t 切换)",
            );
            return;
        }
        render_playlists_list(f, area, pls, app.fav_sel, None);
        return;
    }
    if app.favorites.len() == 0 {
        render_center(
            f,
            area,
            "暂无收藏歌曲 (在歌曲列表按 Alt+Enter 多选, l 收藏, 右键菜单也可收藏)",
        );
        return;
    }
    let tag = |s: &crate::models::Song| app.source_badge(s);
    render_song_table(
        f,
        area,
        app.favorites.list(),
        app.fav_sel,
        &app.selected,
        &title,
        Some(&tag),
    );
}

/// 列表区里的逐行源状态：`● 网易云音乐  ok`、`○ YouTube  代理未配置`，末尾给汇总。
fn render_source_details(f: &mut Frame, area: Rect, app: &mut App) {
    use crate::ui::status::{self, Tone};
    let items = app.source_status_items();
    let suffix = app.source_status_suffix();
    let width = area.width.saturating_sub(2) as usize; // 去掉边框
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Span::styled(" 源状态 ", theme::title()));
    let mut rows: Vec<ListItem> = Vec::with_capacity(items.len() + 2);
    for (i, it) in items.iter().enumerate() {
        let reason = app.source_reason(it);
        let line = crate::ui::status::truncate(&status::detail_line(it, &reason), width);
        let mut style = match status::tone_of(it) {
            Tone::Good => theme::good(),
            Tone::Warn => theme::warn(),
            Tone::Dim => theme::dim(),
        };
        if it.current || i == app.source_sel {
            style = style.add_modifier(ratatui::style::Modifier::BOLD);
            if it.current {
                style = style.add_modifier(ratatui::style::Modifier::REVERSED);
            }
        }
        rows.push(ListItem::new(Line::from(Span::styled(line, style))));
    }
    rows.push(ListItem::new(Line::from(Span::styled(
        crate::ui::status::truncate(&format!("{suffix} · R 重探 · V 返回结果"), width),
        theme::dim(),
    ))));
    let mut state = ListState::default();
    if !items.is_empty() {
        state.select(Some(app.source_sel.min(items.len() - 1)));
    }
    let list = List::new(rows).block(block);
    f.render_stateful_widget(list, area, &mut state);
}

/// 聚合列表的「源」列：短名 + 能不能出流
type SourceTag<'a> = &'a dyn Fn(&crate::models::Song) -> (String, bool);

fn render_song_table(
    f: &mut Frame,
    area: Rect,
    songs: &[crate::models::Song],
    sel: usize,
    selected: &std::collections::HashSet<i64>,
    title: &str,
    // 每行标来源；`false` 的那行是「仅搜索」（不能出流），红色提示
    tag: Option<SourceTag<'_>>,
) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme::border_style(true))
        .title(Span::styled(title, theme::title()));
    let inner = block.inner(area);
    // 列宽按实际可用宽度算：中文窄终端也不会把「来源」挤没，长标题用 … 收尾
    let cols: usize = if tag.is_some() { 7 } else { 6 };
    let fixed: usize = 3 + 4 + 6; // 选 / # / 时长
    let src_w: usize = if tag.is_some() { 16 } else { 0 };
    let flex = (inner.width as usize)
        .saturating_sub(fixed + src_w + (cols - 1))
        .max(24);
    let (name_w, artist_w, album_w) = if tag.is_some() {
        let n = (flex * 40 / 100).max(8);
        let a = (flex * 28 / 100).max(6);
        (n, a, flex.saturating_sub(n + a).max(6))
    } else {
        let n = (flex * 44 / 100).max(10);
        let a = (flex * 28 / 100).max(8);
        (n, a, flex.saturating_sub(n + a).max(8))
    };
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
            let mut cells = vec![
                Cell::from(mark),
                Cell::from(Span::styled(format!("{:>3}", top + i + 1), theme::dim())),
                Cell::from(crate::ui::status::truncate(&s.name, name_w)),
                Cell::from(crate::ui::status::truncate(
                    &util::artists_of(&s.ar),
                    artist_w,
                )),
            ];
            if let Some(f) = tag {
                let (name, can_stream) = f(s);
                let label = if can_stream {
                    name
                } else {
                    format!("{name}·仅搜")
                };
                cells.push(Cell::from(Span::styled(
                    crate::ui::status::pad_left(&label, src_w.saturating_sub(2)) + " ",
                    if can_stream {
                        theme::source_tag()
                    } else {
                        theme::warn()
                    },
                )));
            }
            cells.push(Cell::from(crate::ui::status::truncate(&s.al.name, album_w)));
            cells.push(Cell::from(Span::styled(
                format!("{:>6}", util::fmt_time(Some(s.dt as f64 / 1000.0))),
                theme::dim(),
            )));
            Row::new(cells)
        })
        .collect();
    let headers: Vec<&str> = if tag.is_some() {
        vec!["选", "#", "歌名", "歌手", "源", "专辑", "时长"]
    } else {
        vec!["选", "#", "歌名", "歌手", "专辑", "时长"]
    };
    let header_cells: Vec<Cell> = headers
        .iter()
        .map(|h| {
            let right = *h == "源" || *h == "时长";
            let w = match *h {
                "源" => src_w.saturating_sub(2),
                "时长" => 4,
                _ => 0,
            };
            if right {
                Cell::from(Span::styled(
                    crate::ui::status::pad_left(h, w) + " ",
                    theme::title(),
                ))
            } else {
                Cell::from(Span::styled(*h, theme::title()))
            }
        })
        .collect();
    let header = Row::new(header_cells);
    let mut widths: Vec<Constraint> = vec![
        Constraint::Length(3),
        Constraint::Length(4),
        Constraint::Length(name_w as u16),
        Constraint::Length(artist_w as u16),
    ];
    if tag.is_some() {
        widths.push(Constraint::Length(src_w as u16));
    }
    widths.push(Constraint::Length(album_w as u16));
    widths.push(Constraint::Length(6));
    let mut ts = TableState::default();
    ts.select(Some(sel.saturating_sub(top)));
    let table = Table::new(rows, widths)
        .style(theme::fg())
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
    let rows_n = ((area.height as usize).saturating_sub(8) / 4).clamp(3, 5);
    let [head, spec, prog, lyric_area] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(rows_n as u16),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(area);
    let title = format!("  {} ", song.name);
    let sub = format!(
        "  {} · {} · {} ",
        util::artists_of(&song.ar),
        song.al.name,
        crate::source::source_display(&song.source)
    );
    let ratio = match (st.time_pos, st.duration) {
        (Some(t), Some(d)) if d > 0.0 => (t / d).clamp(0.0, 1.0),
        _ => 0.0,
    };
    let time_label = format!(
        "{} / {}",
        util::fmt_time(st.time_pos),
        util::fmt_time(st.duration)
    );
    // 切歌淡入：Dim → 中间档 → 正常，三帧一次性
    let fade = crate::ui::anim::fade_step(
        app.anim.fade_on(),
        app.fade_start.map(|t| t.elapsed().as_millis() as u64),
    );
    let title_style = match fade {
        Some(crate::ui::anim::FadeStep::Dim) => theme::dim(),
        Some(crate::ui::anim::FadeStep::Half) => theme::primary(),
        _ => theme::song_title(),
    };
    let [ti, ar] = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(head);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(title, title_style)).alignment(Alignment::Center)),
        ti,
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(sub, theme::dim())).alignment(Alignment::Center)),
        ar,
    );

    // 频谱：优先真解码数据，未就绪/降级时用慢呼吸示意波形；12 段（窄终端 8、宽屏 16）
    let playing = !st.idle && !st.paused && st.time_pos.is_some();
    let t_secs = app.started.elapsed().as_secs_f64();
    // 段数随宽度：宽屏 16、常规 12、窄屏 8、极窄 4（<40 列也不 panic）
    let n_bands = if area.width >= 100 {
        16
    } else if area.width >= 80 {
        12
    } else if area.width >= 40 {
        8
    } else {
        4
    }
    .min((area.width as usize / 2).max(2));
    let spectrum: Option<(Vec<u8>, Vec<u8>)> = if app.anim.equalizer_on() && playing {
        match app.eq_analyzer.as_ref().map(|a| a.frame()) {
            Some(f) if !f.levels.is_empty() => Some((
                crate::ui::eq::merge_bands(&f.levels, n_bands),
                crate::ui::eq::merge_bands(&f.peaks, n_bands),
            )),
            _ => {
                let lv = crate::ui::anim::breathing_levels(t_secs, n_bands);
                Some((lv.clone(), lv))
            }
        }
    } else {
        None
    };
    if let Some((levels, peaks)) = spectrum {
        // 2 格宽柱子 + 1 格间距，左右各留 2 格；宽度不够自动降级
        let margin = 2u16;
        let avail = area.width.saturating_sub(margin * 2) as usize;
        let (bands, band_w, gap) = crate::ui::anim::layout(avail, n_bands);
        let lv = crate::ui::eq::merge_bands(&levels, bands);
        let pk = crate::ui::eq::merge_bands(&peaks, bands);
        let lines = crate::ui::anim::spectrum_rows(&lv, &pk, rows_n, band_w, gap);
        for (i, cells) in lines.iter().enumerate() {
            let mut spans: Vec<Span> = Vec::with_capacity(cells.len());
            for c in cells {
                // 颜色按「高度」分档：低暗、中强调、顶亮
                // 按高度分档：顶行/峰值亮白，中段强调色，底部暗
                let style = if c.ch == '▔' || c.from_bottom + 1 >= rows_n {
                    theme::song_title()
                } else if c.from_bottom * 3 >= rows_n {
                    theme::primary()
                } else {
                    theme::dim()
                };
                spans.push(Span::styled(c.ch.to_string(), style));
            }
            let row = Rect::new(
                spec.x + margin,
                spec.y + i as u16,
                spec.width.saturating_sub(margin * 2),
                1,
            );
            f.render_widget(Paragraph::new(Line::from(spans)), row);
        }
    }
    // 进度：平滑进度条（主题色）+ 右侧时间
    if app.anim.smooth_progress_on() {
        let label_w = (UnicodeWidthStr::width(time_label.as_str()) as u16 + 2).min(prog.width);
        let [l, r] =
            Layout::horizontal([Constraint::Min(0), Constraint::Length(label_w)]).areas(prog);
        f.render_widget(
            Paragraph::new(smooth_progress_line(ratio, l.width as usize)),
            l,
        );
        f.render_widget(
            Paragraph::new(
                Line::from(Span::styled(time_label, theme::dim())).alignment(Alignment::Right),
            ),
            r,
        );
    } else {
        let gauge = Gauge::default()
            .gauge_style(theme::gauge_style())
            .ratio(ratio)
            .label(time_label);
        f.render_widget(gauge, prog);
    }
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
        render_center(
            f,
            area,
            "未检测到 USB 设备 (插入 U 盘后重新进入本界面, 格式化需要 sudo 密码)",
        );
        return;
    }
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(Span::styled(
            " 选择 USB 设备 (Enter = 格式化并下载热榜音乐, 数据将丢失!) ",
            theme::err(),
        ));
    let inner = block.inner(area);
    let (top, count) = window(
        app.super_sel,
        app.super_devices.len(),
        inner.height as usize,
    );
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
    let sel = if count > 0 {
        Some(app.super_sel - top)
    } else {
        None
    };
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
        render_center(
            f,
            area,
            "暂无下载任务 (在歌曲列表按 d 下载单曲, D 批量下载)",
        );
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

mod config;
mod downloader;
mod favorites;
mod models;
mod player;
mod proxy;
mod settings;
mod source;
mod superdl;
mod ui;
mod util;

use crossterm::cursor::Hide;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::stdout;

fn main() -> anyhow::Result<()> {
    let cfg = config::Config::load();
    let api = source::netease::NetEaseSource::new(cfg.api_base.clone());
    let registry = std::sync::Arc::new(source::SourceRegistry::with_defaults(
        cfg.api_base.clone(),
        cfg.proxy.clone(),
        cfg.meting_base.clone(),
    ));
    let quality = std::sync::Arc::new(std::sync::RwLock::new(cfg.quality.clone()));
    let downloader = downloader::Downloader::new(
        cfg.download_dir.clone(),
        quality.clone(),
        cfg.api_base.clone(),
    );
    let player = player::Player::start()?;

    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture, Hide)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = ui::App::new(cfg, api, player, downloader, quality, registry);
    let res = app.run(&mut terminal);

    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    disable_raw_mode()?;

    res
}

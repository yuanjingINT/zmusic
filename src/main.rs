mod config;
mod downloader;
mod favorites;
mod models;
mod player;
mod proxy;
mod settings;
mod i18n;
mod onboarding;
mod settings_ui;
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
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(|s| s.as_str()) {
        // 配置子命令：生成/打开 config.toml，不启动 TUI
        Some("config") => return config_cli(&args[1..]),
        Some("-h") | Some("--help") => {
            print_help();
            return Ok(());
        }
        Some("-V") | Some("--version") => {
            println!("zmusic {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some(other) => {
            eprintln!("未知参数: {other}");
            print_help();
            std::process::exit(2);
        }
        None => {}
    }

    // 首次运行：先走向导（语言/主题/波形/下载目录/音质），只在第一次出现。
    // 判据是配置里的 first_run_done，不是「配置文���存在与否」——升级不会再弹。
    {
        let probe = config::Config::load();
        if !probe.first_run_done {
            onboarding::run(&probe)?;
        }
    }
    // mpv 先起：socket 就绪要 ~120ms，这段和下面的配置/注册表初始化并行，
    // 首帧只等剩下的那点时间（旧实现先初始化、再同步等 mpv，白等一整个量化周期）。
    let starting = player::Player::spawn()?;
    let cfg = config::Config::load();
    let api = source::netease::NetEaseSource::new(cfg.api_base.clone());
    let registry = std::sync::Arc::new(source::SourceRegistry::with_defaults(
        cfg.api_base.clone(),
        cfg.proxy.clone(),
        cfg.meting_url.clone(),
        cfg.ytdlp_binary.clone(),
        cfg.meting_cookie.clone(),
        cfg.jamendo_client_id.clone(),
    ));
    let quality = std::sync::Arc::new(std::sync::RwLock::new(cfg.quality.clone()));
    let downloader = downloader::Downloader::new(
        cfg.download_dir.clone(),
        quality.clone(),
        cfg.api_base.clone(),
    );
    let player = starting.finish(std::time::Duration::from_secs(3))?;

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

fn print_help() {
    println!(
        "zmusic - 网易云/多源 终端音乐播放器\n\
         \n\
         用法:\n\
         \x20 zmusic            启动播放器\n\
         \x20 zmusic config     打开设置界面（外观/波形/下载目录）\n\
         \x20 zmusic config --edit   直接用 $EDITOR 编辑 config.toml\n\
         \x20 zmusic config --wizard 重新走一遍首次设置向导\n\
         \x20 zmusic config --print  只打印配置文件路径\n\
         \x20 zmusic --help / -V\n\
         \n\
         配置说明见 docs/CONFIG.md"
    );
}

/// `zmusic config`：默认打开交互设置界面；
/// `--print` 只打印路径；`--edit` 用 $EDITOR 直接编辑文件。
fn config_cli(rest: &[String]) -> anyhow::Result<()> {
    let cfg = config::Config::load();
    let path = cfg.ensure_config_file()?;
    if rest.iter().any(|a| a == "--print") {
        println!("{}", path.display());
        return Ok(());
    }
    if rest.iter().any(|a| a == "--wizard") {
        return onboarding::run(&cfg);
    }
    if !rest.iter().any(|a| a == "--edit") {
        return settings_ui::run(&cfg);
    }
    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| "vi".to_string());
    let status = std::process::Command::new(&editor)
        .arg(&path)
        .status()
        .map_err(|e| anyhow::anyhow!("启动编辑器 {} 失败: {e}", editor))?;
    if !status.success() {
        anyhow::bail!("{} 退出码 {:?}", editor, status.code());
    }
    Ok(())
}

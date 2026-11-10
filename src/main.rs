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
        // 源管理子命令：默认只有网易云，其它源用 add 打开
        Some("source") | Some("sources") => return source_cli(&args[1..]),
        Some("-h") | Some("--help") => {
            print_help();
            return Ok(());
        }
        Some("-V") | Some("--version") => {
            println!("zmuse {}", env!("CARGO_PKG_VERSION"));
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
        &cfg.enabled_sources,
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

/// `zmuse source …`：列出/启用/停用内置源（写回 config.toml 的 `[sources] enabled`）。
fn source_cli(rest: &[String]) -> anyhow::Result<()> {
    use source::{ALL_SOURCE_IDS, expand_source_ids, source_display};
    let cfg = config::Config::load();
    let mut current = cfg.enabled_sources.clone();

    let cmd = rest.first().map(|s| s.as_str()).unwrap_or("list");
    let all_flag = rest.iter().any(|a| a == "--all" || a == "-a");
    let ids: Vec<String> = rest
        .iter()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .cloned()
        .collect();

    let known = || -> String {
        let mut s = String::from("可用 id:\n");
        for id in ALL_SOURCE_IDS {
            s.push_str(&format!("  {:<16} {}\n", id, source_display(id)));
        }
        s.push_str("  也可用分组：meting（四个 meting 源）、all（全部）");
        s
    };

    match cmd {
        "list" | "ls" => {
            if all_flag {
                println!("{:<16} {:<14} enabled", "id", "name");
                for id in ALL_SOURCE_IDS {
                    let on = current.iter().any(|e| e == id);
                    println!(
                        "{:<16} {:<14} {}",
                        id,
                        source_display(id),
                        if on { "yes" } else { "-" }
                    );
                }
                println!();
                println!("启用: zmuse source add <id>    停用: zmuse source remove <id>");
            } else if current.is_empty() {
                println!("(没有任何启用的源) 用 zmuse source add <id> 添加；zmuse source list --all 看全部");
            } else {
                for id in &current {
                    println!("{:<16} {}", id, source_display(id));
                }
            }
            Ok(())
        }
        "add" | "enable" => {
            if ids.is_empty() {
                println!("用法: zmuse source add <id> [...]\n{}", known());
                return Ok(());
            }
            let (ok, bad) = expand_source_ids(&ids);
            if !bad.is_empty() {
                anyhow::bail!("没有这个源: {}\n{}", bad.join(", "), known());
            }
            let mut added = Vec::new();
            for id in ok {
                if !current.contains(&id) {
                    current.push(id.clone());
                    added.push(id);
                }
            }
            // 按注册顺序排列，S 循环与优先级才稳定
            current.sort_by_key(|id| {
                ALL_SOURCE_IDS
                    .iter()
                    .position(|x| x == id)
                    .unwrap_or(usize::MAX)
            });
            let path = cfg.save_enabled_sources(&current)?;
            if added.is_empty() {
                println!("这些源本来就是启用的：{}", ids.join(", "));
            } else {
                println!("已启用: {}", added.join(", "));
            }
            println!("写回 {}", path.display());
            println!("已启用的源：{}", current.join(", "));
            Ok(())
        }
        "remove" | "rm" | "disable" => {
            if ids.is_empty() {
                println!("用法: zmuse source remove <id> [...]\n{}", known());
                return Ok(());
            }
            let (ok, bad) = expand_source_ids(&ids);
            if !bad.is_empty() {
                anyhow::bail!("没有这个源: {}\n{}", bad.join(", "), known());
            }
            let before = current.len();
            current.retain(|id| !ok.contains(id));
            let path = cfg.save_enabled_sources(&current)?;
            if current.len() == before {
                println!("这些源本来就没启用：{}", ids.join(", "));
            } else {
                println!("已停用: {}", ok.join(", "));
            }
            if !current.iter().any(|id| id == "netease") {
                println!("注意：网易云已停用，你现在没有默认源了。加回来：zmuse source add netease");
            }
            println!("写回 {}", path.display());
            if current.is_empty() {
                println!("已启用的源：（空）");
            } else {
                println!("已启用的源：{}", current.join(", "));
            }
            Ok(())
        }
        other => {
            anyhow::bail!(
                "未知子命令: {}\n用法:\n  zmuse source list [--all]\n  zmuse source add <id> [...]\n  zmuse source remove <id> [...]\n{}",
                other,
                known()
            )
        }
    }
}

fn print_help() {
    println!(
        "zmuse - 网易云/多源 终端音乐播放器\n\
         \n\
         用法:\n\
         \x20 zmuse            启动播放器\n\
         \x20 zmuse config     打开设置界面（外观/波形/下载目录）\n\
         \x20 zmuse config --edit   直接用 $EDITOR 编辑 config.toml\n\
         \x20 zmuse config --wizard 重新走一遍首次设置向导\n\
         \x20 zmuse config --print  只打印配置文件路径\n\
         \x20 zmuse source        列出已启用的源（--all 看全部）\n\
         \x20 zmuse source add <id>     启用一个源\n\
         \x20 zmuse source remove <id>  停用一个源\n\
         \x20 zmuse --help / -V\n\
         \n\
         配置说明见 docs/CONFIG.md"
    );
}

/// `zmuse config`：默认打开交互设置界面；
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

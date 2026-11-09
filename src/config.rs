//! 统一配置：`~/.config/zmusic/config.toml`，环境变量仍可覆盖（向后兼容）。
//!
//! 优先级：环境变量 > config.toml > 内置默认值。
//! 只有「服务/账号类」配置放这里；UI 偏好（播放顺序、上次选的源、下载位置）留在
//! `settings.json`，别让同一个值有两个可改的地方。

use serde::Deserialize;
use std::path::{Path, PathBuf};

/// config.toml 的模板，`zmusic config` 在文件不存在时写出来（带注释）
pub const TEMPLATE: &str = r#"# zmusic 配置文件
# 位置：~/.config/zmusic/config.toml
# 环境变量仍可覆盖这里的值（NCM_API / NCM_DIR / NCM_QUALITY / ZMUSIC_PROXY ...）

[general]
# 音质：standard / higher / exhigh / lossless / hires
quality = "exhigh"
# 下载目录（支持 ~）
download_dir = "~/Music/ncm"

[netease]
# ncm-api 服务地址（Node >= 22，仓库里 ./setup.sh 会装成 systemd --user 服务）
api_base = "http://127.0.0.1:3000"

[meting]
# Meting 侧车地址（腾讯/酷狗/百度/酷我）。装包后先：
#   systemctl --user enable --now meting-sidecar
url = "http://127.0.0.1:8123"
# 可选：平台 cookie，用于会员/受限曲目换流。留空则这些源只能搜不能播。
# 只从本地文件/环境变量读，别提交进仓库。取法见 docs/CONFIG.md
cookie = ""

[ytdlp]
# yt-dlp 可执行文件名或绝对路径（YouTube / SoundCloud 源用）
binary = "yt-dlp"
# YouTube/SoundCloud 需要代理；zmusic 与 mpv 会走同一个出口。
# 例：proxy = "socks5h://127.0.0.1:1080"
proxy = ""

[ui]
# 动效总开关；关掉后界面完全静止（与没有动画的版本行为一致）
animations = true
# 下面四个可以单独关
spinner = true
equalizer = true
smooth_progress = true
fade = true
# 均衡器帧率（5-30，默认 15，和播放时的事件循环节奏对齐）
equalizer_fps = 30
# 均衡器数据来源：pcm = 真解码取频谱（多一份带宽）；synth = 正弦波形（不额外拉流）
equalizer_source = "pcm"
# 自动增益：用最近 10 秒的能量分位数动态标定 dB 范围（安静段自动压到 1-2 档）
eq_agc = true
# 波形满高对应的 dBFS（越小越容易顶满；默认 -6）
eq_reference_dbfs = -6.0
# 波形增益（1.0 = 不放大；默认 1.6。调大波形更高，但副歌会平顶）
eq_gain = 1.6
# 软限幅（tanh）：响的地方挤向顶部但不硬切，平顶更少；默认关
eq_soft_clip = false

[jamendo]
# Jamendo 免费申请的 client_id（https://devportal.jamendo.com/）
client_id = ""
"#;

#[derive(Debug, Default, Deserialize)]
struct General {
    #[serde(default)]
    quality: Option<String>,
    #[serde(default)]
    download_dir: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Ui {
    #[serde(default)]
    animations: Option<bool>,
    #[serde(default)]
    spinner: Option<bool>,
    #[serde(default)]
    equalizer: Option<bool>,
    #[serde(default)]
    smooth_progress: Option<bool>,
    #[serde(default)]
    fade: Option<bool>,
    #[serde(default)]
    equalizer_fps: Option<u8>,
    #[serde(default)]
    equalizer_source: Option<String>,
    #[serde(default)]
    eq_agc: Option<bool>,
    #[serde(default)]
    eq_reference_dbfs: Option<f32>,
    #[serde(default)]
    eq_gain: Option<f32>,
    #[serde(default)]
    eq_soft_clip: Option<bool>,
}

/// 均衡器数据来源
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EqSource {
    /// 真解码取 PCM 算频谱（默认）
    Pcm,
    /// 正弦波形兜底（不额外拉流）
    Synth,
}

/// 动效开关：总开关 + 每个动效单独开关，全部默认开。
/// 总开关关掉时行为与「没有动画的版本」完全一致（渲染函数走静态分支）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Animations {
    pub animations: bool,
    pub spinner: bool,
    pub equalizer: bool,
    pub smooth_progress: bool,
    pub fade: bool,
    /// 均衡器帧率，5–60，默认 30
    pub equalizer_fps: u8,
    /// 频谱来源：pcm / synth
    pub equalizer_source: EqSource,
    /// 自动增益标定（默认开）
    pub eq_agc: bool,
    /// 波形满高对应的 dBFS（默认 -6）
    pub eq_reference_dbfs: f32,
    /// 波形增益（默认 1.6）
    pub eq_gain: f32,
    /// 软限幅（tanh），默认关：开了平顶少、但峰值不会真的贴边
    pub eq_soft_clip: bool,
}

impl Default for Animations {
    fn default() -> Self {
        Animations {
            animations: true,
            spinner: true,
            equalizer: true,
            smooth_progress: true,
            fade: true,
            equalizer_fps: 30,
            equalizer_source: EqSource::Pcm,
            eq_agc: true,
            eq_reference_dbfs: -6.0,
            eq_gain: 1.6,
            eq_soft_clip: false,
        }
    }
}

impl Animations {
    pub fn spinner_on(&self) -> bool {
        self.animations && self.spinner
    }
    pub fn equalizer_on(&self) -> bool {
        self.animations && self.equalizer
    }
    pub fn smooth_progress_on(&self) -> bool {
        self.animations && self.smooth_progress
    }
    pub fn fade_on(&self) -> bool {
        self.animations && self.fade
    }
}

#[derive(Debug, Default, Deserialize)]
struct Netease {
    #[serde(default)]
    api_base: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Meting {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    cookie: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct YtDlp {
    #[serde(default)]
    binary: Option<String>,
    #[serde(default)]
    proxy: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Jamendo {
    #[serde(default)]
    client_id: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    #[serde(default)]
    general: General,
    #[serde(default)]
    netease: Netease,
    #[serde(default)]
    meting: Meting,
    #[serde(default)]
    ytdlp: YtDlp,
    #[serde(default)]
    jamendo: Jamendo,
    #[serde(default)]
    ui: Ui,
}

pub struct Config {
    pub download_dir: PathBuf,
    pub quality: String,
    pub api_base: String,
    pub meting_url: String,
    pub meting_cookie: Option<String>,
    pub proxy: Option<String>,
    pub ytdlp_binary: String,
    pub jamendo_client_id: Option<String>,
    pub config_path: PathBuf,
    pub favorites_path: PathBuf,
    pub settings_path: PathBuf,
    /// 动效开关（`[ui]`）
    pub animations: Animations,
}

fn non_empty(v: String) -> Option<String> {
    let t = v.trim().to_string();
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

fn expand_tilde(s: &str, home: &Path) -> PathBuf {
    if let Some(rest) = s.strip_prefix("~/") {
        home.join(rest)
    } else if s == "~" {
        home.to_path_buf()
    } else {
        PathBuf::from(s)
    }
}

impl Config {
    /// 读取配置；`config.toml` 不存在时用内置默认值（不自动创建，创建是 `zmusic config` 的事）
    pub fn load() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| home.join(".config"))
            .join("zmusic");
        let config_path = config_dir.join("config.toml");
        let text = std::fs::read_to_string(&config_path).ok();
        let cfg = Self::resolve(text.as_deref(), &|k| std::env::var(k).ok(), &home);
        let _ = std::fs::create_dir_all(&cfg.download_dir);
        cfg
    }

    /// 纯逻辑：文件文本 + 环境变量查询闭包 → 最终配置（方便单测）
    fn resolve(file: Option<&str>, env: &dyn Fn(&str) -> Option<String>, home: &Path) -> Self {
        let parsed: FileConfig = file
            .and_then(|t| toml::from_str(t).ok())
            .unwrap_or_default();
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| home.join(".config"))
            .join("zmusic");

        let pick = |env_key: &str, file_val: Option<String>, default: &str| -> String {
            env(env_key)
                .and_then(non_empty)
                .or_else(|| file_val.and_then(non_empty))
                .unwrap_or_else(|| default.to_string())
        };
        let pick_bool = |env: &dyn Fn(&str) -> Option<String>,
                         env_key: &str,
                         file_val: Option<bool>,
                         default: bool|
         -> bool {
            match env(env_key).as_deref().map(str::trim) {
                Some("0") | Some("false") | Some("off") | Some("no") => false,
                Some("1") | Some("true") | Some("on") | Some("yes") => true,
                _ => file_val.unwrap_or(default),
            }
        };
        let pick_u8 = |env: &dyn Fn(&str) -> Option<String>,
                       env_key: &str,
                       file_val: Option<u8>,
                       default: u8|
         -> u8 {
            env(env_key)
                .and_then(|v| v.trim().parse::<u8>().ok())
                .or(file_val)
                .unwrap_or(default)
        };
        let pick_f32 = |env: &dyn Fn(&str) -> Option<String>,
                        env_key: &str,
                        file_val: Option<f32>,
                        default: f32|
         -> f32 {
            env(env_key)
                .and_then(|v| v.trim().parse::<f32>().ok())
                .or(file_val)
                .unwrap_or(default)
        };
        let pick_opt = |env_key: &str, file_val: Option<String>| -> Option<String> {
            env(env_key)
                .and_then(non_empty)
                .or_else(|| file_val.and_then(non_empty))
        };

        let download_dir = match pick_opt("NCM_DIR", parsed.general.download_dir) {
            Some(v) => expand_tilde(&v, home),
            None => home.join("Music").join("ncm"),
        };
        Config {
            download_dir,
            quality: pick("NCM_QUALITY", parsed.general.quality, "exhigh"),
            api_base: pick("NCM_API", parsed.netease.api_base, "http://127.0.0.1:3000"),
            meting_url: pick(
                "ZMUSIC_METING_URL",
                parsed.meting.url,
                "http://127.0.0.1:8123",
            ),
            meting_cookie: pick_opt("ZMUSIC_METING_COOKIE", parsed.meting.cookie),
            proxy: pick_opt("ZMUSIC_PROXY", parsed.ytdlp.proxy),
            ytdlp_binary: pick("ZMUSIC_YTDLP_BIN", parsed.ytdlp.binary, "yt-dlp"),
            jamendo_client_id: pick_opt("ZMUSIC_JAMENDO_CLIENT_ID", parsed.jamendo.client_id),
            config_path: config_dir.join("config.toml"),
            favorites_path: config_dir.join("favorites.json"),
            settings_path: config_dir.join("settings.json"),
            animations: Animations {
                animations: pick_bool(env, "ZMUSIC_ANIMATIONS", parsed.ui.animations, true),
                spinner: parsed.ui.spinner.unwrap_or(true),
                equalizer: parsed.ui.equalizer.unwrap_or(true),
                smooth_progress: parsed.ui.smooth_progress.unwrap_or(true),
                fade: parsed.ui.fade.unwrap_or(true),
                equalizer_fps: pick_u8(env, "ZMUSIC_EQ_FPS", parsed.ui.equalizer_fps, 30),
                eq_agc: pick_bool(env, "ZMUSIC_EQ_AGC", parsed.ui.eq_agc, true),
                eq_reference_dbfs: pick_f32(
                    env,
                    "ZMUSIC_EQ_REF_DBFS",
                    parsed.ui.eq_reference_dbfs,
                    -6.0,
                ),
                eq_gain: pick_f32(env, "ZMUSIC_EQ_GAIN", parsed.ui.eq_gain, 1.6),
                eq_soft_clip: pick_bool(env, "ZMUSIC_EQ_SOFT_CLIP", parsed.ui.eq_soft_clip, false),
                equalizer_source: match pick("ZMUSIC_EQ_SOURCE", parsed.ui.equalizer_source, "pcm")
                    .as_str()
                {
                    "synth" => EqSource::Synth,
                    _ => EqSource::Pcm,
                },
            },
        }
    }

    /// 确保配置文件存在（不存在就写模板），返回路径
    pub fn ensure_config_file(&self) -> std::io::Result<PathBuf> {
        if !self.config_path.exists() {
            if let Some(dir) = self.config_path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&self.config_path, TEMPLATE)?;
        }
        Ok(self.config_path.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn home() -> PathBuf {
        PathBuf::from("/home/tester")
    }

    #[test]
    fn missing_file_uses_defaults() {
        let c = Config::resolve(None, &no_env, &home());
        assert_eq!(c.quality, "exhigh");
        assert_eq!(c.api_base, "http://127.0.0.1:3000");
        assert_eq!(c.meting_url, "http://127.0.0.1:8123");
        assert_eq!(c.ytdlp_binary, "yt-dlp");
        assert_eq!(c.proxy, None);
        assert_eq!(c.meting_cookie, None);
        assert_eq!(c.download_dir, PathBuf::from("/home/tester/Music/ncm"));
    }

    #[test]
    fn partial_file_fills_missing_keys_with_defaults() {
        let text = r#"
[meting]
url = "http://127.0.0.1:9999"
"#;
        let c = Config::resolve(Some(text), &no_env, &home());
        assert_eq!(c.meting_url, "http://127.0.0.1:9999");
        // 没写的字段仍然是默认值
        assert_eq!(c.quality, "exhigh");
        assert_eq!(c.api_base, "http://127.0.0.1:3000");
        assert_eq!(c.ytdlp_binary, "yt-dlp");
    }

    #[test]
    fn file_values_are_used_and_tilde_expanded() {
        let text = r#"
[general]
quality = "lossless"
download_dir = "~/Music/zm"

[netease]
api_base = "http://10.0.0.2:3000"

[meting]
cookie = "uin=o1; qm_keyst=x"

[ytdlp]
proxy = "socks5h://127.0.0.1:1080"
binary = "/usr/local/bin/yt-dlp"

[jamendo]
client_id = "abc123"
"#;
        let c = Config::resolve(Some(text), &no_env, &home());
        assert_eq!(c.quality, "lossless");
        assert_eq!(c.download_dir, PathBuf::from("/home/tester/Music/zm"));
        assert_eq!(c.api_base, "http://10.0.0.2:3000");
        assert_eq!(c.meting_cookie.as_deref(), Some("uin=o1; qm_keyst=x"));
        assert_eq!(c.proxy.as_deref(), Some("socks5h://127.0.0.1:1080"));
        assert_eq!(c.ytdlp_binary, "/usr/local/bin/yt-dlp");
        assert_eq!(c.jamendo_client_id.as_deref(), Some("abc123"));
    }

    #[test]
    fn env_overrides_file_value() {
        let text = r#"
[general]
quality = "standard"
[netease]
api_base = "http://file:3000"
[meting]
url = "http://file:8123"
"#;
        let env = |k: &str| -> Option<String> {
            match k {
                "NCM_QUALITY" => Some("hires".to_string()),
                "ZMUSIC_METING_URL" => Some("http://env:8123".to_string()),
                _ => None,
            }
        };
        let c = Config::resolve(Some(text), &env, &home());
        assert_eq!(c.quality, "hires", "env 覆盖文件");
        assert_eq!(c.meting_url, "http://env:8123", "env 覆盖文件");
        assert_eq!(c.api_base, "http://file:3000", "没设 env 的仍用文件值");
    }

    #[test]
    fn empty_env_var_does_not_override() {
        let env = |k: &str| -> Option<String> {
            if k == "ZMUSIC_PROXY" {
                Some("   ".to_string())
            } else {
                None
            }
        };
        let c = Config::resolve(None, &env, &home());
        assert_eq!(c.proxy, None, "空字符串等于没配");
    }

    #[test]
    fn template_is_valid_toml_and_parses_cleanly() {
        let c = Config::resolve(Some(TEMPLATE), &no_env, &home());
        assert_eq!(c.quality, "exhigh");
        assert_eq!(c.download_dir, PathBuf::from("/home/tester/Music/ncm"));
        assert_eq!(c.proxy, None);
    }
}

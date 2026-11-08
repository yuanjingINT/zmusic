use std::path::PathBuf;

pub struct Config {
    pub api_base: String,
    pub download_dir: PathBuf,
    pub quality: String,
    /// 可选代理（如 socks5h://127.0.0.1:11080）；YouTube 源与 mpv 必须同一出口
    pub proxy: Option<String>,
    /// Meting 侧车地址
    pub meting_base: String,
    pub favorites_path: PathBuf,
    pub settings_path: PathBuf,
}

impl Config {
    pub fn load() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let api_base =
            std::env::var("NCM_API").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());
        let download_dir = std::env::var("NCM_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home.join("Music").join("ncm"));
        let quality = std::env::var("NCM_QUALITY").unwrap_or_else(|_| "exhigh".to_string());
        let meting_base = std::env::var("ZMUSIC_METING_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8123".to_string());
        let proxy = std::env::var("ZMUSIC_PROXY")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| home.join(".config"))
            .join("zmusic");
        let favorites_path = config_dir.join("favorites.json");
        let settings_path = config_dir.join("settings.json");
        let _ = std::fs::create_dir_all(&download_dir);
        Config {
            api_base,
            download_dir,
            quality,
            proxy,
            meting_base,
            favorites_path,
            settings_path,
        }
    }
}

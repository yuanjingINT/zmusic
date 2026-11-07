use std::path::PathBuf;

pub struct Config {
    pub api_base: String,
    pub download_dir: PathBuf,
    pub quality: String,
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
        let config_dir = dirs::config_dir().unwrap_or_else(|| home.join(".config")).join("zmusic");
        let favorites_path = config_dir.join("favorites.json");
        let settings_path = config_dir.join("settings.json");
        let _ = std::fs::create_dir_all(&download_dir);
        Config {
            api_base,
            download_dir,
            quality,
            favorites_path,
            settings_path,
        }
    }
}

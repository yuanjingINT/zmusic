use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub download_location: String,
    #[serde(default)]
    pub usb_mount: String,
    #[serde(default)]
    pub usb_device: String,
    /// 音质（standard/higher/exhigh/lossless/hires），空表示没设过、沿用环境变量
    #[serde(default)]
    pub quality: String,
    /// 播放顺序（sequential/shuffle/repeat_one）
    #[serde(default)]
    pub play_mode: String,
}

impl Settings {
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Settings::default(),
        }
    }

    pub fn save(&self, path: &Path) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, s);
        }
    }

    pub fn is_usb(&self) -> bool {
        self.download_location == "usb"
    }
}

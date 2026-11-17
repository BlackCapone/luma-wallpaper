use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::PathBuf};

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Settings {
    pub folder: String,
    pub all: Option<String>,
    pub per_monitor: BTreeMap<String, String>,
    pub autostart: bool,
    pub pause_fullscreen: bool,
    pub pause_battery: bool,
    pub audio: bool,
    pub steamcmd: String,
    pub steam_user: String,
}

fn base() -> PathBuf {
    PathBuf::from(std::env::var("APPDATA").unwrap_or_else(|_| ".".into())).join("Luma")
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            folder: base().join("Wallpapers").to_string_lossy().into(),
            all: None,
            per_monitor: BTreeMap::new(),
            autostart: false,
            pause_fullscreen: true,
            pause_battery: true,
            audio: true,
            steamcmd: String::new(),
            steam_user: String::new(),
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        let s: Self = fs::read_to_string(base().join("settings.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        let _ = fs::create_dir_all(&s.folder);
        s
    }

    pub fn save(&self) {
        let _ = fs::create_dir_all(base());
        if let Ok(t) = serde_json::to_string_pretty(self) {
            let _ = fs::write(base().join("settings.json"), t);
        }
    }

    pub fn for_monitor(&self, i: usize) -> Option<String> {
        self.per_monitor.get(&i.to_string()).or(self.all.as_ref()).cloned()
    }
}

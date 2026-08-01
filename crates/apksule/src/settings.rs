//! Persistent host settings under `%APPDATA%\Apksule\settings.json`.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostSettings {
    #[serde(default = "default_true")]
    pub auto_update: bool,
}

impl Default for HostSettings {
    fn default() -> Self {
        Self { auto_update: true }
    }
}

const fn default_true() -> bool {
    true
}

impl HostSettings {
    pub fn load() -> Self {
        let path = settings_path();
        match fs::read_to_string(&path) {
            Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = settings_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let raw = serde_json::to_string_pretty(self).map_err(|error| error.to_string())?;
        fs::write(&path, raw).map_err(|error| error.to_string())
    }
}

#[must_use]
pub fn apksule_data_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .or_else(|| std::env::var_os("LOCALAPPDATA"))
        .map_or_else(std::env::temp_dir, PathBuf::from)
        .join("Apksule")
}

#[must_use]
pub fn logs_dir() -> PathBuf {
    apksule_data_dir().join("apps")
}

fn settings_path() -> PathBuf {
    apksule_data_dir().join("settings.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_settings_file() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("apksule-settings-{unique}"));
        fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("settings.json");
        let settings = HostSettings { auto_update: false };
        let raw = serde_json::to_string_pretty(&settings).expect("json");
        fs::write(&path, raw).expect("write");
        let loaded: HostSettings =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert!(!loaded.auto_update);
        let _ = fs::remove_dir_all(dir);
    }
}

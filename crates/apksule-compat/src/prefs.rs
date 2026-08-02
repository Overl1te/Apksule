//! File-backed `SharedPreferences` for M4.

#![allow(clippy::must_use_candidate, clippy::doc_markdown)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::error::{CompatError, Result};
use crate::storage::AppStorage;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum PrefValue {
    String(String),
    Int(i32),
    Long(i64),
    Bool(bool),
    Float(f32),
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct PrefFile {
    values: HashMap<String, PrefValue>,
}

/// In-memory + JSON SharedPreferences store for one prefs name.
#[derive(Debug, Clone)]
pub struct SharedPreferencesStore {
    path: PathBuf,
    inner: Arc<Mutex<PrefFile>>,
}

impl SharedPreferencesStore {
    pub fn open(storage: &AppStorage, name: &str) -> Result<Self> {
        let safe = sanitize_prefs_name(name)?;
        let path = storage.resolve_shared_prefs(format!("{safe}.json"))?;
        let values = if path.exists() {
            let bytes = std::fs::read(&path)
                .map_err(|source| CompatError::Io { path: path.clone(), source })?;
            serde_json::from_slice(&bytes).unwrap_or_default()
        } else {
            PrefFile::default()
        };
        Ok(Self { path, inner: Arc::new(Mutex::new(values)) })
    }

    pub fn get(&self, key: &str) -> Option<PrefValue> {
        self.inner.lock().ok()?.values.get(key).cloned()
    }

    pub fn contains(&self, key: &str) -> bool {
        self.inner.lock().ok().is_some_and(|file| file.values.contains_key(key))
    }

    pub fn put(&self, key: impl Into<String>, value: PrefValue) -> Result<()> {
        let mut file = self.inner.lock().map_err(|_| CompatError::Prefs("lock poisoned".into()))?;
        file.values.insert(key.into(), value);
        Ok(())
    }

    pub fn remove(&self, key: &str) -> Result<()> {
        let mut file = self.inner.lock().map_err(|_| CompatError::Prefs("lock poisoned".into()))?;
        file.values.remove(key);
        Ok(())
    }

    pub fn clear(&self) -> Result<()> {
        let mut file = self.inner.lock().map_err(|_| CompatError::Prefs("lock poisoned".into()))?;
        file.values.clear();
        Ok(())
    }

    pub fn commit(&self) -> Result<()> {
        let file = self.inner.lock().map_err(|_| CompatError::Prefs("lock poisoned".into()))?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|source| CompatError::Io { path: parent.to_path_buf(), source })?;
        }
        let bytes = serde_json::to_vec_pretty(&*file)
            .map_err(|error| CompatError::Prefs(error.to_string()))?;
        std::fs::write(&self.path, bytes)
            .map_err(|source| CompatError::Io { path: self.path.clone(), source })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn sanitize_prefs_name(name: &str) -> Result<String> {
    let safe: String = name
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.' { ch } else { '_' })
        .collect();
    if safe.is_empty() {
        Err(CompatError::Prefs("empty SharedPreferences name".into()))
    } else {
        Ok(safe)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::AppStorage;

    #[test]
    fn prefs_persist_across_open() {
        let unique = format!(
            "apksule-prefs-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        let base = std::env::temp_dir().join(unique);
        let storage = AppStorage::for_package_at(&base, "org.example.notes").expect("storage");
        let prefs = SharedPreferencesStore::open(&storage, "settings").expect("open");
        prefs.put("theme", PrefValue::String("dark".into())).expect("put");
        prefs.put("count", PrefValue::Int(3)).expect("put");
        prefs.commit().expect("commit");

        let again = SharedPreferencesStore::open(&storage, "settings").expect("reopen");
        assert_eq!(again.get("theme"), Some(PrefValue::String("dark".into())));
        assert_eq!(again.get("count"), Some(PrefValue::Int(3)));
        let _ = std::fs::remove_dir_all(base);
    }
}

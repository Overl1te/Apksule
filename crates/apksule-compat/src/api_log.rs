use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::{CompatError, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsupportedApiCall {
    pub timestamp_ms: u128,
    pub class_name: String,
    pub method_name: String,
    pub detail: String,
}

/// Append-only sink for compatibility misses. Clones share one synchronized file.
#[derive(Debug, Clone)]
pub struct ApiLogger {
    path: PathBuf,
    file: Arc<Mutex<File>>,
}

impl ApiLogger {
    pub fn new(log_directory: impl AsRef<Path>) -> Result<Self> {
        let directory = log_directory.as_ref();
        std::fs::create_dir_all(directory)
            .map_err(|source| CompatError::Io { path: directory.to_path_buf(), source })?;
        let path = directory.join("unsupported-api.log");
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|source| CompatError::Io { path: path.clone(), source })?;
        Ok(Self { path, file: Arc::new(Mutex::new(file)) })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn unsupported(
        &self,
        class_name: impl Into<String>,
        method_name: impl Into<String>,
        detail: impl Into<String>,
    ) -> Result<UnsupportedApiCall> {
        let call = UnsupportedApiCall {
            timestamp_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
            class_name: class_name.into(),
            method_name: method_name.into(),
            detail: detail.into(),
        };

        tracing::warn!(
            android_class = %call.class_name,
            android_method = %call.method_name,
            detail = %call.detail,
            "unsupported Android API"
        );

        let clean_detail = call.detail.replace(['\r', '\n', '\t'], " ");
        let mut file = self.file.lock().map_err(|_| CompatError::ApiLogPoisoned)?;
        writeln!(
            file,
            "{}\t{}\t{}\t{}",
            call.timestamp_ms, call.class_name, call.method_name, clean_detail
        )
        .map_err(|source| CompatError::Io { path: self.path.clone(), source })?;
        file.flush().map_err(|source| CompatError::Io { path: self.path.clone(), source })?;
        Ok(call)
    }
}

use std::fmt;
use std::sync::Arc;

use crate::error::{CompatError, Result};

/// Runtime-neutral bridge to bytes stored in an APK.
pub trait ResourceSource: Send + Sync {
    fn contains(&self, path: &str) -> bool;
    fn load(&self, path: &str) -> std::result::Result<Vec<u8>, String>;
}

/// Android-like resource facade. resources.arsc value decoding is intentionally
/// deferred, while raw resources and assets are available now.
#[derive(Clone)]
pub struct Resources {
    package_name: String,
    source: Arc<dyn ResourceSource>,
    density_dpi: u32,
    locale: String,
}

impl Resources {
    #[must_use]
    pub fn new(package_name: impl Into<String>, source: Arc<dyn ResourceSource>) -> Self {
        Self {
            package_name: package_name.into(),
            source,
            density_dpi: 160,
            locale: "en-US".to_owned(),
        }
    }

    #[must_use]
    pub fn package_name(&self) -> &str {
        &self.package_name
    }

    #[must_use]
    pub fn density_dpi(&self) -> u32 {
        self.density_dpi
    }

    #[must_use]
    pub fn locale(&self) -> &str {
        &self.locale
    }

    #[must_use]
    pub fn has_compiled_table(&self) -> bool {
        self.source.contains("resources.arsc")
    }

    pub fn load_compiled_table(&self) -> Result<Vec<u8>> {
        self.load_path("resources.arsc")
    }

    pub fn load_asset(&self, relative_path: &str) -> Result<Vec<u8>> {
        self.load_path(&prefixed_path("assets", relative_path)?)
    }

    pub fn load_raw_resource(&self, relative_path: &str) -> Result<Vec<u8>> {
        self.load_path(&prefixed_path("res", relative_path)?)
    }

    #[must_use]
    pub fn contains(&self, apk_path: &str) -> bool {
        self.source.contains(apk_path)
    }

    /// Load an arbitrary APK entry path (e.g. `res/layout/main.xml`).
    pub fn load_entry(&self, apk_path: &str) -> Result<Vec<u8>> {
        self.load_path(apk_path)
    }

    fn load_path(&self, apk_path: &str) -> Result<Vec<u8>> {
        self.source
            .load(apk_path)
            .map_err(|message| CompatError::Resource { path: apk_path.to_owned(), message })
    }
}

impl fmt::Debug for Resources {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Resources")
            .field("package_name", &self.package_name)
            .field("density_dpi", &self.density_dpi)
            .field("locale", &self.locale)
            .field("has_compiled_table", &self.has_compiled_table())
            .finish_non_exhaustive()
    }
}

fn prefixed_path(prefix: &str, relative_path: &str) -> Result<String> {
    let normalized = relative_path.replace('\\', "/");
    let valid = !normalized.is_empty()
        && !normalized.starts_with('/')
        && normalized.split('/').all(|part| !part.is_empty() && part != "." && part != "..");
    if valid {
        Ok(format!("{prefix}/{normalized}"))
    } else {
        Err(CompatError::InvalidRelativePath(relative_path.into()))
    }
}

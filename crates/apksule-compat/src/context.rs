use std::path::Path;
use std::sync::Arc;

use crate::api_log::{ApiLogger, UnsupportedApiCall};
use crate::error::Result;
use crate::gms::{GmsDetection, GmsShim};
use crate::resources::{ResourceSource, Resources};
use crate::storage::AppStorage;

/// Android-like process context owned by one launched package.
#[derive(Debug, Clone)]
pub struct Context {
    package_name: String,
    storage: AppStorage,
    resources: Resources,
    api_log: ApiLogger,
    gms: GmsShim,
}

impl Context {
    pub fn new(
        package_name: impl Into<String>,
        resource_source: Arc<dyn ResourceSource>,
        gms_signals: &[String],
    ) -> Result<Self> {
        let package_name = package_name.into();
        let storage = AppStorage::for_package(&package_name)?;
        Self::from_parts(package_name, resource_source, gms_signals, storage)
    }

    pub fn with_storage_base(
        package_name: impl Into<String>,
        resource_source: Arc<dyn ResourceSource>,
        gms_signals: &[String],
        storage_base: impl AsRef<Path>,
    ) -> Result<Self> {
        let package_name = package_name.into();
        let storage = AppStorage::for_package_at(storage_base, &package_name)?;
        Self::from_parts(package_name, resource_source, gms_signals, storage)
    }

    fn from_parts(
        package_name: String,
        resource_source: Arc<dyn ResourceSource>,
        gms_signals: &[String],
        storage: AppStorage,
    ) -> Result<Self> {
        let api_log = ApiLogger::new(storage.logs_dir())?;
        let resources = Resources::new(package_name.clone(), resource_source);
        let detection = GmsDetection::from_signals(gms_signals.iter().map(String::as_str));
        let gms = GmsShim::new(detection, api_log.clone());
        Ok(Self { package_name, storage, resources, api_log, gms })
    }

    #[must_use]
    pub fn package_name(&self) -> &str {
        &self.package_name
    }

    #[must_use]
    pub fn storage(&self) -> &AppStorage {
        &self.storage
    }

    #[must_use]
    pub fn resources(&self) -> &Resources {
        &self.resources
    }

    #[must_use]
    pub fn api_log(&self) -> &ApiLogger {
        &self.api_log
    }

    #[must_use]
    pub fn gms(&self) -> &GmsShim {
        &self.gms
    }

    pub fn unsupported_api(
        &self,
        class_name: impl Into<String>,
        method_name: impl Into<String>,
        detail: impl Into<String>,
    ) -> Result<UnsupportedApiCall> {
        self.api_log.unsupported(class_name, method_name, detail)
    }
}

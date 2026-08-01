use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

use crate::error::{CompatError, Result};

/// Per-package storage sandbox rooted below `%APPDATA%\Apksule\apps`.
#[derive(Debug, Clone)]
pub struct AppStorage {
    root: PathBuf,
    files: PathBuf,
    cache: PathBuf,
    databases: PathBuf,
    logs: PathBuf,
}

impl AppStorage {
    pub fn for_package(package_name: &str) -> Result<Self> {
        let app_data = std::env::var_os("APPDATA")
            .or_else(|| std::env::var_os("LOCALAPPDATA"))
            .ok_or(CompatError::MissingAppData)?;
        Self::for_package_at(Path::new(&app_data).join("Apksule").join("apps"), package_name)
    }

    pub fn for_package_at(base: impl AsRef<Path>, package_name: &str) -> Result<Self> {
        validate_package_name(package_name)?;
        let root = base.as_ref().join(package_name);
        let storage = Self {
            files: root.join("files"),
            cache: root.join("cache"),
            databases: root.join("databases"),
            logs: root.join("logs"),
            root,
        };
        storage.ensure_layout()?;
        Ok(storage)
    }

    fn ensure_layout(&self) -> Result<()> {
        for path in [&self.files, &self.cache, &self.databases, &self.logs] {
            std::fs::create_dir_all(path)
                .map_err(|source| CompatError::Io { path: path.clone(), source })?;
        }
        Ok(())
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn files_dir(&self) -> &Path {
        &self.files
    }

    #[must_use]
    pub fn cache_dir(&self) -> &Path {
        &self.cache
    }

    #[must_use]
    pub fn databases_dir(&self) -> &Path {
        &self.databases
    }

    #[must_use]
    pub fn logs_dir(&self) -> &Path {
        &self.logs
    }

    pub fn resolve_file(&self, relative_path: impl AsRef<Path>) -> Result<PathBuf> {
        secure_join(&self.files, relative_path.as_ref())
    }

    pub fn resolve_cache(&self, relative_path: impl AsRef<Path>) -> Result<PathBuf> {
        secure_join(&self.cache, relative_path.as_ref())
    }

    pub fn resolve_database(&self, relative_path: impl AsRef<Path>) -> Result<PathBuf> {
        secure_join(&self.databases, relative_path.as_ref())
    }

    pub fn write_file(&self, relative_path: impl AsRef<Path>, bytes: &[u8]) -> Result<()> {
        let path = self.resolve_file(relative_path)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|source| CompatError::Io { path: parent.to_path_buf(), source })?;
        }
        std::fs::write(&path, bytes).map_err(|source| CompatError::Io { path, source })
    }

    pub fn read_file(&self, relative_path: impl AsRef<Path>) -> Result<Vec<u8>> {
        let path = self.resolve_file(relative_path)?;
        std::fs::read(&path).map_err(|source| CompatError::Io { path, source })
    }
}

fn validate_package_name(package_name: &str) -> Result<()> {
    let valid = !package_name.is_empty()
        && package_name.split('.').all(|segment| {
            !segment.is_empty()
                && segment
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '_')
        });
    if valid { Ok(()) } else { Err(CompatError::InvalidPackageName(package_name.to_owned())) }
}

fn secure_join(root: &Path, relative: &Path) -> Result<PathBuf> {
    let valid = !relative.as_os_str().is_empty()
        && relative.components().all(|component| match component {
            Component::Normal(name) => name != OsStr::new(""),
            Component::CurDir => true,
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => false,
        });
    if valid {
        Ok(root.join(relative))
    } else {
        Err(CompatError::InvalidRelativePath(relative.to_path_buf()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_rejects_parent_traversal() {
        let unique = format!(
            "apksule-storage-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        let base = std::env::temp_dir().join(unique);
        let storage = AppStorage::for_package_at(&base, "org.example.notes").expect("storage");

        assert!(storage.resolve_file("../outside.txt").is_err());
        assert!(storage.resolve_file("notes/one.txt").is_ok());

        let _ = std::fs::remove_dir_all(base);
    }
}

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use zip::ZipArchive;

use crate::error::{ApkError, Result};
use crate::manifest::{
    ActivityInfo, ApkVersion, ComponentInfo, ManifestInfo, SdkRequirements, parse_manifest,
};

const MANIFEST_PATH: &str = "AndroidManifest.xml";

/// Metadata for one file in the APK ZIP central directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApkEntry {
    pub path: String,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    pub is_directory: bool,
}

/// Resource and bytecode inventory discovered without decoding resources.arsc.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceInventory {
    pub has_resource_table: bool,
    pub dex_entries: Vec<String>,
    pub asset_entries: Vec<String>,
    pub resource_entries: Vec<String>,
    pub native_libraries: Vec<String>,
}

/// Self-contained launch metadata plus a reference to the original APK.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApkPackage {
    pub source_path: PathBuf,
    pub package_name: String,
    pub version: ApkVersion,
    pub main_activity: Option<String>,
    pub permissions: Vec<String>,
    pub activities: Vec<ActivityInfo>,
    pub components: Vec<ComponentInfo>,
    pub application_label: Option<String>,
    pub sdk: SdkRequirements,
    pub entries: Vec<ApkEntry>,
    pub resources: ResourceInventory,
}

impl ApkPackage {
    #[must_use]
    pub fn contains_entry(&self, path: &str) -> bool {
        self.entries.iter().any(|entry| entry.path == path)
    }

    /// Read one entry on demand. No APK content is extracted to disk.
    pub fn read_entry(&self, path: &str) -> Result<Vec<u8>> {
        let file = File::open(&self.source_path)
            .map_err(|source| ApkError::Open { path: self.source_path.clone(), source })?;
        let mut archive = ZipArchive::new(file)?;
        let mut entry = archive.by_name(path)?;
        let mut bytes = Vec::with_capacity(entry.size().try_into().unwrap_or(0));
        entry
            .read_to_end(&mut bytes)
            .map_err(|source| ApkError::ReadEntry { entry: path.to_owned(), source })?;
        Ok(bytes)
    }

    /// Load an `assets/` entry while rejecting traversal-like names.
    pub fn read_asset(&self, relative_path: &str) -> Result<Vec<u8>> {
        let normalized = relative_path.replace('\\', "/");
        if normalized.starts_with('/') || normalized.split('/').any(|part| part == "..") {
            return Err(ApkError::Zip(zip::result::ZipError::InvalidArchive(
                "invalid asset path".into(),
            )));
        }
        self.read_entry(&format!("assets/{normalized}"))
    }
}

/// Zero-extraction APK inspector.
#[derive(Debug, Default, Clone, Copy)]
pub struct ApkLoader;

impl ApkLoader {
    pub fn open(path: impl AsRef<Path>) -> Result<ApkPackage> {
        let path = path.as_ref();
        if !path.exists() {
            return Err(ApkError::NotFound(path.to_path_buf()));
        }
        if !path.is_file() {
            return Err(ApkError::NotAFile(path.to_path_buf()));
        }

        let file = File::open(path)
            .map_err(|source| ApkError::Open { path: path.to_path_buf(), source })?;
        let mut archive = ZipArchive::new(file)?;

        let manifest_bytes = {
            let mut manifest = archive.by_name(MANIFEST_PATH).map_err(|error| match error {
                zip::result::ZipError::FileNotFound => ApkError::MissingManifest,
                other => ApkError::Zip(other),
            })?;
            let mut bytes = Vec::with_capacity(manifest.size().try_into().unwrap_or(0));
            manifest.read_to_end(&mut bytes).map_err(|source| ApkError::ReadEntry {
                entry: MANIFEST_PATH.to_owned(),
                source,
            })?;
            bytes
        };

        let manifest = parse_manifest(&manifest_bytes)?;
        let entries = index_entries(&mut archive)?;
        let resources = inventory(&entries);

        Ok(package_from_manifest(path, manifest, entries, resources))
    }
}

fn index_entries(archive: &mut ZipArchive<File>) -> Result<Vec<ApkEntry>> {
    let mut entries = Vec::with_capacity(archive.len());
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        entries.push(ApkEntry {
            path: entry.name().to_owned(),
            compressed_size: entry.compressed_size(),
            uncompressed_size: entry.size(),
            is_directory: entry.is_dir(),
        });
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(entries)
}

fn inventory(entries: &[ApkEntry]) -> ResourceInventory {
    let mut inventory = ResourceInventory {
        has_resource_table: entries.iter().any(|entry| entry.path == "resources.arsc"),
        ..ResourceInventory::default()
    };

    for entry in entries.iter().filter(|entry| !entry.is_directory) {
        if entry.path.starts_with("classes") && has_extension(&entry.path, "dex") {
            inventory.dex_entries.push(entry.path.clone());
        } else if entry.path.starts_with("assets/") {
            inventory.asset_entries.push(entry.path.clone());
        } else if entry.path.starts_with("res/") {
            inventory.resource_entries.push(entry.path.clone());
        } else if entry.path.starts_with("lib/") && has_extension(&entry.path, "so") {
            inventory.native_libraries.push(entry.path.clone());
        }
    }
    inventory
}

fn has_extension(path: &str, expected: &str) -> bool {
    Path::new(path).extension().is_some_and(|extension| extension.eq_ignore_ascii_case(expected))
}

fn package_from_manifest(
    path: &Path,
    manifest: ManifestInfo,
    entries: Vec<ApkEntry>,
    resources: ResourceInventory,
) -> ApkPackage {
    ApkPackage {
        source_path: path.to_path_buf(),
        package_name: manifest.package_name,
        version: manifest.version,
        main_activity: manifest.main_activity,
        permissions: manifest.permissions,
        activities: manifest.activities,
        components: manifest.components,
        application_label: manifest.application_label,
        sdk: manifest.sdk,
        entries,
        resources,
    }
}

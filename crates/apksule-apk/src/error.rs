use std::path::PathBuf;

use thiserror::Error;

/// Failures produced while inspecting an APK without extracting it.
#[derive(Debug, Error)]
pub enum ApkError {
    #[error("APK does not exist: {0}")]
    NotFound(PathBuf),
    #[error("APK path is not a file: {0}")]
    NotAFile(PathBuf),
    #[error("failed to open APK {path}: {source}")]
    Open {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid APK ZIP container: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("APK is missing AndroidManifest.xml")]
    MissingManifest,
    #[error("failed to read APK entry {entry}: {source}")]
    ReadEntry {
        entry: String,
        #[source]
        source: std::io::Error,
    },
    #[error("AndroidManifest.xml could not be decoded (AXML: {axml}; XML: {xml})")]
    ManifestDecode { axml: String, xml: String },
    #[error("AndroidManifest.xml is missing the manifest element")]
    MissingManifestElement,
    #[error("AndroidManifest.xml is missing the package attribute")]
    MissingPackageName,
}

pub type Result<T> = std::result::Result<T, ApkError>;

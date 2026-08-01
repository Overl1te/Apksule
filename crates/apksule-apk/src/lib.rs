//! APK container inspection and AndroidManifest.xml decoding.
//!
//! This crate deliberately does not execute DEX and never extracts an APK.

mod archive;
mod error;
mod manifest;

pub use archive::{ApkEntry, ApkLoader, ApkPackage, ResourceInventory};
pub use error::{ApkError, Result};
pub use manifest::{
    ActivityInfo, ApkVersion, ComponentInfo, ComponentKind, ManifestInfo, SdkRequirements,
    parse_manifest,
};

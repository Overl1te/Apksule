use apksule_apk::ApkPackage;
use apksule_compat::{Context, InputEvent};
use thiserror::Error;

use crate::lifecycle::ActivityState;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum DexStatus {
    #[default]
    NotLoaded,
    Unsupported {
        dex_files: usize,
        reason: String,
    },
}

#[derive(Debug, Error)]
pub enum DexError {
    #[error("compatibility layer failed while handling DEX: {0}")]
    Compat(#[from] apksule_compat::CompatError),
    #[error("DEX runtime failed: {0}")]
    Runtime(String),
}

/// Stable seam for the M2 interpreter. The host never depends on this trait.
pub trait DexRuntime {
    fn load(&mut self, package: &ApkPackage, context: &Context) -> Result<(), DexError>;
    fn on_lifecycle(&mut self, state: ActivityState) -> Result<(), DexError>;
    fn on_input(&mut self, event: &InputEvent) -> Result<(), DexError>;
    fn on_surface_changed(&mut self, width: u32, height: u32) -> Result<(), DexError>;
    fn status(&self) -> &DexStatus;
}

#[derive(Debug, Default)]
pub struct StubDexRuntime {
    status: DexStatus,
    input_events_seen: u64,
}

impl StubDexRuntime {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn input_events_seen(&self) -> u64 {
        self.input_events_seen
    }
}

impl DexRuntime for StubDexRuntime {
    fn load(&mut self, package: &ApkPackage, context: &Context) -> Result<(), DexError> {
        let dex_files = package.resources.dex_entries.len();
        let reason = if dex_files == 0 {
            "APK has no classes*.dex entry".to_owned()
        } else {
            "Dalvik bytecode execution is scheduled for milestone M2".to_owned()
        };
        context.unsupported_api(
            "dalvik.system.DexFile",
            "execute",
            format!("package={} dex_files={} reason={reason}", package.package_name, dex_files),
        )?;
        self.status = DexStatus::Unsupported { dex_files, reason };
        Ok(())
    }

    fn on_lifecycle(&mut self, state: ActivityState) -> Result<(), DexError> {
        tracing::debug!(?state, "Activity lifecycle delivered to DEX boundary");
        Ok(())
    }

    fn on_input(&mut self, _event: &InputEvent) -> Result<(), DexError> {
        self.input_events_seen = self.input_events_seen.saturating_add(1);
        Ok(())
    }

    fn on_surface_changed(&mut self, width: u32, height: u32) -> Result<(), DexError> {
        tracing::debug!(width, height, "APK surface resized");
        Ok(())
    }

    fn status(&self) -> &DexStatus {
        &self.status
    }
}

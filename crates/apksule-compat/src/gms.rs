use serde::{Deserialize, Serialize};

use crate::api_log::ApiLogger;
use crate::error::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GmsAvailability {
    NotRequested,
    StubOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmsDetection {
    pub availability: GmsAvailability,
    pub evidence: Vec<String>,
}

impl GmsDetection {
    pub fn from_signals<'a>(signals: impl IntoIterator<Item = &'a str>) -> Self {
        let mut evidence: Vec<_> = signals
            .into_iter()
            .filter(|signal| {
                let normalized = signal.replace('/', ".");
                normalized.contains("com.google.android.gms")
                    || normalized.contains("com.google.firebase")
                    || normalized.contains("com.android.vending")
            })
            .map(str::to_owned)
            .collect();
        evidence.sort();
        evidence.dedup();
        Self {
            availability: if evidence.is_empty() {
                GmsAvailability::NotRequested
            } else {
                GmsAvailability::StubOnly
            },
            evidence,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StubResponse {
    /// Conventional `GoogleApiAvailability` `SERVICE_MISSING` result.
    ServiceMissing(i32),
    Boolean(bool),
    Null,
    Empty,
    Unsupported {
        reason: String,
    },
}

/// Explicit, non-networked Google Play Services compatibility boundary.
#[derive(Debug, Clone)]
pub struct GmsShim {
    detection: GmsDetection,
    api_log: ApiLogger,
}

impl GmsShim {
    #[must_use]
    pub fn new(detection: GmsDetection, api_log: ApiLogger) -> Self {
        Self { detection, api_log }
    }

    #[must_use]
    pub fn detection(&self) -> &GmsDetection {
        &self.detection
    }

    /// Return deterministic stubs and record that real GMS behavior is absent.
    pub fn call(&self, service: &str, method: &str) -> Result<StubResponse> {
        let response = match (service, method) {
            ("GoogleApiAvailability", "isGooglePlayServicesAvailable") => {
                StubResponse::ServiceMissing(1)
            }
            ("FusedLocationProviderClient", "getLastLocation")
            | ("FirebaseAnalytics", "getInstance") => StubResponse::Null,
            ("Tasks", "forResult") => StubResponse::Empty,
            _ => StubResponse::Unsupported {
                reason: format!("{service}.{method} is outside the M1 compatibility surface"),
            },
        };
        self.api_log.unsupported(
            format!("com.google.android.gms.{service}"),
            method,
            format!("GMS shim returned {response:?}"),
        )?;
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_gms_from_component_names() {
        let detection = GmsDetection::from_signals([
            "android.permission.INTERNET",
            "com.google.android.gms.common.GoogleApiActivity",
        ]);
        assert_eq!(detection.availability, GmsAvailability::StubOnly);
        assert_eq!(detection.evidence.len(), 1);
    }
}

//! Minimal Android API surface used by the Apksule runtime.
//!
//! APIs that are not implemented must cross [`ApiLogger`] so compatibility
//! gaps are observable rather than silently ignored.

mod api_log;
mod context;
mod error;
mod gms;
mod input;
mod resources;
mod storage;
mod window;

pub use api_log::{ApiLogger, UnsupportedApiCall};
pub use context::Context;
pub use error::{CompatError, Result};
pub use gms::{GmsAvailability, GmsDetection, GmsShim, StubResponse};
pub use input::{
    AndroidKeyCode, InputEvent, KeyAction, KeyEvent, MotionAction, MotionEvent, PointerButton,
};
pub use resources::{ResourceSource, Resources};
pub use storage::AppStorage;
pub use window::{WindowDelegate, WindowInsets, WindowMetrics};

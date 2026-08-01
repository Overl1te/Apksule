//! Minimal Android API surface used by the Apksule runtime.
//!
//! APIs that are not implemented must cross [`ApiLogger`] so compatibility
//! gaps are observable rather than silently ignored.

mod api_log;
mod arsc;
mod context;
mod error;
mod gms;
mod input;
mod layout;
mod resources;
mod storage;
mod ui_host;
mod view;
mod window;

pub use api_log::{ApiLogger, UnsupportedApiCall};
pub use arsc::{ResourceEntry, ResourceTable, ResourceValue, build_minimal_arsc};
pub use context::Context;
pub use error::{CompatError, Result};
pub use gms::{GmsAvailability, GmsDetection, GmsShim, StubResponse};
pub use input::{
    AndroidKeyCode, InputEvent, KeyAction, KeyEvent, MotionAction, MotionEvent, PointerButton,
};
pub use layout::{build_minimal_layout_axml, inflate_axml, inflate_layout};
pub use resources::{ResourceSource, Resources};
pub use storage::AppStorage;
pub use ui_host::UiHost;
pub use view::{
    LayoutParams, Orientation, Rect, ViewId, ViewKind, ViewNode, ViewStore, Visibility,
};
pub use window::{WindowDelegate, WindowInsets, WindowMetrics};


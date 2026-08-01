use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowInsets {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowMetrics {
    pub width_px: u32,
    pub height_px: u32,
    pub scale_factor: f64,
    pub insets: WindowInsets,
}

/// Minimal contract exposed to Android surface code instead of a host UI.
pub trait WindowDelegate {
    fn metrics(&self) -> WindowMetrics;
    fn request_redraw(&self);
    fn set_ime_visible(&self, visible: bool);
}

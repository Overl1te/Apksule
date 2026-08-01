use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MotionAction {
    Down,
    Move,
    Up,
    Cancel,
    Scroll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PointerButton {
    Primary,
    Secondary,
    Middle,
    Other(u16),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MotionEvent {
    pub action: MotionAction,
    pub pointer_id: u32,
    pub x: f32,
    pub y: f32,
    pub button: Option<PointerButton>,
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub timestamp_ms: u128,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyAction {
    Down,
    Up,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AndroidKeyCode {
    Back,
    Enter,
    Tab,
    Escape,
    Space,
    Delete,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Character(String),
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyEvent {
    pub action: KeyAction,
    pub key_code: AndroidKeyCode,
    pub text: Option<String>,
    pub repeat: bool,
    pub timestamp_ms: u128,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum InputEvent {
    Motion(MotionEvent),
    Key(KeyEvent),
}

#![allow(clippy::cast_possible_truncation)]

use std::time::{SystemTime, UNIX_EPOCH};

use apksule_compat::{
    AndroidKeyCode, InputEvent, KeyAction, KeyEvent, MotionAction, MotionEvent, PointerButton,
};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::keyboard::{Key, NamedKey};

#[derive(Debug, Default)]
pub struct InputTranslator {
    cursor_x: f32,
    cursor_y: f32,
}

impl InputTranslator {
    pub fn translate(&mut self, event: &WindowEvent) -> Option<InputEvent> {
        match event {
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_x = position.x as f32;
                self.cursor_y = position.y as f32;
                Some(InputEvent::Motion(self.motion(MotionAction::Move, None, 0.0, 0.0)))
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let action = match state {
                    ElementState::Pressed => MotionAction::Down,
                    ElementState::Released => MotionAction::Up,
                };
                Some(InputEvent::Motion(self.motion(action, Some(map_button(*button)), 0.0, 0.0)))
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (scroll_x, scroll_y) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (*x, *y),
                    MouseScrollDelta::PixelDelta(position) => {
                        (position.x as f32, position.y as f32)
                    }
                };
                Some(InputEvent::Motion(self.motion(
                    MotionAction::Scroll,
                    None,
                    scroll_x,
                    scroll_y,
                )))
            }
            WindowEvent::KeyboardInput { event, .. } => Some(InputEvent::Key(KeyEvent {
                action: match event.state {
                    ElementState::Pressed => KeyAction::Down,
                    ElementState::Released => KeyAction::Up,
                },
                key_code: map_key(&event.logical_key),
                text: event.text.as_ref().map(ToString::to_string),
                repeat: event.repeat,
                timestamp_ms: now_ms(),
            })),
            _ => None,
        }
    }

    fn motion(
        &self,
        action: MotionAction,
        button: Option<PointerButton>,
        scroll_x: f32,
        scroll_y: f32,
    ) -> MotionEvent {
        MotionEvent {
            action,
            pointer_id: 0,
            x: self.cursor_x,
            y: self.cursor_y,
            button,
            scroll_x,
            scroll_y,
            timestamp_ms: now_ms(),
        }
    }
}

fn map_button(button: MouseButton) -> PointerButton {
    match button {
        MouseButton::Left => PointerButton::Primary,
        MouseButton::Right => PointerButton::Secondary,
        MouseButton::Middle => PointerButton::Middle,
        MouseButton::Back => PointerButton::Other(4),
        MouseButton::Forward => PointerButton::Other(5),
        MouseButton::Other(value) => PointerButton::Other(value),
    }
}

fn map_key(key: &Key) -> AndroidKeyCode {
    match key {
        Key::Named(NamedKey::BrowserBack) => AndroidKeyCode::Back,
        Key::Named(NamedKey::Enter) => AndroidKeyCode::Enter,
        Key::Named(NamedKey::Tab) => AndroidKeyCode::Tab,
        Key::Named(NamedKey::Escape) => AndroidKeyCode::Escape,
        Key::Named(NamedKey::Space) => AndroidKeyCode::Space,
        Key::Named(NamedKey::Backspace | NamedKey::Delete) => AndroidKeyCode::Delete,
        Key::Named(NamedKey::ArrowUp) => AndroidKeyCode::ArrowUp,
        Key::Named(NamedKey::ArrowDown) => AndroidKeyCode::ArrowDown,
        Key::Named(NamedKey::ArrowLeft) => AndroidKeyCode::ArrowLeft,
        Key::Named(NamedKey::ArrowRight) => AndroidKeyCode::ArrowRight,
        Key::Character(text) => AndroidKeyCode::Character(text.to_string()),
        other => AndroidKeyCode::Unknown(format!("{other:?}")),
    }
}

fn now_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis()
}

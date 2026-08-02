//! Host-owned content view tree for M3.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::doc_markdown,
    clippy::must_use_candidate,
    clippy::significant_drop_tightening,
    clippy::too_many_lines
)]

use std::sync::{Arc, Mutex};

use crate::view::{
    LayoutParams, Orientation, Rect, ViewId, ViewKind, ViewNode, ViewStore, Visibility,
};

/// Shared UI host between the DEX bridge and the runtime renderer.
#[derive(Debug, Default, Clone)]
pub struct UiHost {
    inner: Arc<Mutex<UiHostState>>,
}

#[derive(Debug, Default)]
struct UiHostState {
    store: ViewStore,
    root: Option<ViewId>,
    next_id: u32,
    /// Maps opaque VM object identity (ObjectRef.0) → view id.
    object_views: std::collections::HashMap<u32, ViewId>,
    focused: Option<ViewId>,
    dirty: bool,
    width: u32,
    height: u32,
    /// Last click marker requested by a button (for tests / bridge side effects).
    pending_click_marker: Option<String>,
}

impl UiHost {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn has_content(&self) -> bool {
        self.with(|state| state.root.is_some())
    }

    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.with(|state| state.dirty)
    }

    pub fn clear_dirty(&self) {
        self.with_mut(|state| state.dirty = false);
    }

    pub fn set_surface_size(&self, width: u32, height: u32) {
        self.with_mut(|state| {
            if state.width != width || state.height != height {
                state.width = width;
                state.height = height;
                state.dirty = true;
                if let Some(root) = state.root {
                    layout_tree(&mut state.store, root, width, height);
                }
            }
        });
    }

    pub fn set_content_view(&self, root: ViewId) {
        self.with_mut(|state| {
            state.root = Some(root);
            state.dirty = true;
            layout_tree(&mut state.store, root, state.width.max(1), state.height.max(1));
        });
    }

    pub fn create_view(&self, kind: ViewKind) -> ViewId {
        self.with_mut(|state| {
            let id = ViewId(state.next_id);
            state.next_id = state.next_id.saturating_add(1);
            state.store.insert(ViewNode::new(id, kind));
            state.dirty = true;
            id
        })
    }

    pub fn bind_object(&self, object_id: u32, view: ViewId) {
        self.with_mut(|state| {
            state.object_views.insert(object_id, view);
        });
    }

    #[must_use]
    pub fn view_for_object(&self, object_id: u32) -> Option<ViewId> {
        self.with(|state| state.object_views.get(&object_id).copied())
    }

    pub fn add_child(&self, parent: ViewId, child: ViewId) {
        self.with_mut(|state| {
            if let Some(node) = state.store.get_mut(parent)
                && let Some(children) = node.kind.children_mut()
            {
                children.push(child);
                state.dirty = true;
            }
            if let Some(root) = state.root {
                layout_tree(&mut state.store, root, state.width.max(1), state.height.max(1));
            }
        });
    }

    pub fn set_text(&self, id: ViewId, text: impl Into<String>) {
        let text = text.into();
        self.with_mut(|state| {
            if let Some(node) = state.store.get_mut(id) {
                node.kind.set_text(text);
                state.dirty = true;
                if let Some(root) = state.root {
                    layout_tree(&mut state.store, root, state.width.max(1), state.height.max(1));
                }
            }
        });
    }

    pub fn set_click_marker(&self, id: ViewId, marker: impl Into<String>) {
        self.with_mut(|state| {
            if let Some(node) = state.store.get_mut(id) {
                node.click_marker = Some(marker.into());
            }
        });
    }

    pub fn set_android_id(&self, id: ViewId, android_id: i32) {
        self.with_mut(|state| {
            if let Some(node) = state.store.get_mut(id) {
                node.android_id = android_id;
            }
        });
    }

    #[must_use]
    pub fn find_view_by_android_id(&self, android_id: i32) -> Option<ViewId> {
        self.with(|state| {
            state.store.iter().find_map(|(id, node)| {
                if node.android_id == android_id { Some(*id) } else { None }
            })
        })
    }

    pub fn clear_children(&self, id: ViewId) {
        self.with_mut(|state| {
            if let Some(node) = state.store.get_mut(id)
                && let Some(children) = node.kind.children_mut()
            {
                children.clear();
                state.dirty = true;
            }
        });
    }

    pub fn set_layout_params(&self, id: ViewId, layout: LayoutParams) {
        self.with_mut(|state| {
            if let Some(node) = state.store.get_mut(id) {
                node.layout = layout;
                state.dirty = true;
            }
        });
    }

    /// Snapshot of visible nodes for rendering (depth-first).
    #[must_use]
    pub fn snapshot(&self) -> Vec<ViewNode> {
        self.with(|state| {
            let Some(root) = state.root else {
                return Vec::new();
            };
            let mut out = Vec::new();
            collect_visible(&state.store, root, &mut out);
            out
        })
    }

    /// Dispatch a pointer up; returns click marker if a clickable view was hit.
    pub fn pointer_up(&self, x: i32, y: i32) -> Option<String> {
        self.with_mut(|state| {
            let root = state.root?;
            let hit = hit_test(&state.store, root, x, y)?;
            if let Some(prev) = state.focused
                && prev != hit
                && let Some(previous) = state.store.get_mut(prev)
            {
                previous.focused = false;
            }
            let node = state.store.get_mut(hit)?;
            if node.kind.is_editable() {
                node.focused = true;
                state.focused = Some(hit);
                state.dirty = true;
            }
            if let Some(marker) = node.click_marker.clone() {
                state.pending_click_marker = Some(marker.clone());
                state.dirty = true;
                return Some(marker);
            }
            if matches!(node.kind, ViewKind::Button { .. }) {
                state.dirty = true;
                return Some(String::new());
            }
            None
        })
    }

    /// Append a character to the focused EditText.
    pub fn type_char(&self, ch: char) -> bool {
        self.with_mut(|state| {
            let Some(focused) = state.focused else {
                return false;
            };
            let Some(node) = state.store.get_mut(focused) else {
                return false;
            };
            if !node.kind.is_editable() {
                return false;
            }
            match &mut node.kind {
                ViewKind::EditText { text } => {
                    if ch == '\u{8}' {
                        text.pop();
                    } else if !ch.is_control() {
                        text.push(ch);
                    } else {
                        return false;
                    }
                    state.dirty = true;
                    true
                }
                _ => false,
            }
        })
    }

    pub fn take_pending_click_marker(&self) -> Option<String> {
        self.with_mut(|state| state.pending_click_marker.take())
    }

    fn with<R>(&self, f: impl FnOnce(&UiHostState) -> R) -> R {
        let state = self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&state)
    }

    fn with_mut<R>(&self, f: impl FnOnce(&mut UiHostState) -> R) -> R {
        let mut state = self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&mut state)
    }
}

fn collect_visible(store: &ViewStore, id: ViewId, out: &mut Vec<ViewNode>) {
    let Some(node) = store.get(id) else {
        return;
    };
    if node.visibility == Visibility::Gone {
        return;
    }
    out.push(node.clone());
    if node.visibility == Visibility::Invisible {
        return;
    }
    for child in node.kind.children() {
        collect_visible(store, *child, out);
    }
}

fn hit_test(store: &ViewStore, id: ViewId, x: i32, y: i32) -> Option<ViewId> {
    let node = store.get(id)?;
    if node.visibility != Visibility::Visible || !node.bounds.contains(x, y) {
        return None;
    }
    // Prefer deepest child.
    for child in node.kind.children().iter().rev() {
        if let Some(hit) = hit_test(store, *child, x, y) {
            return Some(hit);
        }
    }
    Some(id)
}

fn layout_tree(store: &mut ViewStore, root: ViewId, width: u32, height: u32) {
    let w = i32::try_from(width).unwrap_or(i32::MAX);
    let h = i32::try_from(height).unwrap_or(i32::MAX);
    measure(store, root, w, h);
    layout(store, root, 0, 0, w, h);
}

fn measure(store: &mut ViewStore, id: ViewId, max_w: i32, max_h: i32) -> (i32, i32) {
    let Some(node) = store.get(id).cloned() else {
        return (0, 0);
    };
    if node.visibility == Visibility::Gone {
        return (0, 0);
    }

    let resolve = |spec: i32, max: i32, content: i32| -> i32 {
        if spec == LayoutParams::MATCH_PARENT {
            max
        } else if spec == LayoutParams::WRAP_CONTENT {
            content.min(max).max(0)
        } else {
            spec.clamp(0, max)
        }
    };

    let (content_w, content_h) = match &node.kind {
        ViewKind::LinearLayout { orientation, children } => {
            let mut cw: i32 = 0;
            let mut ch: i32 = 0;
            for child in children {
                let (w, h) = measure(store, *child, max_w, max_h);
                match orientation {
                    Orientation::Vertical => {
                        cw = cw.max(w);
                        ch = ch.saturating_add(h).saturating_add(8);
                    }
                    Orientation::Horizontal => {
                        ch = ch.max(h);
                        cw = cw.saturating_add(w).saturating_add(8);
                    }
                }
            }
            (cw, ch)
        }
        ViewKind::FrameLayout { children } | ViewKind::RecyclerView { children } => {
            let mut cw: i32 = 0;
            let mut ch: i32 = 0;
            for child in children {
                let (w, h) = measure(store, *child, max_w, max_h);
                cw = cw.max(w);
                ch = ch.max(h);
            }
            (cw, ch)
        }
        ViewKind::TextView { text } | ViewKind::EditText { text } | ViewKind::Button { text } => {
            let w = i32::try_from(text.chars().count().saturating_mul(12).saturating_add(24))
                .unwrap_or(i32::MAX);
            let h = if matches!(node.kind, ViewKind::Button { .. }) { 40 } else { 28 };
            (w, h)
        }
        ViewKind::View => (max_w.min(100), 24),
    };

    let width = resolve(node.layout.width, max_w, content_w);
    let height = resolve(node.layout.height, max_h, content_h);
    if let Some(node) = store.get_mut(id) {
        // stash measured size in bounds temporarily (left/top = 0)
        node.bounds = Rect { left: 0, top: 0, right: width, bottom: height };
    }
    (width, height)
}

fn layout(store: &mut ViewStore, id: ViewId, left: i32, top: i32, width: i32, height: i32) {
    let Some(node) = store.get(id).cloned() else {
        return;
    };
    if let Some(slot) = store.get_mut(id) {
        slot.bounds = Rect { left, top, right: left.saturating_add(width), bottom: top.saturating_add(height) };
    }
    match node.kind {
        ViewKind::LinearLayout { orientation, children } => {
            let mut cursor_x = left.saturating_add(12);
            let mut cursor_y = top.saturating_add(12);
            for child in children {
                let child_bounds = store.get(child).map(|n| n.bounds).unwrap_or_default();
                let cw = child_bounds.width();
                let ch = child_bounds.height();
                layout(store, child, cursor_x, cursor_y, cw, ch);
                match orientation {
                    Orientation::Vertical => cursor_y = cursor_y.saturating_add(ch).saturating_add(8),
                    Orientation::Horizontal => cursor_x = cursor_x.saturating_add(cw).saturating_add(8),
                }
            }
        }
        ViewKind::FrameLayout { children } | ViewKind::RecyclerView { children } => {
            for child in children {
                let child_bounds = store.get(child).map(|n| n.bounds).unwrap_or_default();
                layout(
                    store,
                    child,
                    left.saturating_add(8),
                    top.saturating_add(8),
                    child_bounds.width(),
                    child_bounds.height(),
                );
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_vertical_linear_and_hits_button() {
        let host = UiHost::new();
        host.set_surface_size(400, 300);
        let root = host.create_view(ViewKind::LinearLayout {
            orientation: Orientation::Vertical,
            children: Vec::new(),
        });
        let label = host.create_view(ViewKind::TextView { text: "Hello".into() });
        let button = host.create_view(ViewKind::Button { text: "Go".into() });
        host.add_child(root, label);
        host.add_child(root, button);
        host.set_click_marker(button, "clicked");
        host.set_content_view(root);

        let nodes = host.snapshot();
        assert_eq!(nodes.len(), 3);
        let button_node = nodes.iter().find(|n| n.kind.text() == Some("Go")).expect("button");
        let x = i32::midpoint(button_node.bounds.left, button_node.bounds.right);
        let y = i32::midpoint(button_node.bounds.top, button_node.bounds.bottom);
        assert_eq!(host.pointer_up(x, y).as_deref(), Some("clicked"));
    }
}

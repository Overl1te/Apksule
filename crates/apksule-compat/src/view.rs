//! Minimal Android View model for M3 (no Skia — runtime draws this tree).

#![allow(clippy::doc_markdown, clippy::must_use_candidate)]

use std::collections::HashMap;

/// Stable handle for a view node inside [`crate::UiHost`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ViewId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    Vertical,
    Horizontal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Visible,
    Invisible,
    Gone,
}

/// Layout dimension: match_parent (-1), wrap_content (-2), or exact pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutParams {
    pub width: i32,
    pub height: i32,
}

impl LayoutParams {
    pub const MATCH_PARENT: i32 = -1;
    pub const WRAP_CONTENT: i32 = -2;

    #[must_use]
    pub const fn match_parent() -> Self {
        Self { width: Self::MATCH_PARENT, height: Self::MATCH_PARENT }
    }

    #[must_use]
    pub const fn wrap_content() -> Self {
        Self { width: Self::WRAP_CONTENT, height: Self::WRAP_CONTENT }
    }
}

impl Default for LayoutParams {
    fn default() -> Self {
        Self::wrap_content()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    #[must_use]
    pub const fn width(self) -> i32 {
        self.right.saturating_sub(self.left)
    }

    #[must_use]
    pub const fn height(self) -> i32 {
        self.bottom.saturating_sub(self.top)
    }

    #[must_use]
    pub const fn contains(self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewKind {
    View,
    TextView { text: String },
    EditText { text: String },
    Button { text: String },
    LinearLayout { orientation: Orientation, children: Vec<ViewId> },
    FrameLayout { children: Vec<ViewId> },
}

impl ViewKind {
    #[must_use]
    pub fn children(&self) -> &[ViewId] {
        match self {
            Self::LinearLayout { children, .. } | Self::FrameLayout { children } => children,
            _ => &[],
        }
    }

    pub fn children_mut(&mut self) -> Option<&mut Vec<ViewId>> {
        match self {
            Self::LinearLayout { children, .. } | Self::FrameLayout { children } => Some(children),
            _ => None,
        }
    }

    #[must_use]
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::TextView { text } | Self::EditText { text } | Self::Button { text } => {
                Some(text.as_str())
            }
            _ => None,
        }
    }

    pub fn set_text(&mut self, value: impl Into<String>) {
        match self {
            Self::TextView { text } | Self::EditText { text } | Self::Button { text } => {
                *text = value.into();
            }
            _ => {}
        }
    }

    #[must_use]
    pub const fn is_editable(&self) -> bool {
        matches!(self, Self::EditText { .. })
    }

    #[must_use]
    pub const fn is_clickable(&self) -> bool {
        matches!(self, Self::Button { .. } | Self::TextView { .. } | Self::View)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewNode {
    pub id: ViewId,
    pub android_id: i32,
    pub kind: ViewKind,
    pub layout: LayoutParams,
    pub bounds: Rect,
    pub visibility: Visibility,
    /// Optional marker written to app storage when clicked (test / bridge hook).
    pub click_marker: Option<String>,
    pub focused: bool,
}

impl ViewNode {
    #[must_use]
    pub fn new(id: ViewId, kind: ViewKind) -> Self {
        Self {
            id,
            android_id: 0,
            kind,
            layout: LayoutParams::default(),
            bounds: Rect::default(),
            visibility: Visibility::Visible,
            click_marker: None,
            focused: false,
        }
    }
}

/// Flat store used while inflating or constructing trees.
#[derive(Debug, Default, Clone)]
pub struct ViewStore {
    nodes: HashMap<ViewId, ViewNode>,
}

impl ViewStore {
    #[must_use]
    pub fn get(&self, id: ViewId) -> Option<&ViewNode> {
        self.nodes.get(&id)
    }

    pub fn get_mut(&mut self, id: ViewId) -> Option<&mut ViewNode> {
        self.nodes.get_mut(&id)
    }

    pub fn insert(&mut self, node: ViewNode) {
        self.nodes.insert(node.id, node);
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&ViewId, &ViewNode)> {
        self.nodes.iter()
    }
}

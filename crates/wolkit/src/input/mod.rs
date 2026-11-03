// SPDX-License-Identifier: BSD-2-Clause
//! Input routing for every backend that gets real or injected input (plan section 2.7): hit test,
//! then focus, then pointer capture and bubbling; keyboard navigation and the focus ring; the
//! editor behind `text_input`; coalescing of pointer motion and scroll to one per frame.
//!
//! Owner: WK-W4. No Wayland and no D-Bus here: the native backend (`window::native`) turns
//! protocol events into the calls below, the headless backend turns injected ones into the same
//! calls, so both route alike and `tests/native_input.rs` covers both.
//!
//! - [`hit`]: the deepest node under a point (overlays first, scroll offsets applied, a modal
//!   layer absorbs the rest of the window).
//! - [`focus`]: tab order, arrow keys in lists, the modal focus trap, focus return, focus-visible.
//! - [`pointer`]: hover, press with pointer capture, click, slider drag, scroll, context menu.
//! - [`keys`]: key routing, activation, Escape, shortcuts (`cmd+s` form).
//! - [`edit`]: the focused `text_input`'s editor (`text::edit::Editor`, WK-W3), IME and clipboard.
//! - [`coalesce`]: one pointer move and one scroll sum per frame.
//!
//! Every event goes to one receiver: the nearest ancestor (or the node) that bound its kind
//! (`window::route::receiver`), never the whole path.

pub mod coalesce;
pub mod edit;
pub mod focus;
pub mod hit;
pub mod keys;
pub mod pointer;

use crate::scene::{Idx, NIL};

/// `PointerInput::button` values (evdev codes are mapped to these by the native seat).
pub const BUTTON_LEFT: u32 = 1;
pub const BUTTON_MIDDLE: u32 = 2;
pub const BUTTON_RIGHT: u32 = 3;

/// Focus-return entries kept for nested modal layers.
pub const MAX_FOCUS_RETURN: usize = 8;

/// Per-window input state that is not in the scene.
#[derive(Default)]
pub struct InputState {
    /// The editor of the focused `text_input`, if any.
    pub field: edit::Field,
    /// Pending pointer motion and scroll.
    pub coalesce: coalesce::Coalescer,
    /// The node input last put the focus ring on (to see programmatic `FOCUS` ops).
    pub focused: Idx,
    pub focused_id: crate::scene::NodeId,
    /// The last focus change came from the keyboard: programmatic focus shows the ring too.
    pub keyboard_mode: bool,
    /// Where focus goes back to when a modal layer closes (innermost last).
    pub focus_return: Vec<(Idx, crate::scene::NodeId)>,
    /// The modal layer that holds focus, if any.
    pub modal: Idx,
    /// Hit-test cache: overlays and modal layers in tree order, for `scene_version`.
    pub layers: hit::Layers,
    pub(crate) chain_a: Vec<Idx>,
    pub(crate) chain_b: Vec<Idx>,
}

impl InputState {
    pub fn new() -> InputState {
        InputState {
            focused: NIL,
            modal: NIL,
            ..Default::default()
        }
    }
}

/// What a backend must do after a key or a text edit that the engine cannot finish alone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Request {
    #[default]
    None,
    /// Put this text on the clipboard (copy or cut in a text field).
    Copy(String),
    /// Read the clipboard and call [`edit::paste`] with its text.
    Paste,
    /// The focused field changed: enable (true) or disable the input method, and send it the
    /// surrounding text and cursor rectangle.
    Ime(bool),
}

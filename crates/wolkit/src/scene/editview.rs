// SPDX-License-Identifier: BSD-2-Clause
//! What the painter needs to know about a text field being edited or scrolled. The editor itself lives with the
//! input state (`input::edit::Field`); after every key, click or input-method batch the input layer
//! copies the drawable part of it here, so paint reads it through [`super::SceneRead::edit_view`]
//! and never reaches into the input state. A view is kept after the field loses focus, with
//! `active` false, so a multi-line field keeps where it was scrolled to.

use std::ops::Range;

use super::ids::{Idx, NodeId};

/// Views the scene keeps at once.
pub const MAX_EDIT_VIEWS: usize = 8;

#[derive(Clone, Debug, PartialEq)]
pub struct EditView {
    pub node: Idx,
    pub id: NodeId,
    /// The field has focus: draw the caret, the selection and the preedit.
    pub active: bool,
    /// The text to draw: the committed text with the preedit inside it (byte offsets below are
    /// into this string). Meaningful while `active`.
    pub text: String,
    /// The node's own text when the view was made: if the app has changed it since, the view is
    /// stale and paint draws the node's text without a caret until the input layer catches up.
    pub base: String,
    pub caret: usize,
    pub selection: Range<usize>,
    pub preedit: Option<Range<usize>>,
    /// How far the field's text is scrolled, in logical pixels (x, y).
    pub scroll: (f32, f32),
}

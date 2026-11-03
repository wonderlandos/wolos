// SPDX-License-Identifier: BSD-2-Clause
//! The assistive-technology vocabulary: the wire numbers of roles and states the desktop
//! accessibility bus uses (its role and state enumerations, stable for two decades), and how the
//! engine's roles (`WOLKIT_ROLE_*`, `scene::kind::ROLES`) and node kinds map onto them.

use crate::scene::Kind;

/// Wire role numbers (only the ones the engine uses).
pub mod role {
    pub const ALERT: u32 = 2;
    pub const CANVAS: u32 = 6;
    pub const CHECK_BOX: u32 = 7;
    pub const DIALOG: u32 = 16;
    pub const DRAWING_AREA: u32 = 18;
    pub const FILLER: u32 = 20;
    pub const FRAME: u32 = 23;
    pub const IMAGE: u32 = 27;
    pub const LABEL: u32 = 29;
    pub const LIST: u32 = 31;
    pub const LIST_ITEM: u32 = 32;
    pub const MENU: u32 = 33;
    pub const MENU_ITEM: u32 = 35;
    pub const PAGE_TAB: u32 = 37;
    pub const PAGE_TAB_LIST: u32 = 38;
    pub const PANEL: u32 = 39;
    pub const PASSWORD_TEXT: u32 = 40;
    pub const PROGRESS_BAR: u32 = 42;
    pub const BUTTON: u32 = 43;
    pub const SCROLL_PANE: u32 = 49;
    pub const SEPARATOR: u32 = 50;
    pub const SLIDER: u32 = 51;
    pub const TOGGLE_BUTTON: u32 = 62;
    pub const APPLICATION: u32 = 75;
    pub const ENTRY: u32 = 79;
    pub const HEADING: u32 = 83;
    pub const LINK: u32 = 88;
}

/// Wire state bit numbers (bit `n` of the 64-bit state set).
pub mod st {
    pub const ACTIVE: u32 = 1;
    pub const BUSY: u32 = 3;
    pub const CHECKED: u32 = 4;
    pub const EDITABLE: u32 = 7;
    pub const ENABLED: u32 = 8;
    pub const EXPANDABLE: u32 = 9;
    pub const EXPANDED: u32 = 10;
    pub const FOCUSABLE: u32 = 11;
    pub const FOCUSED: u32 = 12;
    pub const HORIZONTAL: u32 = 14;
    pub const MODAL: u32 = 16;
    pub const PRESSED: u32 = 20;
    pub const RESIZABLE: u32 = 21;
    pub const SELECTABLE: u32 = 22;
    pub const SELECTED: u32 = 23;
    pub const SENSITIVE: u32 = 24;
    pub const SHOWING: u32 = 25;
    pub const SINGLE_LINE: u32 = 26;
    pub const VISIBLE: u32 = 30;
    pub const SELECTABLE_TEXT: u32 = 38;
    pub const CHECKABLE: u32 = 41;
    pub const READ_ONLY: u32 = 43;
    /// Names for the dump, in bit order.
    pub const NAMES: &[(u32, &str)] = &[
        (ACTIVE, "active"),
        (BUSY, "busy"),
        (CHECKED, "checked"),
        (EDITABLE, "editable"),
        (ENABLED, "enabled"),
        (EXPANDABLE, "expandable"),
        (EXPANDED, "expanded"),
        (FOCUSABLE, "focusable"),
        (FOCUSED, "focused"),
        (HORIZONTAL, "horizontal"),
        (MODAL, "modal"),
        (PRESSED, "pressed"),
        (RESIZABLE, "resizable"),
        (SELECTABLE, "selectable"),
        (SELECTED, "selected"),
        (SENSITIVE, "sensitive"),
        (SHOWING, "showing"),
        (SINGLE_LINE, "single-line"),
        (VISIBLE, "visible"),
        (SELECTABLE_TEXT, "selectable-text"),
        (CHECKABLE, "checkable"),
        (READ_ONLY, "read-only"),
    ];
}

/// `WOLKIT_ROLE_*` ids (`scene::kind::ROLES`).
pub mod wk {
    pub const NONE: u32 = 0;
    pub const GROUP: u32 = 1;
    pub const LABEL: u32 = 2;
    pub const BUTTON: u32 = 3;
    pub const TOGGLE: u32 = 4;
    pub const CHECKBOX: u32 = 5;
    pub const SLIDER: u32 = 6;
    pub const TEXTBOX: u32 = 7;
    pub const IMAGE: u32 = 8;
    pub const LIST: u32 = 9;
    pub const LISTITEM: u32 = 10;
    pub const HEADING: u32 = 11;
    pub const TAB: u32 = 12;
    pub const TABLIST: u32 = 13;
    pub const TABPANEL: u32 = 14;
    pub const MENU: u32 = 15;
    pub const MENUITEM: u32 = 16;
    pub const DIALOG: u32 = 17;
    pub const ALERT: u32 = 18;
    pub const SEPARATOR: u32 = 19;
    pub const SCROLLVIEW: u32 = 20;
    pub const LINK: u32 = 21;
    pub const PROGRESS: u32 = 22;
}

/// The engine role a node has: the app's (`SET_A11Y`) or the one its kind implies.
pub fn engine_role(kind: Kind, app_role: u32) -> u32 {
    if app_role != wk::NONE {
        return app_role;
    }
    match kind {
        Kind::Text => wk::LABEL,
        Kind::Image => wk::IMAGE,
        Kind::Divider => wk::SEPARATOR,
        Kind::Button => wk::BUTTON,
        Kind::Toggle => wk::TOGGLE,
        Kind::Slider => wk::SLIDER,
        Kind::TextInput => wk::TEXTBOX,
        Kind::Scroll => wk::SCROLLVIEW,
        Kind::Vlist => wk::LIST,
        Kind::Box | Kind::Spacer | Kind::Canvas | Kind::Surface | Kind::Overlay => wk::NONE,
    }
}

/// The wire role of a node with engine role `r` (from [`engine_role`]).
pub fn wire_role(kind: Kind, r: u32, secure: bool) -> u32 {
    match r {
        wk::GROUP | wk::TABPANEL => role::PANEL,
        wk::LABEL => role::LABEL,
        wk::BUTTON => role::BUTTON,
        wk::TOGGLE => role::TOGGLE_BUTTON,
        wk::CHECKBOX => role::CHECK_BOX,
        wk::SLIDER => role::SLIDER,
        wk::TEXTBOX if secure => role::PASSWORD_TEXT,
        wk::TEXTBOX => role::ENTRY,
        wk::IMAGE => role::IMAGE,
        wk::LIST => role::LIST,
        wk::LISTITEM => role::LIST_ITEM,
        wk::HEADING => role::HEADING,
        wk::TAB => role::PAGE_TAB,
        wk::TABLIST => role::PAGE_TAB_LIST,
        wk::MENU => role::MENU,
        wk::MENUITEM => role::MENU_ITEM,
        wk::DIALOG => role::DIALOG,
        wk::ALERT => role::ALERT,
        wk::SEPARATOR => role::SEPARATOR,
        wk::SCROLLVIEW => role::SCROLL_PANE,
        wk::LINK => role::LINK,
        wk::PROGRESS => role::PROGRESS_BAR,
        _ => match kind {
            Kind::Canvas => role::CANVAS,
            Kind::Surface => role::DRAWING_AREA,
            Kind::Spacer => role::FILLER,
            _ => role::PANEL,
        },
    }
}

/// The name the wire uses for a role, for the dump and `GetRoleName`.
pub fn role_name(r: u32) -> &'static str {
    match r {
        role::ALERT => "alert",
        role::CANVAS => "canvas",
        role::CHECK_BOX => "check box",
        role::DIALOG => "dialog",
        role::DRAWING_AREA => "drawing area",
        role::FILLER => "filler",
        role::FRAME => "frame",
        role::IMAGE => "image",
        role::LABEL => "label",
        role::LIST => "list",
        role::LIST_ITEM => "list item",
        role::MENU => "menu",
        role::MENU_ITEM => "menu item",
        role::PAGE_TAB => "page tab",
        role::PAGE_TAB_LIST => "page tab list",
        role::PANEL => "panel",
        role::PASSWORD_TEXT => "password text",
        role::PROGRESS_BAR => "progress bar",
        role::BUTTON => "button",
        role::SCROLL_PANE => "scroll pane",
        role::SEPARATOR => "separator",
        role::SLIDER => "slider",
        role::TOGGLE_BUTTON => "toggle button",
        role::APPLICATION => "application",
        role::ENTRY => "entry",
        role::HEADING => "heading",
        role::LINK => "link",
        _ => "unknown",
    }
}

/// Roles whose name comes from the text inside them (a button's label), so that text is not
/// read a second time as its own label.
pub fn named_by_content(r: u32) -> bool {
    matches!(
        r,
        wk::BUTTON | wk::TOGGLE | wk::CHECKBOX | wk::TAB | wk::MENUITEM | wk::LINK | wk::HEADING
    )
}

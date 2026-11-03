// SPDX-License-Identifier: BSD-2-Clause
//! Node kinds (wolkit.h `WOLKIT_KIND_*`). Ids are append-only and snapshot-tested in
//! `abi/wolkit-1.0.props`; a new primitive adds an id, never a symbol.

macro_rules! kinds {
    ($($id:literal $Var:ident $name:literal;)*) => {
        /// What the engine must own behaviour for (plan section 2.2).
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[repr(u8)]
        pub enum Kind { $($Var = $id,)* }
        impl Kind {
            pub const ALL: &'static [Kind] = &[$(Kind::$Var,)*];
            pub fn from_u32(v: u32) -> Option<Kind> {
                match v { $($id => Some(Kind::$Var),)* _ => None }
            }
            /// The wire id (`WOLKIT_KIND_*`).
            pub fn id(self) -> u32 { match self { $(Kind::$Var => $id,)* } }
            /// The wire id as a byte (the scene stores kinds in a `u8` column).
            pub fn id_u8(self) -> u8 { match self { $(Kind::$Var => $id,)* } }
            pub fn name(self) -> &'static str { match self { $(Kind::$Var => $name,)* } }
            /// The C name: `WOLKIT_KIND_BOX`.
            pub fn define(self) -> String { format!("WOLKIT_KIND_{}", self.name().to_ascii_uppercase()) }
        }
    };
}

kinds! {
    1 Box "box";
    2 Text "text";
    3 Image "image";
    4 Spacer "spacer";
    5 Divider "divider";
    6 Button "button";
    7 Toggle "toggle";
    8 Slider "slider";
    9 TextInput "text_input";
    10 Scroll "scroll";
    11 Vlist "vlist";
    12 Canvas "canvas";
    13 Surface "surface";
    14 Overlay "overlay";
}

impl Kind {
    /// Kinds that take pointer and key focus by themselves.
    pub fn focusable(self) -> bool {
        matches!(
            self,
            Kind::Button | Kind::Toggle | Kind::Slider | Kind::TextInput
        )
    }

    /// Kinds with no children (the validator refuses an INSERT below them).
    pub fn leaf(self) -> bool {
        matches!(
            self,
            Kind::Text
                | Kind::Image
                | Kind::Spacer
                | Kind::Divider
                | Kind::Slider
                | Kind::Canvas
                | Kind::Surface
                | Kind::TextInput
        )
    }
}

/// Accessibility roles (`WOLKIT_ROLE_*`), append-only.
pub const ROLES: &[(u32, &str)] = &[
    (0, "NONE"),
    (1, "GROUP"),
    (2, "LABEL"),
    (3, "BUTTON"),
    (4, "TOGGLE"),
    (5, "CHECKBOX"),
    (6, "SLIDER"),
    (7, "TEXTBOX"),
    (8, "IMAGE"),
    (9, "LIST"),
    (10, "LISTITEM"),
    (11, "HEADING"),
    (12, "TAB"),
    (13, "TABLIST"),
    (14, "TABPANEL"),
    (15, "MENU"),
    (16, "MENUITEM"),
    (17, "DIALOG"),
    (18, "ALERT"),
    (19, "SEPARATOR"),
    (20, "SCROLLVIEW"),
    (21, "LINK"),
    (22, "PROGRESS"),
];

/// State bits (`WOLKIT_STATE_*`). The first four are owned by the engine, the rest by the app
/// (`SET_STATE` replaces the app-owned bits and ignores the others).
pub mod state {
    pub const HOVER: u32 = 1;
    pub const ACTIVE: u32 = 2;
    pub const FOCUS: u32 = 4;
    pub const FOCUS_VISIBLE: u32 = 8;
    pub const DISABLED: u32 = 16;
    pub const SELECTED: u32 = 32;
    pub const CHECKED: u32 = 64;
    pub const EXPANDED: u32 = 128;
    pub const EMPTY: u32 = 256;
    pub const BUSY: u32 = 512;
    pub const ENGINE: u32 = HOVER | ACTIVE | FOCUS | FOCUS_VISIBLE | EMPTY;
    pub const APP: u32 = DISABLED | SELECTED | CHECKED | EXPANDED | BUSY;
    pub const NAMES: &[(&str, u32)] = &[
        ("HOVER", HOVER),
        ("ACTIVE", ACTIVE),
        ("FOCUS", FOCUS),
        ("FOCUS_VISIBLE", FOCUS_VISIBLE),
        ("DISABLED", DISABLED),
        ("SELECTED", SELECTED),
        ("CHECKED", CHECKED),
        ("EXPANDED", EXPANDED),
        ("EMPTY", EMPTY),
        ("BUSY", BUSY),
    ];
}

/// Dirty bits kept per node (`Scene::dirty`). `SUBTREE` says some descendant has a bit set.
pub mod dirty {
    pub const STYLE: u8 = 1;
    pub const LAYOUT: u8 = 2;
    pub const PAINT: u8 = 4;
    pub const CHILDREN: u8 = 8;
    pub const A11Y: u8 = 16;
    pub const SUBTREE: u8 = 32;
    pub const ALL: u8 = STYLE | LAYOUT | PAINT | CHILDREN | A11Y;
}

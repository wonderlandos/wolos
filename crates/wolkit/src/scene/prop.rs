// SPDX-License-Identifier: BSD-2-Clause
//! The property table (wolkit.h `WOLKIT_PROP_*`): which inline properties a node can carry, their
//! wire type and the enum names. Ids are append-only and snapshot-tested in
//! `abi/wolkit-1.0.props`. This table says what can cross the wire and how a value is checked;
//! what a property means (resolution, transitions, lints) belongs to `style`.
//!
//! A value on the wire is `Ty::arity()` little-endian 32-bit words after `(node, prop)`.

use crate::scene::ids::Ix;

/// Wire type of a property value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ty {
    /// One `f32` (finite).
    F32,
    /// One `i32`.
    I32,
    /// One word, 0 or 1.
    Bool,
    /// One word, an index into the property's enum names.
    Enum,
    /// One word, a colour token id (0..13, the order of `shared::theme::TOKENS`).
    Token,
    /// One word, a string reference into the patch's pool (or `NO_STR`).
    Str,
    /// One word, any `u32`: an id of a resource (image, anchor node).
    U32,
    /// Two words: unit (0 auto, 1 px, 2 percent) then an `f32`.
    Dim,
    /// Two `f32`: a point or a scale.
    Pair,
    /// Two words: width `f32`, colour token.
    Border,
    /// Five words: angle `f32`, stop count (2 or 3), three colour tokens.
    Gradient,
    /// Four words: dx, dy, blur `f32`, colour token.
    Shadow,
    /// Three words: property id (0 = all), duration (0 fast, 1 normal, 2 slow), easing (enum).
    Transition,
}

impl Ty {
    pub const fn arity(self) -> usize {
        match self {
            Ty::F32 | Ty::I32 | Ty::Bool | Ty::Enum | Ty::Token | Ty::Str | Ty::U32 => 1,
            Ty::Dim | Ty::Pair | Ty::Border => 2,
            Ty::Transition => 3,
            Ty::Shadow => 4,
            Ty::Gradient => 5,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Ty::F32 => "f32",
            Ty::I32 => "i32",
            Ty::Bool => "bool",
            Ty::Enum => "enum",
            Ty::Token => "token",
            Ty::Str => "str",
            Ty::U32 => "u32",
            Ty::Dim => "dim",
            Ty::Pair => "pair",
            Ty::Border => "border",
            Ty::Gradient => "gradient",
            Ty::Shadow => "shadow",
            Ty::Transition => "transition",
        }
    }
}

/// Most words one value takes.
pub const MAX_WORDS: usize = 5;
/// Colour tokens (`shared::theme::TOKENS`).
pub const TOKENS: u32 = 13;
/// Dim units.
pub const DIM_AUTO: u32 = 0;
pub const DIM_PX: u32 = 1;
pub const DIM_PERCENT: u32 = 2;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PropDef {
    pub id: u16,
    pub name: &'static str,
    /// `FLEX_DIRECTION` (the `WOLKIT_PROP_` suffix).
    pub ident: &'static str,
    pub ty: Ty,
    pub enums: &'static [&'static str],
}

const NONE: &[&str] = &[];
const DISPLAY: &[&str] = &["flex", "none"];
const DIRECTION: &[&str] = &["row", "column", "row-reverse", "column-reverse"];
const WRAP: &[&str] = &["nowrap", "wrap"];
const ALIGN: &[&str] = &["start", "center", "end", "stretch", "baseline"];
const SELF_ALIGN: &[&str] = &["auto", "start", "center", "end", "stretch", "baseline"];
const CONTENT: &[&str] = &[
    "start",
    "center",
    "end",
    "stretch",
    "space-between",
    "space-around",
];
const JUSTIFY: &[&str] = &[
    "start",
    "center",
    "end",
    "space-between",
    "space-around",
    "space-evenly",
];
const POSITION: &[&str] = &["relative", "absolute"];
const OVERFLOW: &[&str] = &["visible", "hidden", "scroll", "auto"];
const CURSOR: &[&str] = &[
    "default",
    "pointer",
    "text",
    "grab",
    "grabbing",
    "not-allowed",
];
const POINTER: &[&str] = &["auto", "none"];
const VISIBILITY: &[&str] = &["visible", "hidden"];
const BACKDROP: &[&str] = &["none", "glass"];
const FAMILY: &[&str] = &["ui", "mono"];
const SIZE: &[&str] = &["display", "large", "title", "body", "secondary", "caption"];
const WEIGHT: &[&str] = &["400", "500", "600", "700"];
const FSTYLE: &[&str] = &["normal", "italic"];
const TALIGN: &[&str] = &["start", "center", "end", "justify"];
const DECORATION: &[&str] = &["none", "underline", "strikethrough"];
const TOVERFLOW: &[&str] = &["clip", "ellipsis"];
const WHITESPACE: &[&str] = &["normal", "nowrap", "pre-wrap"];
const FEATURE: &[&str] = &["none", "tnum"];
const SELECT: &[&str] = &["none", "text"];
const SPRING: &[&str] = &["gentle", "snappy", "bouncy"];
const FIT: &[&str] = &["contain", "cover", "fill", "none"];
const SIDE: &[&str] = &["bottom", "top", "right", "left"];
const ANCHOR_ALIGN: &[&str] = &["start", "center", "end"];

macro_rules! props {
    ($($id:literal $IDENT:ident $name:literal $ty:ident $enums:expr;)*) => {
        /// Property ids as constants: `id::FLEX_DIRECTION`.
        pub mod id { $(pub const $IDENT: u16 = $id;)* }
        /// Every property, in id order.
        pub const TABLE: &[PropDef] = &[
            $(PropDef { id: $id, name: $name, ident: stringify!($IDENT), ty: Ty::$ty, enums: $enums },)*
        ];
    };
}

props! {
    // Layout
    1 DISPLAY "display" Enum DISPLAY;
    2 FLEX_DIRECTION "flex-direction" Enum DIRECTION;
    3 FLEX_WRAP "flex-wrap" Enum WRAP;
    4 GROW "grow" F32 NONE;
    5 SHRINK "shrink" F32 NONE;
    6 BASIS "basis" Dim NONE;
    7 GAP "gap" F32 NONE;
    8 ROW_GAP "row-gap" F32 NONE;
    9 COLUMN_GAP "column-gap" F32 NONE;
    10 ALIGN_ITEMS "align-items" Enum ALIGN;
    11 ALIGN_SELF "align-self" Enum SELF_ALIGN;
    12 ALIGN_CONTENT "align-content" Enum CONTENT;
    13 JUSTIFY_CONTENT "justify-content" Enum JUSTIFY;
    14 PADDING "padding" F32 NONE;
    15 PADDING_TOP "padding-top" F32 NONE;
    16 PADDING_RIGHT "padding-right" F32 NONE;
    17 PADDING_BOTTOM "padding-bottom" F32 NONE;
    18 PADDING_LEFT "padding-left" F32 NONE;
    19 MARGIN "margin" Dim NONE;
    20 MARGIN_TOP "margin-top" Dim NONE;
    21 MARGIN_RIGHT "margin-right" Dim NONE;
    22 MARGIN_BOTTOM "margin-bottom" Dim NONE;
    23 MARGIN_LEFT "margin-left" Dim NONE;
    24 WIDTH "width" Dim NONE;
    25 HEIGHT "height" Dim NONE;
    26 MIN_WIDTH "min-width" Dim NONE;
    27 MIN_HEIGHT "min-height" Dim NONE;
    28 MAX_WIDTH "max-width" Dim NONE;
    29 MAX_HEIGHT "max-height" Dim NONE;
    30 ASPECT_RATIO "aspect-ratio" F32 NONE;
    31 POSITION "position" Enum POSITION;
    32 INSET_TOP "inset-top" Dim NONE;
    33 INSET_RIGHT "inset-right" Dim NONE;
    34 INSET_BOTTOM "inset-bottom" Dim NONE;
    35 INSET_LEFT "inset-left" Dim NONE;
    36 OVERFLOW "overflow" Enum OVERFLOW;
    37 Z_INDEX "z-index" I32 NONE;
    38 TAB_INDEX "tab-index" I32 NONE;
    // Paint
    50 BACKGROUND "background" Token NONE;
    51 BACKGROUND_GRADIENT "background-gradient" Gradient NONE;
    52 BORDER "border" Border NONE;
    53 BORDER_RADIUS "border-radius" F32 NONE;
    54 RADIUS_TL "radius-top-left" F32 NONE;
    55 RADIUS_TR "radius-top-right" F32 NONE;
    56 RADIUS_BR "radius-bottom-right" F32 NONE;
    57 RADIUS_BL "radius-bottom-left" F32 NONE;
    58 SHADOW_1 "box-shadow-1" Shadow NONE;
    59 SHADOW_2 "box-shadow-2" Shadow NONE;
    60 OPACITY "opacity" F32 NONE;
    61 OUTLINE "outline" Border NONE;
    62 TRANSLATE "transform-translate" Pair NONE;
    63 SCALE "transform-scale" Pair NONE;
    64 ROTATE "transform-rotate" F32 NONE;
    65 CURSOR "cursor" Enum CURSOR;
    66 POINTER_EVENTS "pointer-events" Enum POINTER;
    67 VISIBILITY "visibility" Enum VISIBILITY;
    68 BACKDROP "backdrop" Enum BACKDROP;
    // Text
    100 COLOR "color" Token NONE;
    101 FONT_FAMILY "font-family" Enum FAMILY;
    102 FONT_SIZE "font-size" Enum SIZE;
    103 FONT_WEIGHT "font-weight" Enum WEIGHT;
    104 FONT_STYLE "font-style" Enum FSTYLE;
    105 LINE_HEIGHT "line-height" F32 NONE;
    106 LETTER_SPACING "letter-spacing" F32 NONE;
    107 TEXT_ALIGN "text-align" Enum TALIGN;
    108 TEXT_DECORATION "text-decoration" Enum DECORATION;
    109 TEXT_OVERFLOW "text-overflow" Enum TOVERFLOW;
    110 WHITE_SPACE "white-space" Enum WHITESPACE;
    111 MAX_LINES "max-lines" I32 NONE;
    112 FONT_FEATURE "font-feature" Enum FEATURE;
    113 USER_SELECT "user-select" Enum SELECT;
    // Motion
    140 TRANSITION "transition" Transition NONE;
    141 SPRING "spring" Enum SPRING;
    // Node specific
    150 SLIDER_MIN "slider-min" F32 NONE;
    151 SLIDER_MAX "slider-max" F32 NONE;
    152 SLIDER_VALUE "slider-value" F32 NONE;
    153 SLIDER_STEP "slider-step" F32 NONE;
    154 PLACEHOLDER "placeholder" Str NONE;
    155 MAX_LENGTH "max-length" I32 NONE;
    156 SECURE "secure" Bool NONE;
    157 READONLY "readonly" Bool NONE;
    158 IMAGE_SOURCE "image-source" Str NONE;
    159 IMAGE_ID "image-id" U32 NONE;
    160 IMAGE_FIT "image-fit" Enum FIT;
    161 SCROLL_OFFSET "scroll-offset" Pair NONE;
    162 TOOLTIP "tooltip" Str NONE;
    163 FOCUSABLE "focusable" Bool NONE;
    164 ANCHOR "anchor" U32 NONE;
    165 ANCHOR_SIDE "anchor-side" Enum SIDE;
    166 ANCHOR_ALIGN "anchor-align" Enum ANCHOR_ALIGN;
    167 MODAL "modal" Bool NONE;
    // Set by LIST_RANGE
    190 LIST_COUNT "list-count" U32 NONE;
    191 LIST_HEIGHT "list-height" F32 NONE;
    192 LIST_OVERSCAN "list-overscan" U32 NONE;
}

/// The definition of property `id`.
pub fn def(id: u32) -> Option<&'static PropDef> {
    let i = TABLE.binary_search_by_key(&id, |d| u32::from(d.id)).ok()?;
    TABLE.get(i)
}

/// Whether `ty` accepts `words` (arity already matches). `Str` words are checked by the caller.
pub fn check_value(d: &PropDef, words: &[u32]) -> Result<(), &'static str> {
    let f = |w: u32| f32::from_bits(w).is_finite();
    match d.ty {
        Ty::F32 => f(words[0])
            .then_some(())
            .ok_or("A number property needs a finite number."),
        Ty::I32 | Ty::U32 | Ty::Str => Ok(()),
        Ty::Bool => (words[0] <= 1)
            .then_some(())
            .ok_or("A flag property is 0 or 1."),
        Ty::Enum => ((words[0].ix()) < d.enums.len())
            .then_some(())
            .ok_or("That is not one of the values of this property."),
        Ty::Token => (words[0] < TOKENS)
            .then_some(())
            .ok_or("A colour is one of the 13 theme tokens."),
        Ty::Dim => {
            if words[0] > DIM_PERCENT || !f(words[1]) {
                Err("A size is auto, pixels or a percentage.")
            } else {
                Ok(())
            }
        }
        Ty::Pair => (f(words[0]) && f(words[1]))
            .then_some(())
            .ok_or("A pair needs finite numbers."),
        Ty::Border => (f(words[0]) && words[0].cast_signed() >= 0 && words[1] < TOKENS)
            .then_some(())
            .ok_or("A border is a width and a theme token."),
        Ty::Gradient => {
            let n = words[1];
            if !f(words[0])
                || !(2..=3).contains(&n)
                || words[2..2 + n.ix()].iter().any(|t| *t >= TOKENS)
            {
                Err("A gradient is an angle and two or three theme tokens.")
            } else {
                Ok(())
            }
        }
        Ty::Shadow => (f(words[0]) && f(words[1]) && f(words[2]) && words[3] < TOKENS)
            .then_some(())
            .ok_or("A shadow is an offset, a blur and a theme token."),
        // The easing is an index into the style's `Ease` (ease, linear, in, out, in-out).
        Ty::Transition => (words[1] <= 2 && words[2].ix() < crate::style::value::Ease::ALL.len())
            .then_some(())
            .ok_or("A transition is a property, fast, normal or slow, and an easing."),
    }
}

/// `abi/wolkit-1.0.props` lines: `id IDENT name type`.
pub fn snapshot_lines() -> Vec<String> {
    TABLE
        .iter()
        .map(|d| format!("{} {} {} {}", d.id, d.ident, d.name, d.ty.name()))
        .collect()
}

/// Whether property `id` makes sense on a node of `kind`. Node-specific properties (150 and up)
/// belong to one kind; the style properties apply to every kind.
pub fn applies(id: u16, kind: super::kind::Kind) -> bool {
    use super::kind::Kind;
    match id {
        id::SLIDER_MIN..=id::SLIDER_STEP => kind == Kind::Slider,
        id::PLACEHOLDER..=id::READONLY => kind == Kind::TextInput,
        id::IMAGE_SOURCE..=id::IMAGE_FIT => kind == Kind::Image,
        id::SCROLL_OFFSET => matches!(kind, Kind::Scroll | Kind::Vlist),
        id::ANCHOR..=id::MODAL => kind == Kind::Overlay,
        id::LIST_COUNT..=id::LIST_OVERSCAN => false, // only LIST_RANGE sets these
        _ => true,
    }
}

/// Extra dirty bits a property sets beyond `STYLE` (the style stage derives the rest by
/// comparing computed styles; these are the properties that bypass it).
pub fn dirty_extra(id: u16) -> u8 {
    use super::kind::dirty;
    match id {
        id::SLIDER_MIN..=id::SLIDER_STEP | id::IMAGE_SOURCE..=id::IMAGE_FIT | id::SCROLL_OFFSET => {
            dirty::LAYOUT | dirty::PAINT
        }
        id::PLACEHOLDER..=id::READONLY => dirty::LAYOUT | dirty::PAINT | dirty::A11Y,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_sorted_unique_and_named() {
        for w in TABLE.windows(2) {
            assert!(w[0].id < w[1].id, "{} {}", w[0].name, w[1].name);
        }
        let mut names: Vec<_> = TABLE.iter().map(|d| d.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), TABLE.len());
        for d in TABLE {
            assert_eq!(d.ty == Ty::Enum, !d.enums.is_empty(), "{}", d.name);
        }
    }

    #[test]
    fn values_are_checked() {
        let d = def(u32::from(id::WIDTH)).unwrap();
        assert!(check_value(d, &[DIM_PX, 12f32.to_bits()]).is_ok());
        assert!(check_value(d, &[3, 0]).is_err());
        assert!(check_value(d, &[DIM_PX, f32::NAN.to_bits()]).is_err());
        let d = def(u32::from(id::BACKGROUND)).unwrap();
        assert!(check_value(d, &[12]).is_ok());
        assert!(check_value(d, &[13]).is_err());
        let d = def(u32::from(id::FLEX_DIRECTION)).unwrap();
        assert!(check_value(d, &[3]).is_ok());
        assert!(check_value(d, &[4]).is_err());
        assert!(def(0).is_none() && def(999).is_none());
    }

    #[test]
    fn every_easing_of_the_style_is_accepted_inline() {
        use crate::style::value::Ease;
        let d = def(u32::from(id::TRANSITION)).unwrap();
        for e in Ease::ALL {
            let w = [u32::from(id::BACKGROUND), 1, u32::from(e.to_u8())];
            assert!(check_value(d, &w).is_ok(), "{}", e.kw());
        }
        assert_eq!(Ease::from_u8(4), Some(Ease::InOut));
        let past = u32::try_from(Ease::ALL.len()).unwrap();
        assert!(check_value(d, &[0, 1, past]).is_err());
        assert!(
            check_value(d, &[0, 3, 0]).is_err(),
            "durations are fast, normal or slow"
        );
    }
}

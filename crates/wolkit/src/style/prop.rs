// SPDX-License-Identifier: BSD-2-Clause
//! The property table: the single source of every supported property. `docs/modules/wolkit-style.md`
//! is generated from it (`style::doc`), the parser, the cascade and the `WSB1` codec are driven by
//! it, and ids are append-only (`tests/style_props.rs` snapshots them). A 128-bit mask marks which
//! properties a rule sets, so applying a rule touches only those.
use super::value::{
    AlignContent, AlignItems, AlignSelf, Backdrop, ColorRef, Cursor, Dim, Direction, Display,
    FontFamily, FontStyle, Justify, Overflow, Paint, PointerEvents, Position, Props, Rd, Shadow,
    Spring, TextAlign, TextDecoration, TextOverflow, Transform, Transition, UserSelect, Val,
    Visibility, WhiteSpace, Wr, Wrap,
};
use super::vparse::{
    Cur, PRes, parse_aspect, parse_border_width, parse_color, parse_feature, parse_font_size,
    parse_inset, parse_kw, parse_letter_spacing, parse_line_height, parse_margin, parse_max_lines,
    parse_nonneg, parse_opacity, parse_outline_width, parse_paint, parse_radius, parse_shadow,
    parse_size, parse_space, parse_transform, parse_transition, parse_weight, parse_zindex,
};
use crate::scene::ids::{Ix, to_u8};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Layout,
    Paint,
    Text,
    Motion,
}

impl Group {
    pub fn name(self) -> &'static str {
        match self {
            Group::Layout => "Layout",
            Group::Paint => "Paint",
            Group::Text => "Text",
            Group::Motion => "Motion",
        }
    }
}

/// What a change to the property invalidates. A paint change never dirties layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Layout,
    Paint,
}

pub struct PropInfo {
    pub id: PropId,
    pub name: &'static str,
    pub group: Group,
    pub inherited: bool,
    pub effect: Effect,
    /// Whether a `transition` can animate it.
    pub anim: bool,
    pub grammar: &'static str,
    pub doc: &'static str,
}

macro_rules! props {
    ($(
        $id:ident $name:literal $grp:ident $inh:literal $eff:ident $anim:literal $ty:ty
            = ($p:ident => $place:expr) $parse:path ; $gram:literal ; $doc:literal;
    )+) => {
        /// Property ids, in declaration order. Append-only: an id is part of the `WSB1` format.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[repr(u8)]
        pub enum PropId { $($id),+ }

        impl PropId {
            pub const ALL: &'static [PropId] = &[$(PropId::$id),+];
            pub const COUNT: usize = PropId::ALL.len();
            pub fn info(self) -> &'static PropInfo { &PROPS[(self as u64).ix()] }
            pub fn name(self) -> &'static str { self.info().name }
            pub fn from_u8(b: u8) -> Option<PropId> { Self::ALL.get(b.ix()).copied() }
            pub fn parse_name(s: &str) -> Option<PropId> { PROPS.iter().find(|p| p.name == s).map(|p| p.id) }
            pub fn bit(self) -> u128 { 1u128 << to_u8(self as u64) }
        }

        pub static PROPS: &[PropInfo] = &[
            $(PropInfo { id: PropId::$id, name: $name, group: Group::$grp, inherited: $inh,
                effect: Effect::$eff, anim: $anim, grammar: $gram, doc: $doc }),+
        ];

        /// Copy one property's value from `src` into `dst`.
        pub fn copy_prop(id: PropId, dst: &mut Props, src: &Props) {
            match id { $(PropId::$id => {
                let v = { let $p = src; $place.clone() };
                let $p = &mut *dst;
                $place = v;
            })+ }
        }

        /// Write one property's value.
        pub fn put_prop(id: PropId, src: &Props, w: &mut Wr) {
            match id { $(PropId::$id => { let $p = src; Val::put(&$place, w) })+ }
        }

        /// Read one property's value into `dst`; `None` when the bytes are not a valid value.
        pub fn get_prop(id: PropId, dst: &mut Props, r: &mut Rd) -> Option<()> {
            match id { $(PropId::$id => {
                let v = <$ty as Val>::get(r)?;
                let $p = &mut *dst;
                $place = v;
                Some(())
            })+ }
        }

        /// Whether one property has the same value in two styles.
        pub fn eq_prop(id: PropId, a: &Props, b: &Props) -> bool {
            match id { $(PropId::$id => { let $p = a; let va = &$place; let $p = b; va == &$place })+ }
        }

        /// A value as text, for goldens and diagnostics.
        pub fn fmt_prop(id: PropId, src: &Props) -> String {
            match id { $(PropId::$id => { let $p = src; format!("{:?}", $place) })+ }
        }

        /// Parse one value from the cursor into `dst`.
        pub fn parse_prop(id: PropId, c: &mut Cur, dst: &mut Props) -> PRes<()> {
            match id { $(PropId::$id => {
                let v: $ty = $parse(c)?;
                let $p = &mut *dst;
                $place = v;
                Ok(())
            })+ }
        }
    };
}

props! {
    // ---- Layout (ids 0..) ----
    Display "display" Layout false Layout false Display = (p => p.layout.display) parse_kw::<Display>;
        "flex | none";
        "`none` removes the node from layout and paint.";
    Direction "flex-direction" Layout false Layout false Direction = (p => p.layout.direction) parse_kw::<Direction>;
        "row | column | row-reverse | column-reverse";
        "Main axis of the node's children.";
    FlexWrap "flex-wrap" Layout false Layout false Wrap = (p => p.layout.wrap) parse_kw::<Wrap>;
        "nowrap | wrap";
        "Whether children break onto more lines.";
    Grow "grow" Layout false Layout false f32 = (p => p.layout.grow) parse_nonneg;
        "<number>";
        "Share of the free main-axis space.";
    Shrink "shrink" Layout false Layout false f32 = (p => p.layout.shrink) parse_nonneg;
        "<number>";
        "Share of the overflow taken back; ignored in `scroll` and `auto` containers.";
    Basis "basis" Layout false Layout false Dim = (p => p.layout.basis) parse_size;
        "auto | <length> | <percent>";
        "Starting main size before grow and shrink.";
    RowGap "row-gap" Layout false Layout false f32 = (p => p.layout.row_gap) parse_space;
        "<space>";
        "Space between lines (and between children of a column).";
    ColumnGap "column-gap" Layout false Layout false f32 = (p => p.layout.col_gap) parse_space;
        "<space>";
        "Space between children of a row.";
    AlignItems "align-items" Layout false Layout false AlignItems = (p => p.layout.align_items) parse_kw::<AlignItems>;
        "stretch | start | center | end | baseline";
        "Cross-axis placement of children; `baseline` behaves as `start` in 1.0.";
    AlignSelf "align-self" Layout false Layout false AlignSelf = (p => p.layout.align_self) parse_kw::<AlignSelf>;
        "auto | stretch | start | center | end | baseline";
        "Overrides the parent's `align-items` for this node.";
    AlignContent "align-content" Layout false Layout false AlignContent = (p => p.layout.align_content) parse_kw::<AlignContent>;
        "stretch | start | center | end | space-between | space-around";
        "Placement of lines when wrapping.";
    Justify "justify-content" Layout false Layout false Justify = (p => p.layout.justify) parse_kw::<Justify>;
        "start | center | end | space-between | space-around | space-evenly";
        "Main-axis placement of children.";
    PaddingTop "padding-top" Layout false Layout false f32 = (p => p.layout.padding[0]) parse_space;
        "<space>";
        "Inner space, top.";
    PaddingRight "padding-right" Layout false Layout false f32 = (p => p.layout.padding[1]) parse_space;
        "<space>";
        "Inner space, right.";
    PaddingBottom "padding-bottom" Layout false Layout false f32 = (p => p.layout.padding[2]) parse_space;
        "<space>";
        "Inner space, bottom.";
    PaddingLeft "padding-left" Layout false Layout false f32 = (p => p.layout.padding[3]) parse_space;
        "<space>";
        "Inner space, left.";
    MarginTop "margin-top" Layout false Layout false Dim = (p => p.layout.margin[0]) parse_margin;
        "auto | <space>";
        "Outer space, top.";
    MarginRight "margin-right" Layout false Layout false Dim = (p => p.layout.margin[1]) parse_margin;
        "auto | <space>";
        "Outer space, right.";
    MarginBottom "margin-bottom" Layout false Layout false Dim = (p => p.layout.margin[2]) parse_margin;
        "auto | <space>";
        "Outer space, bottom.";
    MarginLeft "margin-left" Layout false Layout false Dim = (p => p.layout.margin[3]) parse_margin;
        "auto | <space>";
        "Outer space, left.";
    Width "width" Layout false Layout false Dim = (p => p.layout.width) parse_size;
        "auto | <length> | <percent>";
        "Border-box width.";
    Height "height" Layout false Layout false Dim = (p => p.layout.height) parse_size;
        "auto | <length> | <percent>";
        "Border-box height.";
    MinWidth "min-width" Layout false Layout false Dim = (p => p.layout.min_width) parse_size;
        "auto | <length> | <percent>";
        "Lower bound of the width; `auto` is 0.";
    MinHeight "min-height" Layout false Layout false Dim = (p => p.layout.min_height) parse_size;
        "auto | <length> | <percent>";
        "Lower bound of the height; `auto` is 0.";
    MaxWidth "max-width" Layout false Layout false Dim = (p => p.layout.max_width) parse_size;
        "none | <length> | <percent>";
        "Upper bound of the width.";
    MaxHeight "max-height" Layout false Layout false Dim = (p => p.layout.max_height) parse_size;
        "none | <length> | <percent>";
        "Upper bound of the height.";
    AspectRatio "aspect-ratio" Layout false Layout false f32 = (p => p.layout.aspect) parse_aspect;
        "auto | <number> | <number> / <number>";
        "Width over height, used when one side is open.";
    Position "position" Layout false Layout false Position = (p => p.layout.position) parse_kw::<Position>;
        "relative | absolute";
        "`absolute` leaves the flow and is placed by its insets in the parent's box.";
    InsetTop "top" Layout false Layout false Dim = (p => p.layout.inset[0]) parse_inset;
        "auto | <length> | <percent>";
        "Offset from the top edge of an absolute node.";
    InsetRight "right" Layout false Layout false Dim = (p => p.layout.inset[1]) parse_inset;
        "auto | <length> | <percent>";
        "Offset from the right edge of an absolute node.";
    InsetBottom "bottom" Layout false Layout false Dim = (p => p.layout.inset[2]) parse_inset;
        "auto | <length> | <percent>";
        "Offset from the bottom edge of an absolute node.";
    InsetLeft "left" Layout false Layout false Dim = (p => p.layout.inset[3]) parse_inset;
        "auto | <length> | <percent>";
        "Offset from the left edge of an absolute node.";
    Overflow "overflow" Layout false Layout false Overflow = (p => p.layout.overflow) parse_kw::<Overflow>;
        "visible | hidden | scroll | auto";
        "Clipping and scrolling of children.";
    ZIndex "z-index" Layout false Paint false i16 = (p => p.layout.z_index) parse_zindex;
        "<integer>";
        "Paint order among siblings.";
    BorderWidth "border-width" Paint false Layout false f32 = (p => p.layout.border_width) parse_border_width;
        "<pixels>";
        "Border width; counts as padding for layout.";
    // ---- Paint ----
    Background "background" Paint false Paint true Paint = (p => p.paint.background) parse_paint;
        "none | <color> | linear-gradient(<angle>, <color>, <color> [, <color>])";
        "Fill; a gradient has two or three evenly spaced stops.";
    BorderColor "border-color" Paint false Paint true ColorRef = (p => p.paint.border_color) parse_color;
        "<color>";
        "Border colour.";
    RadiusTl "border-top-left-radius" Paint false Paint true f32 = (p => p.paint.radius[0]) parse_radius;
        "<radius>";
        "Corner radius, top left.";
    RadiusTr "border-top-right-radius" Paint false Paint true f32 = (p => p.paint.radius[1]) parse_radius;
        "<radius>";
        "Corner radius, top right.";
    RadiusBr "border-bottom-right-radius" Paint false Paint true f32 = (p => p.paint.radius[2]) parse_radius;
        "<radius>";
        "Corner radius, bottom right.";
    RadiusBl "border-bottom-left-radius" Paint false Paint true f32 = (p => p.paint.radius[3]) parse_radius;
        "<radius>";
        "Corner radius, bottom left.";
    BoxShadow "box-shadow" Paint false Paint true [Shadow; 2] = (p => p.paint.shadow) parse_shadow;
        "none | <shadow> [, <shadow>]";
        "Elevation: `sm`, `md` or `lg`; two layers at most (`lg` is two).";
    Opacity "opacity" Paint false Paint true f32 = (p => p.paint.opacity) parse_opacity;
        "<number> | <percent>";
        "Group opacity.";
    OutlineWidth "outline-width" Paint false Paint true f32 = (p => p.paint.outline_width) parse_outline_width;
        "<pixels>";
        "Focus ring width.";
    OutlineColor "outline-color" Paint false Paint true ColorRef = (p => p.paint.outline_color) parse_color;
        "<color>";
        "Focus ring colour.";
    Transform "transform" Paint false Paint true Transform = (p => p.paint.transform) parse_transform;
        "none | translate(<px>[, <px>]) scale(<n>[, <n>]) rotate(<deg>)";
        "Paint-only transform; layout is unchanged.";
    Cursor "cursor" Paint true Paint false Cursor = (p => p.paint.cursor) parse_kw::<Cursor>;
        "default | pointer | text | grab | grabbing | not-allowed | progress";
        "Pointer shape.";
    PointerEvents "pointer-events" Paint false Paint false PointerEvents = (p => p.paint.pointer_events) parse_kw::<PointerEvents>;
        "auto | none";
        "`none` makes the node transparent to the pointer.";
    Visibility "visibility" Paint true Paint false Visibility = (p => p.paint.visibility) parse_kw::<Visibility>;
        "visible | hidden";
        "`hidden` keeps the space but paints nothing.";
    Backdrop "backdrop" Paint false Paint false Backdrop = (p => p.paint.backdrop) parse_kw::<Backdrop>;
        "none | glass";
        "A request to the compositor to blur what is behind the node.";
    // ---- Text ----
    Color "color" Text true Paint true ColorRef = (p => p.text.color) parse_color;
        "<color>";
        "Text colour.";
    FontFamily "font-family" Text true Layout false FontFamily = (p => p.text.family) parse_kw::<FontFamily>;
        "ui | mono";
        "The interface face or the fixed-width face.";
    FontSize "font-size" Text true Layout false f32 = (p => p.text.size) parse_font_size;
        "<role> | <pixels, at least 11>";
        "A type role (display, large, title, body, secondary, caption) or a size from 11 px.";
    FontWeight "font-weight" Text true Layout false u16 = (p => p.text.weight) parse_weight;
        "400 | 500 | 600 | 700 | normal | bold";
        "Weight.";
    FontStyle "font-style" Text true Layout false FontStyle = (p => p.text.style) parse_kw::<FontStyle>;
        "normal | italic";
        "Slant.";
    LineHeight "line-height" Text true Layout false f32 = (p => p.text.line_height) parse_line_height;
        "normal | <number>";
        "A multiple of the font size.";
    LetterSpacing "letter-spacing" Text true Layout false f32 = (p => p.text.letter_spacing) parse_letter_spacing;
        "normal | <pixels>";
        "Extra space between letters, -2 to 8 px.";
    TextAlign "text-align" Text true Layout false TextAlign = (p => p.text.align) parse_kw::<TextAlign>;
        "start | center | end | justify";
        "Alignment of lines in the box.";
    TextDecoration "text-decoration" Text false Paint false TextDecoration = (p => p.text.decoration) parse_kw::<TextDecoration>;
        "none | underline | strikethrough";
        "Line drawn with the text.";
    TextOverflow "text-overflow" Text false Layout false TextOverflow = (p => p.text.overflow) parse_kw::<TextOverflow>;
        "clip | ellipsis";
        "What shows when a line is cut.";
    WhiteSpace "white-space" Text true Layout false WhiteSpace = (p => p.text.white_space) parse_kw::<WhiteSpace>;
        "normal | nowrap | pre-wrap";
        "Wrapping and spaces.";
    MaxLines "max-lines" Text false Layout false u16 = (p => p.text.max_lines) parse_max_lines;
        "none | <integer>";
        "Lines shown before the text is cut.";
    FontFeature "font-feature" Text true Layout false u8 = (p => p.text.feature) parse_feature;
        "none | tnum";
        "`tnum` turns on tabular numbers.";
    UserSelect "user-select" Text true Paint false UserSelect = (p => p.text.select) parse_kw::<UserSelect>;
        "auto | none | text";
        "Whether the text can be selected.";
    // ---- Motion ----
    Transition "transition" Motion false Paint false Transition = (p => p.motion.transition) parse_transition;
        "<property>[, <property>] [fast | normal | slow | <ms>] [ease | linear | in | out | in-out]";
        "Animate these properties when the node enters this style; at most 500 ms.";
    Spring "spring" Motion false Paint false Spring = (p => p.motion.spring) parse_kw::<Spring>;
        "none | gentle | snappy | bouncy";
        "Move numeric properties with a spring instead of a curve.";
}

/// Longhands a shorthand sets, and how many values it takes (1 to `max`, CSS-style repetition).
pub struct Shorthand {
    pub name: &'static str,
    pub sets: &'static [PropId],
    pub grammar: &'static str,
    pub doc: &'static str,
}

pub static SHORTHANDS: &[Shorthand] = &[
    Shorthand {
        name: "padding",
        sets: &[
            PropId::PaddingTop,
            PropId::PaddingRight,
            PropId::PaddingBottom,
            PropId::PaddingLeft,
        ],
        grammar: "<space>{1,4}",
        doc: "One value for all sides, two for vertical and horizontal, four for top, right, bottom, left.",
    },
    Shorthand {
        name: "margin",
        sets: &[
            PropId::MarginTop,
            PropId::MarginRight,
            PropId::MarginBottom,
            PropId::MarginLeft,
        ],
        grammar: "(auto | <space>){1,4}",
        doc: "Same forms as `padding`.",
    },
    Shorthand {
        name: "gap",
        sets: &[PropId::RowGap, PropId::ColumnGap],
        grammar: "<space>{1,2}",
        doc: "One value for both, or row then column.",
    },
    Shorthand {
        name: "inset",
        sets: &[
            PropId::InsetTop,
            PropId::InsetRight,
            PropId::InsetBottom,
            PropId::InsetLeft,
        ],
        grammar: "(auto | <length>){1,4}",
        doc: "Same forms as `padding`.",
    },
    Shorthand {
        name: "border-radius",
        sets: &[
            PropId::RadiusTl,
            PropId::RadiusTr,
            PropId::RadiusBr,
            PropId::RadiusBl,
        ],
        grammar: "<radius>{1,4}",
        doc: "Corners in the order top left, top right, bottom right, bottom left; fewer values repeat as in CSS.",
    },
    Shorthand {
        name: "border",
        sets: &[PropId::BorderWidth, PropId::BorderColor],
        grammar: "<pixels> [<color>]",
        doc: "Width and colour of a solid border.",
    },
    Shorthand {
        name: "outline",
        sets: &[PropId::OutlineWidth, PropId::OutlineColor],
        grammar: "<pixels> [<color>]",
        doc: "Width and colour of the focus ring.",
    },
];

/// Properties copied from the parent when a rule does not set them.
pub fn inherited_ids() -> impl Iterator<Item = PropId> {
    PropId::ALL.iter().copied().filter(|p| p.info().inherited)
}

/// Properties set by a rule: a mask and the values (only the masked ones mean anything).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Decls {
    pub mask: u128,
    pub p: Props,
}

impl Decls {
    pub fn is_empty(&self) -> bool {
        self.mask == 0
    }
    /// Later declarations win: copy `other`'s set properties over ours.
    pub fn merge(&mut self, other: &Decls) {
        apply_mask(&mut self.p, &other.p, other.mask);
        self.mask |= other.mask;
    }
    pub fn set_from(&mut self, id: PropId, src: &Props) {
        copy_prop(id, &mut self.p, src);
        self.mask |= id.bit();
    }
}

/// Copy every property in `mask` from `src` to `dst`.
pub fn apply_mask(dst: &mut Props, src: &Props, mut mask: u128) {
    while mask != 0 {
        let b = to_u8(mask.trailing_zeros());
        mask &= mask - 1;
        if let Some(id) = PropId::from_u8(b) {
            copy_prop(id, dst, src);
        }
    }
}

/// Mask of the properties whose change needs a new layout.
pub fn layout_mask() -> u128 {
    PropId::ALL
        .iter()
        .filter(|p| p.info().effect == Effect::Layout)
        .fold(0, |m, p| m | p.bit())
}

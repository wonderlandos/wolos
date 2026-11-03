// SPDX-License-Identifier: BSD-2-Clause
//! Value types of the style system: keyword enums, lengths, colour references, paints and the
//! property groups (`LayoutStyle`, `PaintStyle`, `TextStyle`, `MotionStyle`) that make up a
//! computed style. Plain data, no allocation, comparable, with a stable byte codec (`WSB1`).
use super::tokens::Token;

/// Declares a keyword enum: variants map to CSS keywords, to a stable `u8` (declaration order,
/// append-only) and back. The first variant is the default.
macro_rules! kw_enum {
    ($(#[$m:meta])* pub enum $name:ident { $($var:ident = $kw:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[repr(u8)]
        pub enum $name { $($var),+ }
        impl Default for $name { fn default() -> Self { Self::ALL[0] } }
        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$var),+];
            pub const KEYWORDS: &'static [&'static str] = &[$($kw),+];
            pub fn kw(self) -> &'static str { match self { $($name::$var => $kw),+ } }
            pub fn parse(s: &str) -> Option<Self> { match s { $($kw => Some($name::$var),)+ _ => None } }
            pub fn from_u8(b: u8) -> Option<Self> { Self::ALL.get(b as usize).copied() }
            pub fn to_u8(self) -> u8 { self as u8 }
        }
    };
}

kw_enum! { pub enum Display { Flex = "flex", None = "none" } }
kw_enum! { pub enum Direction { Row = "row", Column = "column", RowReverse = "row-reverse", ColumnReverse = "column-reverse" } }
kw_enum! { pub enum Wrap { NoWrap = "nowrap", Wrap = "wrap" } }
kw_enum! { pub enum AlignItems { Stretch = "stretch", Start = "start", Center = "center", End = "end", Baseline = "baseline" } }
kw_enum! { pub enum AlignSelf { Auto = "auto", Stretch = "stretch", Start = "start", Center = "center", End = "end", Baseline = "baseline" } }
kw_enum! { pub enum AlignContent {
Stretch = "stretch", Start = "start", Center = "center", End = "end",
SpaceBetween = "space-between", SpaceAround = "space-around" } }
kw_enum! { pub enum Justify {
Start = "start", Center = "center", End = "end",
SpaceBetween = "space-between", SpaceAround = "space-around", SpaceEvenly = "space-evenly" } }
kw_enum! { pub enum Position { Relative = "relative", Absolute = "absolute" } }
kw_enum! { pub enum Overflow { Visible = "visible", Hidden = "hidden", Scroll = "scroll", Auto = "auto" } }

kw_enum! { pub enum Cursor {
Default = "default", Pointer = "pointer", Text = "text", Grab = "grab", Grabbing = "grabbing",
NotAllowed = "not-allowed", Progress = "progress" } }
kw_enum! { pub enum PointerEvents { Auto = "auto", None = "none" } }
kw_enum! { pub enum Visibility { Visible = "visible", Hidden = "hidden" } }
kw_enum! { pub enum Backdrop { None = "none", Glass = "glass" } }

kw_enum! { pub enum FontFamily { Ui = "ui", Mono = "mono" } }
kw_enum! { pub enum FontStyle { Normal = "normal", Italic = "italic" } }
kw_enum! { pub enum TextAlign { Start = "start", Center = "center", End = "end", Justify = "justify" } }
kw_enum! { pub enum TextDecoration { None = "none", Underline = "underline", Strikethrough = "strikethrough" } }
kw_enum! { pub enum TextOverflow { Clip = "clip", Ellipsis = "ellipsis" } }
kw_enum! { pub enum WhiteSpace { Normal = "normal", NoWrap = "nowrap", PreWrap = "pre-wrap" } }
kw_enum! { pub enum UserSelect { Auto = "auto", None = "none", Text = "text" } }

kw_enum! { pub enum Ease { Standard = "ease", Linear = "linear", In = "in", Out = "out", InOut = "in-out" } }
kw_enum! { pub enum Spring { None = "none", Gentle = "gentle", Snappy = "snappy", Bouncy = "bouncy" } }

/// `font-feature` bit set.
pub const FEATURE_TNUM: u8 = 1;

/// A length: `auto`, pixels or a percentage of the available size.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Dim {
    #[default]
    Auto,
    Px(f32),
    Pct(f32),
}

impl Dim {
    /// The pixel value if definite against `base` (a percentage needs a definite base).
    pub fn resolve(self, base: Option<f32>) -> Option<f32> {
        match self {
            Dim::Auto => None,
            Dim::Px(v) => Some(v),
            Dim::Pct(p) => base.map(|b| b * p / 100.0),
        }
    }
    pub fn is_auto(self) -> bool {
        matches!(self, Dim::Auto)
    }
}

/// A colour is always a reference: a theme token, a token with an alpha percentage, or clear.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColorRef {
    Token(Token),
    /// Token at this alpha, 0 to 100 percent.
    Alpha(Token, u8),
    /// Black at this alpha, 0 to 100 percent: the fixed material of shadows, not themable. Only
    /// the shadow scale produces it; a sheet cannot write a raw colour.
    Shade(u8),
    Transparent,
}

impl ColorRef {
    pub fn render(self) -> String {
        match self {
            ColorRef::Token(t) => t.name().to_string(),
            ColorRef::Alpha(t, a) => format!("alpha({}, {}%)", t.name(), a),
            ColorRef::Shade(a) => format!("shade({a}%)"),
            ColorRef::Transparent => "transparent".to_string(),
        }
    }
}

/// One shadow layer: offset, blur radius and colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub dx: f32,
    pub dy: f32,
    pub blur: f32,
    pub color: ColorRef,
}

impl Shadow {
    pub const NONE: Shadow = Shadow {
        dx: 0.0,
        dy: 0.0,
        blur: 0.0,
        color: ColorRef::Transparent,
    };
}

/// `background`: nothing, a colour, or an evenly spaced 2 or 3 stop linear gradient.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Paint {
    #[default]
    None,
    Color(ColorRef),
    Linear {
        angle: i16,
        stops: [ColorRef; 3],
        n: u8,
    },
}

/// Paint-only transform. Identity is `{0, 0, 1, 1, 0}`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub tx: f32,
    pub ty: f32,
    pub sx: f32,
    pub sy: f32,
    /// Degrees, clockwise.
    pub rotate: f32,
}

impl Default for Transform {
    fn default() -> Self {
        Transform {
            tx: 0.0,
            ty: 0.0,
            sx: 1.0,
            sy: 1.0,
            rotate: 0.0,
        }
    }
}

/// `transition`: which properties animate, for how long, with which curve. Applies to the style
/// being entered. Duration is at most 500 ms (RU-060).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Transition {
    /// Bit per `PropId`.
    pub props: u128,
    pub ms: u16,
    pub ease: Ease,
}

/// Longest transition the compiler accepts (RU-060).
pub const MAX_TRANSITION_MS: u16 = 500;

#[derive(Clone, Debug, PartialEq)]
pub struct LayoutStyle {
    pub display: Display,
    pub direction: Direction,
    pub wrap: Wrap,
    pub grow: f32,
    pub shrink: f32,
    pub basis: Dim,
    pub row_gap: f32,
    pub col_gap: f32,
    pub align_items: AlignItems,
    pub align_self: AlignSelf,
    pub align_content: AlignContent,
    pub justify: Justify,
    /// Top, right, bottom, left.
    pub padding: [f32; 4],
    pub margin: [Dim; 4],
    pub width: Dim,
    pub height: Dim,
    pub min_width: Dim,
    pub min_height: Dim,
    pub max_width: Dim,
    pub max_height: Dim,
    /// Width over height; 0 means none.
    pub aspect: f32,
    pub position: Position,
    pub inset: [Dim; 4],
    pub overflow: Overflow,
    pub z_index: i16,
    /// Uniform border width (counts as padding for layout).
    pub border_width: f32,
}

impl Default for LayoutStyle {
    fn default() -> Self {
        LayoutStyle {
            display: Display::Flex,
            direction: Direction::Row,
            wrap: Wrap::NoWrap,
            grow: 0.0,
            shrink: 1.0,
            basis: Dim::Auto,
            row_gap: 0.0,
            col_gap: 0.0,
            align_items: AlignItems::Stretch,
            align_self: AlignSelf::Auto,
            align_content: AlignContent::Stretch,
            justify: Justify::Start,
            padding: [0.0; 4],
            margin: [Dim::Px(0.0); 4],
            width: Dim::Auto,
            height: Dim::Auto,
            min_width: Dim::Auto,
            min_height: Dim::Auto,
            max_width: Dim::Auto,
            max_height: Dim::Auto,
            aspect: 0.0,
            position: Position::Relative,
            inset: [Dim::Auto; 4],
            overflow: Overflow::Visible,
            z_index: 0,
            border_width: 0.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PaintStyle {
    pub background: Paint,
    pub border_color: ColorRef,
    /// Top-left, top-right, bottom-right, bottom-left.
    pub radius: [f32; 4],
    pub shadow: [Shadow; 2],
    pub opacity: f32,
    pub outline_width: f32,
    pub outline_color: ColorRef,
    pub transform: Transform,
    pub cursor: Cursor,
    pub pointer_events: PointerEvents,
    pub visibility: Visibility,
    pub backdrop: Backdrop,
}

impl Default for PaintStyle {
    fn default() -> Self {
        PaintStyle {
            background: Paint::None,
            border_color: ColorRef::Token(Token::Line),
            radius: [0.0; 4],
            shadow: [Shadow::NONE; 2],
            opacity: 1.0,
            outline_width: 0.0,
            outline_color: ColorRef::Token(Token::Accent),
            transform: Transform::default(),
            cursor: Cursor::Default,
            pointer_events: PointerEvents::Auto,
            visibility: Visibility::Visible,
            backdrop: Backdrop::None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    pub color: ColorRef,
    pub family: FontFamily,
    pub size: f32,
    pub weight: u16,
    pub style: FontStyle,
    /// Multiple of the font size; 0 means the font's natural line height.
    pub line_height: f32,
    pub letter_spacing: f32,
    pub align: TextAlign,
    pub decoration: TextDecoration,
    pub overflow: TextOverflow,
    pub white_space: WhiteSpace,
    /// 0 means unlimited.
    pub max_lines: u16,
    pub feature: u8,
    pub select: UserSelect,
}

impl Default for TextStyle {
    fn default() -> Self {
        TextStyle {
            color: ColorRef::Token(Token::Text),
            family: FontFamily::Ui,
            size: 13.0,
            weight: 400,
            style: FontStyle::Normal,
            line_height: 0.0,
            letter_spacing: 0.0,
            align: TextAlign::Start,
            decoration: TextDecoration::None,
            overflow: TextOverflow::Clip,
            white_space: WhiteSpace::Normal,
            max_lines: 0,
            feature: 0,
            select: UserSelect::Auto,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct MotionStyle {
    pub transition: Transition,
    pub spring: Spring,
}

/// One node's complete style after the cascade. Interned: equal styles share one table entry.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Props {
    pub layout: LayoutStyle,
    pub paint: PaintStyle,
    pub text: TextStyle,
    pub motion: MotionStyle,
}

/// Little-endian writer for the `WSB1` codec.
#[derive(Default)]
pub struct Wr(pub Vec<u8>);

impl Wr {
    pub fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    pub fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn i16(&mut self, v: i16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u128(&mut self, v: u128) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    pub fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
}

/// Bounded little-endian reader; every read reports truncation as `None`.
pub struct Rd<'a> {
    pub b: &'a [u8],
    pub at: usize,
}

impl<'a> Rd<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        Rd { b, at: 0 }
    }
    pub fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(n)?;
        let s = self.b.get(self.at..end)?;
        self.at = end;
        Some(s)
    }
    pub fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|s| s[0])
    }
    pub fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|s| u16::from_le_bytes([s[0], s[1]]))
    }
    pub fn i16(&mut self) -> Option<i16> {
        self.take(2).map(|s| i16::from_le_bytes([s[0], s[1]]))
    }
    pub fn u32(&mut self) -> Option<u32> {
        self.take(4)
            .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    pub fn u128(&mut self) -> Option<u128> {
        self.take(16).map(|s| {
            let mut a = [0u8; 16];
            a.copy_from_slice(s);
            u128::from_le_bytes(a)
        })
    }
    /// A finite float only: NaN and infinities are rejected so equal tables compare equal.
    pub fn f32(&mut self) -> Option<f32> {
        let v = self
            .take(4)
            .map(|s| f32::from_le_bytes([s[0], s[1], s[2], s[3]]))?;
        v.is_finite().then_some(v)
    }
    pub fn left(&self) -> usize {
        self.b.len() - self.at
    }
}

/// Binary codec of one property value.
pub trait Val: Sized {
    fn put(&self, w: &mut Wr);
    fn get(r: &mut Rd) -> Option<Self>;
}

impl Val for f32 {
    fn put(&self, w: &mut Wr) {
        w.f32(*self)
    }
    fn get(r: &mut Rd) -> Option<Self> {
        r.f32()
    }
}
impl Val for u16 {
    fn put(&self, w: &mut Wr) {
        w.u16(*self)
    }
    fn get(r: &mut Rd) -> Option<Self> {
        r.u16()
    }
}
impl Val for i16 {
    fn put(&self, w: &mut Wr) {
        w.i16(*self)
    }
    fn get(r: &mut Rd) -> Option<Self> {
        r.i16()
    }
}
impl Val for u8 {
    fn put(&self, w: &mut Wr) {
        w.u8(*self)
    }
    fn get(r: &mut Rd) -> Option<Self> {
        r.u8()
    }
}

macro_rules! val_enum {
    ($($t:ty),+) => {$(
        impl Val for $t {
            fn put(&self, w: &mut Wr) { w.u8(self.to_u8()) }
            fn get(r: &mut Rd) -> Option<Self> { Self::from_u8(r.u8()?) }
        }
    )+};
}
val_enum!(
    Display,
    Direction,
    Wrap,
    AlignItems,
    AlignSelf,
    AlignContent,
    Justify,
    Position,
    Overflow,
    Cursor,
    PointerEvents,
    Visibility,
    Backdrop,
    FontFamily,
    FontStyle,
    TextAlign,
    TextDecoration,
    TextOverflow,
    WhiteSpace,
    UserSelect,
    Ease,
    Spring
);

impl Val for Dim {
    fn put(&self, w: &mut Wr) {
        match *self {
            Dim::Auto => w.u8(0),
            Dim::Px(v) => {
                w.u8(1);
                w.f32(v)
            }
            Dim::Pct(v) => {
                w.u8(2);
                w.f32(v)
            }
        }
    }
    fn get(r: &mut Rd) -> Option<Self> {
        Some(match r.u8()? {
            0 => Dim::Auto,
            1 => Dim::Px(r.f32()?),
            2 => Dim::Pct(r.f32()?),
            _ => return None,
        })
    }
}

impl Val for ColorRef {
    fn put(&self, w: &mut Wr) {
        match *self {
            ColorRef::Token(t) => {
                w.u8(0);
                w.u8(t.to_u8());
            }
            ColorRef::Alpha(t, a) => {
                w.u8(1);
                w.u8(t.to_u8());
                w.u8(a);
            }
            ColorRef::Transparent => w.u8(2),
            ColorRef::Shade(a) => {
                w.u8(3);
                w.u8(a);
            }
        }
    }
    fn get(r: &mut Rd) -> Option<Self> {
        Some(match r.u8()? {
            0 => ColorRef::Token(Token::from_u8(r.u8()?)?),
            1 => {
                let t = Token::from_u8(r.u8()?)?;
                let a = r.u8()?;
                if a > 100 {
                    return None;
                }
                ColorRef::Alpha(t, a)
            }
            2 => ColorRef::Transparent,
            3 => {
                let a = r.u8()?;
                if a > 100 {
                    return None;
                }
                ColorRef::Shade(a)
            }
            _ => return None,
        })
    }
}

impl Val for Paint {
    fn put(&self, w: &mut Wr) {
        match *self {
            Paint::None => w.u8(0),
            Paint::Color(c) => {
                w.u8(1);
                c.put(w);
            }
            Paint::Linear { angle, stops, n } => {
                w.u8(2);
                w.i16(angle);
                w.u8(n);
                for s in stops.iter().take(n as usize) {
                    s.put(w);
                }
            }
        }
    }
    fn get(r: &mut Rd) -> Option<Self> {
        Some(match r.u8()? {
            0 => Paint::None,
            1 => Paint::Color(ColorRef::get(r)?),
            2 => {
                let angle = r.i16()?;
                let n = r.u8()?;
                if !(2..=3).contains(&n) {
                    return None;
                }
                let mut stops = [ColorRef::Transparent; 3];
                for s in stops.iter_mut().take(n as usize) {
                    *s = ColorRef::get(r)?;
                }
                Paint::Linear { angle, stops, n }
            }
            _ => return None,
        })
    }
}

impl Val for Transform {
    fn put(&self, w: &mut Wr) {
        w.f32(self.tx);
        w.f32(self.ty);
        w.f32(self.sx);
        w.f32(self.sy);
        w.f32(self.rotate);
    }
    fn get(r: &mut Rd) -> Option<Self> {
        Some(Transform {
            tx: r.f32()?,
            ty: r.f32()?,
            sx: r.f32()?,
            sy: r.f32()?,
            rotate: r.f32()?,
        })
    }
}

impl Val for Shadow {
    fn put(&self, w: &mut Wr) {
        w.f32(self.dx);
        w.f32(self.dy);
        w.f32(self.blur);
        self.color.put(w);
    }
    fn get(r: &mut Rd) -> Option<Self> {
        Some(Shadow {
            dx: r.f32()?,
            dy: r.f32()?,
            blur: r.f32()?,
            color: ColorRef::get(r)?,
        })
    }
}

impl Val for [Shadow; 2] {
    fn put(&self, w: &mut Wr) {
        self[0].put(w);
        self[1].put(w);
    }
    fn get(r: &mut Rd) -> Option<Self> {
        Some([Shadow::get(r)?, Shadow::get(r)?])
    }
}

impl Val for Transition {
    fn put(&self, w: &mut Wr) {
        w.u128(self.props);
        w.u16(self.ms);
        w.u8(self.ease.to_u8());
    }
    fn get(r: &mut Rd) -> Option<Self> {
        let props = r.u128()?;
        let ms = r.u16()?;
        if ms > MAX_TRANSITION_MS {
            return None;
        }
        Some(Transition {
            props,
            ms,
            ease: Ease::from_u8(r.u8()?)?,
        })
    }
}

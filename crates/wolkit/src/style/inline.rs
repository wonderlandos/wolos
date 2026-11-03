// SPDX-License-Identifier: BSD-2-Clause
//! Inline properties: the wire form of tier 1 (`SET_PROP`, ids in `scene::prop`) turned into
//! declarations of the same kind a `.wolcss` rule makes, so both tiers meet in one cascade. The
//! mapping goes by property, never by position, and a value that the wire table accepted is
//! always representable. Node-specific properties (slider, text input, image, anchor, list) are
//! not style and are ignored here.
use super::prop::*;
use super::tokens::{self as tk, Token};
use super::value::*;
use super::vparse::{Cur, tokenize};
use crate::scene::PropValue;
use crate::scene::prop::{self as wire, id};

fn f(v: &PropValue) -> f32 {
    v.f32()
}

fn dim(v: &PropValue) -> Dim {
    let x = f32::from_bits(v.w[1]);
    match v.w[0] {
        wire::DIM_PX => Dim::Px(x),
        wire::DIM_PERCENT => Dim::Pct(x),
        _ => Dim::Auto,
    }
}

fn token(w: u32) -> ColorRef {
    Token::from_u8(w as u8).map_or(ColorRef::Token(Token::Text), ColorRef::Token)
}

fn set(out: &mut Decls, p: PropId, f: impl FnOnce(&mut Props)) {
    f(&mut out.p);
    out.mask |= p.bit();
}

/// Parse a keyword of an enum property with the sheet parser, so inline and sheet agree.
fn keyword(out: &mut Decls, def: &wire::PropDef, v: &PropValue) {
    let Some(name) = def.enums.get(v.w[0] as usize) else {
        return;
    };
    let Some(p) = PropId::parse_name(def.name) else {
        return;
    };
    let toks = tokenize(name, 0, name.len());
    let mut c = Cur::new(toks, p.name(), name.len(), true);
    let mut tmp = Props::default();
    if parse_prop(p, &mut c, &mut tmp).is_ok() {
        out.set_from(p, &tmp);
    }
}

/// Wire property id for a transition: which of our properties it animates (0 means every
/// animatable one).
fn transition_mask(wire_id: u32) -> u128 {
    use PropId as P;
    let ids: &[PropId] = match wire_id as u16 {
        0 => {
            return PropId::ALL
                .iter()
                .filter(|p| p.info().anim)
                .fold(0, |m, p| m | p.bit());
        }
        id::BACKGROUND | id::BACKGROUND_GRADIENT => &[P::Background],
        id::BORDER => &[P::BorderColor],
        id::BORDER_RADIUS | id::RADIUS_TL | id::RADIUS_TR | id::RADIUS_BR | id::RADIUS_BL => {
            &[P::RadiusTl, P::RadiusTr, P::RadiusBr, P::RadiusBl]
        }
        id::SHADOW_1 | id::SHADOW_2 => &[P::BoxShadow],
        id::OPACITY => &[P::Opacity],
        id::OUTLINE => &[P::OutlineColor],
        id::TRANSLATE | id::SCALE | id::ROTATE => &[P::Transform],
        id::COLOR => &[P::Color],
        _ => &[],
    };
    ids.iter().fold(0, |m, p| m | p.bit())
}

/// Add the declaration for wire property `prop` with `v` to `out`; later calls win.
pub fn decl_from_wire(prop: u16, v: &PropValue, out: &mut Decls) {
    use PropId as P;
    let Some(def) = wire::def(u32::from(prop)) else {
        return;
    };
    match prop {
        id::DISPLAY
        | id::FLEX_DIRECTION
        | id::FLEX_WRAP
        | id::ALIGN_ITEMS
        | id::ALIGN_SELF
        | id::ALIGN_CONTENT
        | id::JUSTIFY_CONTENT
        | id::POSITION
        | id::OVERFLOW
        | id::CURSOR
        | id::POINTER_EVENTS
        | id::VISIBILITY
        | id::BACKDROP
        | id::FONT_FAMILY
        | id::FONT_STYLE
        | id::TEXT_ALIGN
        | id::TEXT_DECORATION
        | id::TEXT_OVERFLOW
        | id::WHITE_SPACE
        | id::FONT_FEATURE
        | id::USER_SELECT
        | id::SPRING
        | id::FONT_SIZE
        | id::FONT_WEIGHT => keyword(out, def, v),
        id::GROW => set(out, P::Grow, |p| p.layout.grow = f(v).max(0.0)),
        id::SHRINK => set(out, P::Shrink, |p| p.layout.shrink = f(v).max(0.0)),
        id::BASIS => set(out, P::Basis, |p| p.layout.basis = dim(v)),
        id::GAP => {
            set(out, P::RowGap, |p| p.layout.row_gap = f(v).max(0.0));
            set(out, P::ColumnGap, |p| p.layout.col_gap = f(v).max(0.0));
        }
        id::ROW_GAP => set(out, P::RowGap, |p| p.layout.row_gap = f(v).max(0.0)),
        id::COLUMN_GAP => set(out, P::ColumnGap, |p| p.layout.col_gap = f(v).max(0.0)),
        id::PADDING => {
            for (i, pid) in [
                P::PaddingTop,
                P::PaddingRight,
                P::PaddingBottom,
                P::PaddingLeft,
            ]
            .into_iter()
            .enumerate()
            {
                set(out, pid, |p| p.layout.padding[i] = f(v).max(0.0));
            }
        }
        id::PADDING_TOP..=id::PADDING_LEFT => {
            let i = (prop - id::PADDING_TOP) as usize;
            let pid = [
                P::PaddingTop,
                P::PaddingRight,
                P::PaddingBottom,
                P::PaddingLeft,
            ][i];
            set(out, pid, |p| p.layout.padding[i] = f(v).max(0.0));
        }
        id::MARGIN => {
            for (i, pid) in [P::MarginTop, P::MarginRight, P::MarginBottom, P::MarginLeft]
                .into_iter()
                .enumerate()
            {
                set(out, pid, |p| p.layout.margin[i] = dim(v));
            }
        }
        id::MARGIN_TOP..=id::MARGIN_LEFT => {
            let i = (prop - id::MARGIN_TOP) as usize;
            let pid = [P::MarginTop, P::MarginRight, P::MarginBottom, P::MarginLeft][i];
            set(out, pid, |p| p.layout.margin[i] = dim(v));
        }
        id::WIDTH => set(out, P::Width, |p| p.layout.width = dim(v)),
        id::HEIGHT => set(out, P::Height, |p| p.layout.height = dim(v)),
        id::MIN_WIDTH => set(out, P::MinWidth, |p| p.layout.min_width = dim(v)),
        id::MIN_HEIGHT => set(out, P::MinHeight, |p| p.layout.min_height = dim(v)),
        id::MAX_WIDTH => set(out, P::MaxWidth, |p| p.layout.max_width = dim(v)),
        id::MAX_HEIGHT => set(out, P::MaxHeight, |p| p.layout.max_height = dim(v)),
        id::ASPECT_RATIO => set(out, P::AspectRatio, |p| p.layout.aspect = f(v).max(0.0)),
        id::INSET_TOP..=id::INSET_LEFT => {
            let i = (prop - id::INSET_TOP) as usize;
            let pid = [P::InsetTop, P::InsetRight, P::InsetBottom, P::InsetLeft][i];
            set(out, pid, |p| p.layout.inset[i] = dim(v));
        }
        id::Z_INDEX => set(out, P::ZIndex, |p| {
            p.layout.z_index = (v.w[0] as i32).clamp(-1000, 1000) as i16
        }),
        id::BACKGROUND => set(out, P::Background, |p| {
            p.paint.background = Paint::Color(token(v.w[0]))
        }),
        id::BACKGROUND_GRADIENT => set(out, P::Background, |p| {
            let n = (v.w[1] as u8).clamp(2, 3);
            let mut stops = [ColorRef::Transparent; 3];
            for (i, s) in stops.iter_mut().enumerate().take(n as usize) {
                *s = token(v.w[2 + i]);
            }
            p.paint.background = Paint::Linear {
                angle: f(v).clamp(-360.0, 360.0) as i16,
                stops,
                n,
            };
        }),
        id::BORDER => {
            set(out, P::BorderWidth, |p| {
                p.layout.border_width = f(v).max(0.0)
            });
            set(out, P::BorderColor, |p| {
                p.paint.border_color = token(v.w[1])
            });
        }
        id::BORDER_RADIUS => {
            for (i, pid) in [P::RadiusTl, P::RadiusTr, P::RadiusBr, P::RadiusBl]
                .into_iter()
                .enumerate()
            {
                set(out, pid, |p| p.paint.radius[i] = f(v).max(0.0));
            }
        }
        id::RADIUS_TL..=id::RADIUS_BL => {
            let i = (prop - id::RADIUS_TL) as usize;
            let pid = [P::RadiusTl, P::RadiusTr, P::RadiusBr, P::RadiusBl][i];
            set(out, pid, |p| p.paint.radius[i] = f(v).max(0.0));
        }
        id::SHADOW_1 | id::SHADOW_2 => {
            let slot = usize::from(prop == id::SHADOW_2);
            let sh = Shadow {
                dx: f(v),
                dy: f32::from_bits(v.w[1]),
                blur: f32::from_bits(v.w[2]).max(0.0),
                color: token(v.w[3]),
            };
            // The other slot keeps its value if it was already declared inline.
            if out.mask & P::BoxShadow.bit() == 0 {
                out.p.paint.shadow = [Shadow::NONE; 2];
            }
            set(out, P::BoxShadow, |p| p.paint.shadow[slot] = sh);
        }
        id::OPACITY => set(out, P::Opacity, |p| p.paint.opacity = f(v).clamp(0.0, 1.0)),
        id::OUTLINE => {
            set(out, P::OutlineWidth, |p| {
                p.paint.outline_width = f(v).max(0.0)
            });
            set(out, P::OutlineColor, |p| {
                p.paint.outline_color = token(v.w[1])
            });
        }
        id::TRANSLATE | id::SCALE | id::ROTATE => {
            if out.mask & P::Transform.bit() == 0 {
                out.p.paint.transform = Transform::default();
            }
            set(out, P::Transform, |p| {
                let t = &mut p.paint.transform;
                match prop {
                    id::TRANSLATE => (t.tx, t.ty) = (f(v), f32::from_bits(v.w[1])),
                    id::SCALE => (t.sx, t.sy) = (f(v).max(0.0), f32::from_bits(v.w[1]).max(0.0)),
                    _ => t.rotate = f(v),
                }
            });
        }
        id::COLOR => set(out, P::Color, |p| p.text.color = token(v.w[0])),
        id::LINE_HEIGHT => set(out, P::LineHeight, |p| p.text.line_height = f(v).max(0.0)),
        id::LETTER_SPACING => set(out, P::LetterSpacing, |p| {
            p.text.letter_spacing = f(v).clamp(-2.0, 8.0)
        }),
        id::MAX_LINES => set(out, P::MaxLines, |p| {
            p.text.max_lines = (v.w[0] as i32).clamp(0, 1000) as u16
        }),
        id::TRANSITION => {
            let ms = tk::MOTION
                .get(v.w[1] as usize)
                .map_or(250, |m| m.value as u16);
            let ease = Ease::from_u8(v.w[2] as u8).unwrap_or(Ease::Standard);
            set(out, P::Transition, |p| {
                p.motion.transition = Transition {
                    props: transition_mask(v.w[0]),
                    ms,
                    ease,
                };
            });
        }
        _ => {}
    }
}

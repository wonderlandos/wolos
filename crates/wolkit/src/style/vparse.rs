// SPDX-License-Identifier: BSD-2-Clause
//! Value tokenizer and the per-kind value parsers of `.wolcss`. Each parser reads one value from a
//! `Cur` and returns the typed value or one `PErr` (lint id, span, sentence, suggestion).
use super::lint as L;
use super::tokens::{self as tk, ShadowTok, Token};
use super::value::*;

/// A parse error inside one value: byte span in the file.
#[derive(Clone, Debug, PartialEq)]
pub struct PErr {
    pub id: &'static str,
    pub s: usize,
    pub e: usize,
    pub msg: String,
    pub help: Option<String>,
}

pub type PRes<T> = Result<T, PErr>;

#[derive(Clone, Debug, PartialEq)]
pub enum T {
    Ident(String),
    /// Value and unit (`""`, `"px"`, `"%"`, `"ms"`, `"deg"`).
    Num(f32, String),
    Hash(String),
    Func(String),
    RParen,
    Comma,
    Slash,
    Bang,
    Str,
    Other(char),
}

#[derive(Clone, Debug)]
pub struct Tok {
    pub t: T,
    pub s: usize,
    pub e: usize,
}

/// Tokenize `src[from..to]`; offsets in the tokens are file offsets.
pub fn tokenize(src: &str, from: usize, to: usize) -> Vec<Tok> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = from;
    let is_id0 = |c: u8| c.is_ascii_alphabetic() || c == b'_' || c == b'-';
    let is_id = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'-';
    while i < to {
        let c = b[i];
        let s = i;
        if c.is_ascii_whitespace() {
            i += 1;
        } else if c == b',' {
            i += 1;
            out.push(Tok { t: T::Comma, s, e: i });
        } else if c == b'/' {
            i += 1;
            out.push(Tok { t: T::Slash, s, e: i });
        } else if c == b')' {
            i += 1;
            out.push(Tok { t: T::RParen, s, e: i });
        } else if c == b'!' {
            i += 1;
            out.push(Tok { t: T::Bang, s, e: i });
        } else if c == b'#' {
            i += 1;
            while i < to && is_id(b[i]) {
                i += 1;
            }
            out.push(Tok { t: T::Hash(src[s..i].to_string()), s, e: i });
        } else if c == b'"' || c == b'\'' {
            i += 1;
            while i < to && b[i] != c {
                i += 1;
            }
            i = (i + 1).min(to);
            out.push(Tok { t: T::Str, s, e: i });
        } else if c.is_ascii_digit()
            || c == b'.'
            || ((c == b'-' || c == b'+') && i + 1 < to && (b[i + 1].is_ascii_digit() || b[i + 1] == b'.'))
        {
            i += 1;
            while i < to && (b[i].is_ascii_digit() || b[i] == b'.') {
                i += 1;
            }
            let num: f32 = src[s..i].parse().unwrap_or(f32::NAN);
            let us = i;
            while i < to && (b[i].is_ascii_alphabetic() || b[i] == b'%') {
                i += 1;
            }
            out.push(Tok { t: T::Num(num, src[us..i].to_ascii_lowercase()), s, e: i });
        } else if is_id0(c) {
            i += 1;
            while i < to && is_id(b[i]) {
                i += 1;
            }
            if i < to && b[i] == b'(' {
                let name = src[s..i].to_ascii_lowercase();
                i += 1;
                out.push(Tok { t: T::Func(name), s, e: i });
            } else {
                out.push(Tok { t: T::Ident(src[s..i].to_string()), s, e: i });
            }
        } else {
            let ch = src[i..].chars().next().unwrap_or('?');
            i += ch.len_utf8();
            out.push(Tok { t: T::Other(ch), s, e: i });
        }
    }
    out
}

/// Read position over the tokens of one declaration value.
pub struct Cur {
    pub toks: Vec<Tok>,
    pub i: usize,
    /// Property being parsed (for sentences).
    pub prop: &'static str,
    /// Offset just after the value, for "expected a value" errors.
    pub end: usize,
    /// The file carries the off-scale pragma.
    pub off_scale: bool,
}

impl Cur {
    pub fn new(toks: Vec<Tok>, prop: &'static str, end: usize, off_scale: bool) -> Self {
        Cur { toks, i: 0, prop, end, off_scale }
    }
    pub fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.i)
    }
    pub fn at_end(&self) -> bool {
        self.i >= self.toks.len()
    }
    pub fn next(&mut self) -> PRes<Tok> {
        match self.toks.get(self.i) {
            Some(t) => {
                self.i += 1;
                Ok(t.clone())
            }
            None => Err(self.err_end(format!("`{}` needs a value here.", self.prop))),
        }
    }
    pub fn err_end(&self, msg: String) -> PErr {
        PErr { id: L::BAD_VALUE, s: self.end, e: self.end, msg, help: None }
    }
    pub fn err(&self, t: &Tok, id: &'static str, msg: String, help: Option<String>) -> PErr {
        PErr { id, s: t.s, e: t.e, msg, help }
    }
    pub fn bad(&self, t: &Tok, grammar: &str) -> PErr {
        let found = describe(&t.t);
        self.err(
            t,
            unsupported_id(&t.t).unwrap_or(L::BAD_VALUE),
            format!("`{}` does not take {}.", self.prop, found),
            Some(format!("write {grammar}")),
        )
    }
    pub fn eat_comma(&mut self) -> bool {
        if matches!(self.peek().map(|t| &t.t), Some(T::Comma)) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    pub fn expect_end(&self) -> PRes<()> {
        match self.peek() {
            None => Ok(()),
            Some(t) => Err(self.err(
                t,
                L::BAD_VALUE,
                format!("`{}` has more in its value than it can use.", self.prop),
                Some("remove the extra part".to_string()),
            )),
        }
    }
}

fn describe(t: &T) -> String {
    match t {
        T::Ident(s) => format!("`{s}`"),
        T::Num(v, u) => format!("`{v}{u}`"),
        T::Hash(s) => format!("`{s}`"),
        T::Func(f) => format!("`{f}()`"),
        T::Str => "a string".to_string(),
        T::Comma => "a comma here".to_string(),
        _ => "this".to_string(),
    }
}

/// Features that exist in CSS but not in 1.0 get their own lint.
fn unsupported_id(t: &T) -> Option<&'static str> {
    match t {
        T::Func(f) if matches!(f.as_str(), "calc" | "min" | "max" | "clamp" | "var" | "env" | "blur" | "drop-shadow" | "brightness") => {
            Some(L::UNSUPPORTED)
        }
        _ => None,
    }
}

/// Keyword enums parse by their keyword list.
pub trait Kw: Sized + Copy {
    const KEYWORDS: &'static [&'static str];
    fn from_kw(s: &str) -> Option<Self>;
}
macro_rules! impl_kw {
    ($($t:ty),+) => {$(
        impl Kw for $t {
            const KEYWORDS: &'static [&'static str] = <$t>::KEYWORDS;
            fn from_kw(s: &str) -> Option<Self> { <$t>::parse(s) }
        }
    )+};
}
impl_kw!(
    Display, Direction, Wrap, AlignItems, AlignSelf, AlignContent, Justify, Position, Overflow,
    Cursor, PointerEvents, Visibility, Backdrop, FontFamily, FontStyle, TextAlign, TextDecoration,
    TextOverflow, WhiteSpace, UserSelect, Ease, Spring
);

fn list(words: &[&str]) -> String {
    words.iter().map(|w| format!("`{w}`")).collect::<Vec<_>>().join(", ")
}

pub fn parse_kw<E: Kw>(c: &mut Cur) -> PRes<E> {
    let t = c.next()?;
    if let T::Ident(s) = &t.t {
        if let Some(v) = E::from_kw(s) {
            return Ok(v);
        }
        let help = match tk::suggest(s, E::KEYWORDS.iter().copied()) {
            Some(n) => format!("did you mean `{n}`? The values are {}.", list(E::KEYWORDS)),
            None => format!("the values are {}.", list(E::KEYWORDS)),
        };
        return Err(c.err(&t, L::BAD_VALUE, format!("`{s}` is not a value of `{}`.", c.prop), Some(help)));
    }
    Err(c.bad(&t, &format!("one of {}", list(E::KEYWORDS))))
}

fn num_in(c: &Cur, t: &Tok, v: f32, lo: f32, hi: f32, what: &str) -> PRes<f32> {
    if v.is_nan() || v < lo || v > hi {
        return Err(c.err(
            t,
            L::BAD_VALUE,
            format!("`{}` takes {what} from {lo} to {hi}.", c.prop),
            None,
        ));
    }
    Ok(v)
}

fn scale_names(steps: &[tk::Step]) -> String {
    steps.iter().map(|s| format!("`{}`", s.name)).collect::<Vec<_>>().join(", ")
}

fn off_scale_err(c: &Cur, t: &Tok, px: f32, spacing: bool) -> PErr {
    let (id, steps, what) = if spacing {
        (L::OFF_SCALE_SPACING, &tk::SPACE[..], "spacing")
    } else {
        (L::OFF_SCALE_RADIUS, &tk::RADIUS[..], "radius")
    };
    let values: Vec<String> = steps.iter().filter(|s| s.value < 1000.0).map(|s| format!("{}px", s.value)).collect();
    let near = steps
        .iter()
        .filter(|s| s.value < 1000.0)
        .min_by(|a, b| (a.value - px).abs().total_cmp(&(b.value - px).abs()))
        .map(|s| s.name)
        .unwrap_or("none");
    c.err(
        t,
        id,
        format!("{px}px is not on the {what} scale ({}).", values.join(", ")),
        Some(format!(
            "use a name ({}), the nearest is `{near}`; a file may opt out with /* wolkit: off-scale */",
            scale_names(steps)
        )),
    )
}

/// A scale value: a name, or a pixel value that is on the scale (or anything, after the pragma).
fn scale_value(c: &mut Cur, spacing: bool, allow_neg: bool) -> PRes<f32> {
    let t = c.next()?;
    let steps: &[tk::Step] = if spacing { &tk::SPACE } else { &tk::RADIUS };
    match &t.t {
        T::Ident(s) => match steps.iter().find(|x| x.name == s.as_str()) {
            Some(x) => Ok(x.value),
            None => {
                let help = match tk::suggest(s, steps.iter().map(|x| x.name)) {
                    Some(n) => format!("did you mean `{n}`? The names are {}.", scale_names(steps)),
                    None => format!("the names are {}.", scale_names(steps)),
                };
                Err(c.err(&t, L::UNKNOWN_NAME, format!("`{s}` is not a {} name.", if spacing { "spacing" } else { "radius" }), Some(help)))
            }
        },
        T::Num(v, u) if u.is_empty() || u == "px" => {
            if v.is_nan() {
                return Err(c.bad(&t, "a name or a pixel value"));
            }
            if *v == 0.0 {
                return Ok(0.0);
            }
            if !u.is_empty() || *v != 0.0 {
                let mag = v.abs();
                let neg = *v < 0.0;
                if neg && !allow_neg {
                    return Err(c.err(&t, L::BAD_VALUE, format!("`{}` cannot be negative.", c.prop), None));
                }
                if u.is_empty() {
                    return Err(c.err(
                        &t,
                        L::BAD_VALUE,
                        format!("`{}` needs a unit on `{v}`.", c.prop),
                        Some(format!("write `{v}px` or a name ({})", scale_names(steps))),
                    ));
                }
                let on = if spacing { tk::on_space_scale(mag) } else { tk::on_radius_scale(mag) };
                if !on && !c.off_scale {
                    return Err(off_scale_err(c, &t, *v, spacing));
                }
                return Ok(*v);
            }
            Ok(0.0)
        }
        _ => Err(c.bad(&t, &format!("a name ({}) or a pixel value on the scale", scale_names(steps)))),
    }
}

pub fn parse_space(c: &mut Cur) -> PRes<f32> {
    scale_value(c, true, false)
}
pub fn parse_radius(c: &mut Cur) -> PRes<f32> {
    scale_value(c, false, false)
}

pub fn parse_margin(c: &mut Cur) -> PRes<Dim> {
    if let Some(Tok { t: T::Ident(s), .. }) = c.peek() {
        if s == "auto" {
            c.i += 1;
            return Ok(Dim::Auto);
        }
    }
    Ok(Dim::Px(scale_value(c, true, true)?))
}

/// Lengths for sizes and insets: `auto`/`none`, pixels, or a percentage; spacing names also work.
fn parse_length(c: &mut Cur, allow_neg: bool, none_word: &'static str) -> PRes<Dim> {
    let t = c.next()?;
    match &t.t {
        T::Ident(s) if s == "auto" || s == none_word => Ok(Dim::Auto),
        T::Ident(s) => match tk::space_value(s) {
            Some(v) => Ok(Dim::Px(v)),
            None => Err(c.err(
                &t,
                L::BAD_VALUE,
                format!("`{s}` is not a length."),
                Some("write auto, a pixel value, a percentage or a spacing name".to_string()),
            )),
        },
        T::Num(v, u) if u == "px" || u == "%" || (u.is_empty() && *v == 0.0) => {
            if v.is_nan() || (!allow_neg && *v < 0.0) || v.abs() > 100_000.0 {
                return Err(c.err(&t, L::BAD_VALUE, format!("`{}` takes a length from 0 to 100000.", c.prop), None));
            }
            Ok(if u == "%" { Dim::Pct(*v) } else { Dim::Px(*v) })
        }
        _ => Err(c.bad(&t, "auto, a pixel value or a percentage")),
    }
}

pub fn parse_size(c: &mut Cur) -> PRes<Dim> {
    parse_length(c, false, "none")
}
pub fn parse_inset(c: &mut Cur) -> PRes<Dim> {
    parse_length(c, true, "auto")
}

pub fn parse_nonneg(c: &mut Cur) -> PRes<f32> {
    let t = c.next()?;
    match &t.t {
        T::Num(v, u) if u.is_empty() => num_in(c, &t, *v, 0.0, 1000.0, "a number"),
        _ => Err(c.bad(&t, "a number such as 1")),
    }
}

pub fn parse_aspect(c: &mut Cur) -> PRes<f32> {
    let t = c.next()?;
    match &t.t {
        T::Ident(s) if s == "auto" => Ok(0.0),
        T::Num(v, u) if u.is_empty() => {
            let a = num_in(c, &t, *v, 0.05, 20.0, "a ratio")?;
            if matches!(c.peek().map(|x| &x.t), Some(T::Slash)) {
                c.i += 1;
                let d = c.next()?;
                if let T::Num(w, u2) = &d.t {
                    if u2.is_empty() {
                        let w = num_in(c, &d, *w, 0.05, 20.0, "a ratio")?;
                        return Ok(a / w);
                    }
                }
                return Err(c.bad(&d, "a ratio such as 16 / 9"));
            }
            Ok(a)
        }
        _ => Err(c.bad(&t, "auto, a number or a ratio such as 16 / 9")),
    }
}

pub fn parse_zindex(c: &mut Cur) -> PRes<i16> {
    let t = c.next()?;
    match &t.t {
        T::Num(v, u) if u.is_empty() && v.fract() == 0.0 => Ok(num_in(c, &t, *v, -1000.0, 1000.0, "a whole number")? as i16),
        _ => Err(c.bad(&t, "a whole number from -1000 to 1000")),
    }
}

fn px_in(c: &mut Cur, lo: f32, hi: f32) -> PRes<f32> {
    let t = c.next()?;
    match &t.t {
        T::Num(v, u) if u == "px" || (u.is_empty() && *v == 0.0) => num_in(c, &t, *v, lo, hi, "pixels"),
        _ => Err(c.bad(&t, "a pixel value such as 1px")),
    }
}

pub fn parse_border_width(c: &mut Cur) -> PRes<f32> {
    px_in(c, 0.0, 16.0)
}
pub fn parse_outline_width(c: &mut Cur) -> PRes<f32> {
    px_in(c, 0.0, 8.0)
}
pub fn parse_letter_spacing(c: &mut Cur) -> PRes<f32> {
    if let Some(Tok { t: T::Ident(s), .. }) = c.peek() {
        if s == "normal" {
            c.i += 1;
            return Ok(0.0);
        }
    }
    px_in(c, -2.0, 8.0)
}

pub fn parse_opacity(c: &mut Cur) -> PRes<f32> {
    let t = c.next()?;
    match &t.t {
        T::Num(v, u) if u.is_empty() => num_in(c, &t, *v, 0.0, 1.0, "a number"),
        T::Num(v, u) if u == "%" => Ok(num_in(c, &t, *v, 0.0, 100.0, "a percentage")? / 100.0),
        _ => Err(c.bad(&t, "a number from 0 to 1 or a percentage")),
    }
}

pub fn parse_color(c: &mut Cur) -> PRes<ColorRef> {
    let t = c.next()?;
    match &t.t {
        T::Ident(s) if s == "transparent" => Ok(ColorRef::Transparent),
        T::Ident(s) => match Token::parse(s) {
            Some(tok) => Ok(ColorRef::Token(tok)),
            None => {
                let help = match tk::suggest(s, Token::NAMES.iter().copied()) {
                    Some(n) => format!("did you mean `{n}`? The tokens are {}.", list(&Token::NAMES)),
                    None => format!("the tokens are {}.", list(&Token::NAMES)),
                };
                Err(c.err(&t, L::UNKNOWN_TOKEN, format!("`{s}` is not a colour token."), Some(help)))
            }
        },
        T::Hash(h) => Err(c.err(
            &t,
            L::HEX_LITERAL,
            format!("`{h}` is a hex colour; colours here are theme tokens."),
            Some(format!("use a token such as `card` or `accent`; the tokens are {}", list(&Token::NAMES))),
        )),
        T::Func(f) if f == "alpha" => {
            let tt = c.next()?;
            let tok = match &tt.t {
                T::Ident(s) => Token::parse(s).ok_or_else(|| {
                    c.err(&tt, L::UNKNOWN_TOKEN, format!("`{s}` is not a colour token."), Some(format!("the tokens are {}.", list(&Token::NAMES))))
                })?,
                _ => return Err(c.bad(&tt, "a token name first, as in alpha(accent, 20%)")),
            };
            if !c.eat_comma() {
                return Err(c.bad(&tt, "alpha(token, 20%)"));
            }
            let at = c.next()?;
            let a = match &at.t {
                T::Num(v, u) if u == "%" => num_in(c, &at, *v, 0.0, 100.0, "a percentage")?,
                _ => return Err(c.bad(&at, "a percentage such as 20%")),
            };
            let close = c.next()?;
            if close.t != T::RParen {
                return Err(c.bad(&close, "a closing parenthesis"));
            }
            Ok(ColorRef::Alpha(tok, a.round() as u8))
        }
        T::Func(f) if matches!(f.as_str(), "rgb" | "rgba" | "hsl" | "hsla" | "hwb" | "lab" | "lch" | "oklab" | "oklch" | "color" | "color-mix") => {
            Err(c.err(
                &t,
                L::COLOR_FUNCTION,
                format!("`{f}()` is a raw colour; colours here are theme tokens."),
                Some("use a token, or alpha(token, 20%) for a see-through one".to_string()),
            ))
        }
        _ => Err(c.bad(&t, "a colour token such as `card`")),
    }
}

pub fn parse_paint(c: &mut Cur) -> PRes<Paint> {
    if let Some(Tok { t: T::Ident(s), .. }) = c.peek() {
        if s == "none" {
            c.i += 1;
            return Ok(Paint::None);
        }
    }
    if let Some(Tok { t: T::Func(f), .. }) = c.peek() {
        if f == "linear-gradient" {
            c.i += 1;
            let at = c.next()?;
            let angle = match &at.t {
                T::Num(v, u) if u == "deg" => num_in(c, &at, *v, -360.0, 360.0, "degrees")? as i16,
                T::Ident(s) if s == "to" => {
                    let d = c.next()?;
                    let name = match &d.t {
                        T::Ident(n) => n.clone(),
                        _ => return Err(c.bad(&d, "to top, to bottom, to left or to right")),
                    };
                    match name.as_str() {
                        "top" => 0,
                        "right" => 90,
                        "bottom" => 180,
                        "left" => 270,
                        _ => return Err(c.err(&d, L::BAD_VALUE, format!("`to {name}` is not a direction."), Some("use top, bottom, left or right".into()))),
                    }
                }
                _ => return Err(c.bad(&at, "an angle such as 180deg or `to bottom` first")),
            };
            let mut stops = [ColorRef::Transparent; 3];
            let mut n = 0u8;
            while c.eat_comma() {
                if n == 3 {
                    let t = c.peek().cloned().unwrap_or(at.clone());
                    return Err(c.err(&t, L::BAD_VALUE, "A gradient has two or three stops.".into(), None));
                }
                stops[n as usize] = parse_color(c)?;
                n += 1;
            }
            if n < 2 {
                return Err(c.err(&at, L::BAD_VALUE, "A gradient has two or three stops.".into(), Some("add a colour after the angle".into())));
            }
            let close = c.next()?;
            if close.t != T::RParen {
                return Err(c.bad(&close, "a closing parenthesis"));
            }
            return Ok(Paint::Linear { angle, stops, n });
        }
    }
    Ok(Paint::Color(parse_color(c)?))
}

pub fn parse_shadow(c: &mut Cur) -> PRes<[Shadow; 2]> {
    let mut out = [Shadow::NONE; 2];
    let mut n = 0;
    let first = c.peek().cloned();
    loop {
        let t = c.next()?;
        let tok = match &t.t {
            T::Ident(s) => ShadowTok::parse(s).ok_or_else(|| {
                c.err(&t, L::UNKNOWN_NAME, format!("`{s}` is not a shadow."), Some("the shadows are `none`, `sm`, `md` and `lg`.".into()))
            })?,
            _ => return Err(c.bad(&t, "none, sm, md or lg")),
        };
        for l in tok.layers() {
            if n == 2 {
                return Err(c.err(&t, L::BAD_VALUE, "`box-shadow` holds two layers at most (`lg` is two).".into(), Some("use fewer or smaller shadows".into())));
            }
            out[n] = Shadow { dx: 0.0, dy: l.dy, blur: l.blur, color: ColorRef::Shade(l.alpha) };
            n += 1;
        }
        if !c.eat_comma() {
            break;
        }
    }
    let _ = first;
    Ok(out)
}

pub fn parse_transform(c: &mut Cur) -> PRes<Transform> {
    let mut tr = Transform::default();
    if let Some(Tok { t: T::Ident(s), .. }) = c.peek() {
        if s == "none" {
            c.i += 1;
            return Ok(tr);
        }
    }
    let mut any = false;
    while let Some(Tok { t: T::Func(f), .. }) = c.peek().cloned() {
        c.i += 1;
        any = true;
        match f.as_str() {
            "translate" => {
                tr.tx = px_in(c, -10000.0, 10000.0)?;
                tr.ty = if c.eat_comma() { px_in(c, -10000.0, 10000.0)? } else { 0.0 };
            }
            "scale" => {
                let t = c.next()?;
                match &t.t {
                    T::Num(v, u) if u.is_empty() => {
                        tr.sx = num_in(c, &t, *v, 0.0, 10.0, "a number")?;
                        tr.sy = tr.sx;
                    }
                    _ => return Err(c.bad(&t, "scale(1.05)")),
                }
                if c.eat_comma() {
                    let t2 = c.next()?;
                    match &t2.t {
                        T::Num(v, u) if u.is_empty() => tr.sy = num_in(c, &t2, *v, 0.0, 10.0, "a number")?,
                        _ => return Err(c.bad(&t2, "scale(1.05, 1)")),
                    }
                }
            }
            "rotate" => {
                let t = c.next()?;
                match &t.t {
                    T::Num(v, u) if u == "deg" => tr.rotate = num_in(c, &t, *v, -3600.0, 3600.0, "degrees")?,
                    _ => return Err(c.bad(&t, "rotate(10deg)")),
                }
            }
            other => {
                let t = c.toks[c.i - 1].clone();
                return Err(c.err(&t, L::UNSUPPORTED, format!("`{other}()` is not a transform in 1.0."), Some("use translate, scale or rotate".into())));
            }
        }
        let close = c.next()?;
        if close.t != T::RParen {
            return Err(c.bad(&close, "a closing parenthesis"));
        }
    }
    if !any {
        let t = c.next()?;
        return Err(c.bad(&t, "none or translate(0px, 4px) scale(1.05) rotate(10deg)"));
    }
    Ok(tr)
}

pub fn parse_font_size(c: &mut Cur) -> PRes<f32> {
    let t = c.next()?;
    match &t.t {
        T::Ident(s) => match tk::role(s) {
            Some(r) => Ok(r.size),
            None => {
                let names: Vec<&str> = tk::ROLES.iter().map(|r| r.name).collect();
                let help = match tk::suggest(s, names.iter().copied()) {
                    Some(n) => format!("did you mean `{n}`? The roles are {}.", list(&names)),
                    None => format!("the roles are {}.", list(&names)),
                };
                Err(c.err(&t, L::UNKNOWN_NAME, format!("`{s}` is not a type role."), Some(help)))
            }
        },
        T::Num(v, u) if u == "px" => {
            if *v < tk::MIN_FONT_PX {
                return Err(c.err(
                    &t,
                    L::FONT_TOO_SMALL,
                    format!("{v}px is below the smallest font size, {}px.", tk::MIN_FONT_PX),
                    Some("use `secondary` or `caption` (11px) or larger".into()),
                ));
            }
            num_in(c, &t, *v, tk::MIN_FONT_PX, 200.0, "pixels")
        }
        _ => Err(c.bad(&t, "a type role such as `body` or a size such as 14px")),
    }
}

pub fn parse_weight(c: &mut Cur) -> PRes<u16> {
    let t = c.next()?;
    match &t.t {
        T::Ident(s) if s == "normal" => Ok(400),
        T::Ident(s) if s == "bold" => Ok(700),
        T::Num(v, u) if u.is_empty() => {
            if matches!(*v as i32, 400 | 500 | 600 | 700) && v.fract() == 0.0 {
                Ok(*v as u16)
            } else {
                Err(c.err(&t, L::BAD_WEIGHT, format!("{v} is not a font weight here."), Some("the weights are 400, 500, 600 and 700".into())))
            }
        }
        _ => Err(c.bad(&t, "400, 500, 600 or 700")),
    }
}

pub fn parse_line_height(c: &mut Cur) -> PRes<f32> {
    let t = c.next()?;
    match &t.t {
        T::Ident(s) if s == "normal" => Ok(0.0),
        T::Num(v, u) if u.is_empty() => num_in(c, &t, *v, 0.8, 3.0, "a multiple"),
        _ => Err(c.bad(&t, "normal or a multiple such as 1.4")),
    }
}

pub fn parse_max_lines(c: &mut Cur) -> PRes<u16> {
    let t = c.next()?;
    match &t.t {
        T::Ident(s) if s == "none" => Ok(0),
        T::Num(v, u) if u.is_empty() && v.fract() == 0.0 => Ok(num_in(c, &t, *v, 1.0, 1000.0, "a whole number")? as u16),
        _ => Err(c.bad(&t, "none or a whole number")),
    }
}

pub fn parse_feature(c: &mut Cur) -> PRes<u8> {
    let t = c.next()?;
    match &t.t {
        T::Ident(s) if s == "none" => Ok(0),
        T::Ident(s) if s == "tnum" => Ok(FEATURE_TNUM),
        _ => Err(c.bad(&t, "none or tnum")),
    }
}

/// Duration: `fast`, `normal`, `slow` or `N ms`; over 500 ms is WK-004.
pub fn parse_duration(c: &mut Cur) -> PRes<u16> {
    let t = c.next()?;
    match &t.t {
        T::Ident(s) => tk::motion_ms(s).ok_or_else(|| {
            let names: Vec<&str> = tk::MOTION.iter().map(|m| m.name).collect();
            let help = match tk::suggest(s, names.iter().copied()) {
                Some(n) => format!("did you mean `{n}`?"),
                None => format!("the durations are {}.", list(&names)),
            };
            c.err(&t, L::UNKNOWN_NAME, format!("`{s}` is not a duration."), Some(help))
        }),
        T::Num(v, u) if u == "ms" || u == "s" => {
            let ms = if u == "s" { *v * 1000.0 } else { *v };
            if ms > MAX_TRANSITION_MS as f32 {
                return Err(c.err(
                    &t,
                    L::DURATION_TOO_LONG,
                    format!("{ms}ms is longer than the longest transition, {MAX_TRANSITION_MS}ms."),
                    Some("use `slow` (400ms) or shorter".into()),
                ));
            }
            Ok(num_in(c, &t, ms, 0.0, MAX_TRANSITION_MS as f32, "milliseconds")?.round() as u16)
        }
        _ => Err(c.bad(&t, "fast, normal, slow or a time such as 200ms")),
    }
}

/// `transition: opacity, background normal ease` : properties (comma list or `all`), then an
/// optional duration, then an optional ease.
pub fn parse_transition(c: &mut Cur) -> PRes<Transition> {
    use super::prop::PropId;
    let mut tr = Transition { props: 0, ms: 250, ease: Ease::Standard };
    let first = c.next()?;
    if let T::Ident(s) = &first.t {
        if s == "none" {
            return Ok(Transition::default());
        }
    }
    c.i -= 1;
    loop {
        let t = c.next()?;
        let name = match &t.t {
            T::Ident(s) => s.clone(),
            _ => return Err(c.bad(&t, "property names, then a duration and an ease")),
        };
        if name == "all" {
            for p in PropId::ALL {
                if p.info().anim {
                    tr.props |= 1u128 << (*p as u8);
                }
            }
        } else if let Some(p) = PropId::parse_name(&name) {
            if !p.info().anim {
                return Err(c.err(&t, L::NOT_ANIMATABLE, format!("`{name}` does not animate."), Some("transitions run on colours, opacity, transform, radius and shadows".into())));
            }
            tr.props |= 1u128 << (p as u8);
        } else if let Some(group) = group_props(&name) {
            for p in group {
                tr.props |= 1u128 << (*p as u8);
            }
        } else {
            let help = tk::suggest(&name, PropId::ALL.iter().map(|p| p.name()))
                .map(|n| format!("did you mean `{n}`?"));
            return Err(c.err(&t, L::UNKNOWN_PROPERTY, format!("`{name}` is not a property."), help));
        }
        if !c.eat_comma() {
            break;
        }
    }
    if tr.props == 0 {
        return Err(c.err(&first, L::BAD_VALUE, "A transition names at least one property.".into(), None));
    }
    if let Some(t) = c.peek().cloned() {
        if let T::Ident(s) = &t.t {
            if Ease::parse(s).is_none() {
                tr.ms = parse_duration(c)?;
            }
        } else {
            tr.ms = parse_duration(c)?;
        }
    }
    if let Some(t) = c.peek().cloned() {
        if let T::Ident(s) = &t.t {
            tr.ease = Ease::parse(s).ok_or_else(|| {
                let help = tk::suggest(s, Ease::KEYWORDS.iter().copied()).map(|n| format!("did you mean `{n}`?"));
                c.err(&t, L::UNKNOWN_NAME, format!("`{s}` is not an ease."), help.or(Some(format!("the eases are {}.", list(Ease::KEYWORDS)))))
            })?;
            c.i += 1;
        }
    }
    Ok(tr)
}

/// Shorthand names usable in a transition list.
fn group_props(name: &str) -> Option<&'static [super::prop::PropId]> {
    use super::prop::PropId as P;
    match name {
        "background" => Some(&[P::Background]),
        "border-radius" => Some(&[P::RadiusTl, P::RadiusTr, P::RadiusBr, P::RadiusBl]),
        _ => None,
    }
}

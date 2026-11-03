// SPDX-License-Identifier: BSD-2-Clause
//! Icons: an `image` node whose `image-source` is `icon:<name>` (the DSL's `icon_button :trash`)
//! draws that icon of the icon theme. The name is looked up as `<name>-symbolic.svg`,
//! `<name>.svg`, `<name>-symbolic.png` and `<name>.png` in the icon folders (with `_` read as
//! `-`, and a few short names mapped to their theme names: `back` is `go-previous`, `play` is
//! `media-playback-start`, `info` is `dialog-information`, `more` is `view-more`, `share` is
//! `send-to`, `trash` is `user-trash`). Symbolic SVG icons are drawn in the part's text colour
//! (`currentColor`); PNG icons keep their own colours.
//!
//! The SVG reader takes what symbolic icons are made of: a `viewBox`, `path` elements with `d`
//! (every command, arcs included), `fill` (`none` skips a path) and `opacity`. Paths are filled
//! non-zero by the exact-area rasteriser (`cpu::coverage`) after arcs are turned into cubic
//! curves with square roots only, so an icon is the same bytes on every machine. Anything else
//! in the file is ignored. Reading and rasterising run on the decode workers; an icon that is
//! not ready yet draws nothing.
//!
//! Folders (the first engine of the process decides, as for fonts): `WOLKIT_ICON_DIR` when set,
//! else for each data folder (`XDG_DATA_HOME`, then `XDG_DATA_DIRS`, default
//! `/usr/local/share:/usr/share`) `icons/WolOS/symbolic/{actions,status}`, `icons/WolOS/scalable/
//! {apps,places,devices,mimetypes}` and `icons/hicolor/symbolic/apps`. A headless engine reads
//! only `WOLKIT_ICON_DIR`, or `/usr/share/icons/WolOS/symbolic/{actions,status}` without it.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};

use super::{Entry, Level, Pixels, lock};
use crate::cpu::coverage::{Accumulator, Flattener};
use crate::error::{Error, Result};
use crate::num::trunc_u32;
use crate::paint::geom::{Color, div255};
use crate::scene::ids::{Ix, to_u8};

/// The nominal size of an icon part with no size of its own, logical pixels.
pub const NOMINAL: f32 = 16.0;
/// The variable naming one folder of icons (tests, a checkout's `design/icons/symbolic`).
pub const ICON_DIR_ENV: &str = "WOLKIT_ICON_DIR";
const MAX_SVG: u64 = 256 << 10;
const MAX_PATHS: usize = 1024;
const MAX_COMMANDS: usize = 65_536;
const MAX_PX: u32 = 512;
const MAX_CACHED: usize = 512;

const ALIASES: &[(&str, &str)] = &[
    ("back", "go-previous"),
    ("forward", "go-next"),
    ("up", "go-up"),
    ("down", "go-down"),
    ("home", "go-home"),
    ("play", "media-playback-start"),
    ("pause", "media-playback-pause"),
    ("stop", "media-playback-stop"),
    ("info", "dialog-information"),
    ("more", "view-more"),
    ("menu", "open-menu"),
    ("share", "send-to"),
    ("trash", "user-trash"),
    ("delete", "edit-delete"),
    ("close", "window-close"),
    ("search", "system-search"),
    ("add", "list-add"),
    ("remove", "list-remove"),
    ("refresh", "view-refresh"),
    ("star", "starred"),
    ("undo", "edit-undo"),
    ("redo", "edit-redo"),
    ("copy", "edit-copy"),
    ("paste", "edit-paste"),
];

static DIRS: OnceLock<Vec<PathBuf>> = OnceLock::new();
/// Rasterised icons by name, pixel size and packed colour.
type Cached = HashMap<(String, u32, u32), Arc<Entry>>;
static CACHE: LazyLock<Mutex<Cached>> = LazyLock::new(Default::default);

pub fn is_icon(src: &str) -> bool {
    src.starts_with("icon:")
}

/// The icon folders for an engine made with `headless` (the environment as a lookup).
pub fn dirs_for(headless: bool, var: impl Fn(&str) -> Option<std::ffi::OsString>) -> Vec<PathBuf> {
    if let Some(d) = var(ICON_DIR_ENV).filter(|d| !d.is_empty()) {
        return vec![PathBuf::from(d)];
    }
    let theme = |base: &Path| -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = ["symbolic/actions", "symbolic/status"]
            .iter()
            .map(|s| base.join("icons/WolOS").join(s))
            .collect();
        if !headless {
            for s in [
                "scalable/apps",
                "scalable/places",
                "scalable/devices",
                "scalable/mimetypes",
            ] {
                v.push(base.join("icons/WolOS").join(s));
            }
            v.push(base.join("icons/hicolor/symbolic/apps"));
        }
        v
    };
    if headless {
        return theme(Path::new("/usr/share"));
    }
    let mut bases = Vec::new();
    match var("XDG_DATA_HOME").filter(|d| !d.is_empty()) {
        Some(d) => bases.push(PathBuf::from(d)),
        None => {
            if let Some(h) = var("HOME").filter(|d| !d.is_empty()) {
                bases.push(PathBuf::from(h).join(".local/share"));
            }
        }
    }
    let data = var("XDG_DATA_DIRS").filter(|d| !d.is_empty()).map_or_else(
        || "/usr/local/share:/usr/share".into(),
        |d| d.to_string_lossy().into_owned(),
    );
    bases.extend(
        data.split(':')
            .filter(|d| d.starts_with('/'))
            .map(PathBuf::from),
    );
    bases.iter().flat_map(|b| theme(b)).collect()
}

/// Use `dirs` for icons in this process (the first call wins).
pub fn install(dirs: Vec<PathBuf>) {
    let _ = DIRS.set(dirs);
}

fn dirs() -> &'static [PathBuf] {
    DIRS.get_or_init(|| dirs_for(true, |_| None))
}

/// The file names to try for the DSL name `name`, in order.
pub fn candidates(name: &str) -> Vec<String> {
    let n = name.replace('_', "-");
    let n = ALIASES
        .iter()
        .find(|(k, _)| *k == n)
        .map_or(n.clone(), |(_, v)| (*v).to_owned());
    vec![
        format!("{n}-symbolic.svg"),
        format!("{n}.svg"),
        format!("{n}-symbolic.png"),
        format!("{n}.png"),
    ]
}

fn ok_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 96
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_' || c == b'.')
        && !name.starts_with('.')
}

/// The icon to draw for `src` (`icon:<name>`) in a part of `w x h` logical pixels in `color`.
/// It is rasterised at twice the part's size (a high-density screen; the mip chain serves 1x).
pub fn get(src: &str, w: f32, h: f32, color: Color) -> Option<Arc<Entry>> {
    let name = src.strip_prefix("icon:")?;
    if !ok_name(name) {
        return None;
    }
    let side = if w > 0.0 && h > 0.0 {
        w.min(h)
    } else {
        NOMINAL
    };
    let px = trunc_u32((side * 2.0).ceil()).clamp(8, MAX_PX);
    let key = (name.to_owned(), px, color.pack());
    let mut c = lock(&CACHE);
    if let Some(e) = c.get(&key) {
        return Some(Arc::clone(e));
    }
    if c.len() >= MAX_CACHED {
        c.clear();
    }
    let e = Entry::icon(name.to_owned(), px, color);
    c.insert(key, Arc::clone(&e));
    Some(e)
}

/// Memory Saver: forget the rasterised icons.
pub fn trim() {
    lock(&CACHE).clear();
}

/// Worker side: find, read and draw the icon `name` at `px` pixels square.
pub fn load(name: &str, px: u32, color: Color) -> Result<Pixels> {
    let none = || Error::invalid(format!("There is no icon named {name}."));
    let file = candidates(name)
        .into_iter()
        .flat_map(|f| dirs().iter().map(move |d| d.join(&f)))
        .find(|p| p.is_file())
        .ok_or_else(none)?;
    let f = std::fs::File::open(&file).map_err(|_| none())?;
    let mut b = Vec::new();
    f.take(MAX_SVG * 64)
        .read_to_end(&mut b)
        .map_err(|_| none())?;
    if file.extension().is_some_and(|x| x == "png") {
        return super::decode::decode(&b, px);
    }
    if u64::try_from(b.len()).unwrap_or(u64::MAX) > MAX_SVG {
        return Err(Error::limit("An icon file is at most 256 KiB."));
    }
    let text = std::str::from_utf8(&b).map_err(|_| Error::invalid(BAD_SVG))?;
    let level = raster(text, px, color)?;
    Ok(Pixels::from_level(px, px, level))
}

pub const BAD_SVG: &str = "The icon could not be read: it is not a symbolic SVG icon.";

// ---- the SVG subset ----

/// The value of attribute `name` in the tag text `tag` (`name="…"` or `name='…'`).
fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = tag;
    while let Some(i) = rest.find(name) {
        let before = rest[..i].chars().last();
        let after = &rest[i + name.len()..];
        rest = after;
        if before.is_some_and(|c| !c.is_whitespace()) {
            continue;
        }
        let after = after.trim_start();
        let Some(after) = after.strip_prefix('=') else {
            continue;
        };
        let after = after.trim_start();
        let q = after.chars().next()?;
        if q != '"' && q != '\'' {
            return None;
        }
        let body = &after[1..];
        return body.find(q).map(|e| &body[..e]);
    }
    None
}

fn numbers(s: &str) -> Vec<f32> {
    let mut out = Vec::new();
    let mut p = PathReader {
        b: s.as_bytes(),
        i: 0,
    };
    while let Some(v) = p.number() {
        out.push(v);
    }
    out
}

/// The `viewBox` (or `width` and `height`) of the root element.
fn view_box(svg: &str) -> Option<[f32; 4]> {
    let start = svg.find("<svg")?;
    let end = svg[start..].find('>')? + start;
    let tag = &svg[start..end];
    if let Some(vb) = attr(tag, "viewBox") {
        let v = numbers(vb);
        if v.len() == 4 && v[2] > 0.0 && v[3] > 0.0 {
            return Some([v[0], v[1], v[2], v[3]]);
        }
    }
    let w = attr(tag, "width").map(numbers)?.first().copied()?;
    let h = attr(tag, "height").map(numbers)?.first().copied()?;
    (w > 0.0 && h > 0.0).then_some([0.0, 0.0, w, h])
}

/// A symbolic SVG drawn `px` pixels square in `color`, premultiplied.
pub fn raster(svg: &str, px: u32, color: Color) -> Result<Level> {
    let vb = view_box(svg).ok_or_else(|| Error::invalid(BAD_SVG))?;
    let n = px.ix();
    let s = px as f32 / vb[2].max(vb[3]);
    let ox = (px as f32 - vb[2] * s) / 2.0 - vb[0] * s;
    let oy = (px as f32 - vb[3] * s) / 2.0 - vb[1] * s;
    let mut alpha = vec![0u8; n * n];
    let mut acc = Accumulator::new();
    let mut cov = vec![0u8; n * n];
    let mut paths = 0;
    let mut rest = svg;
    while let Some(i) = rest.find("<path") {
        let tail = &rest[i..];
        let end = tail.find('>').ok_or_else(|| Error::invalid(BAD_SVG))?;
        let tag = &tail[..end];
        rest = &tail[end..];
        paths += 1;
        if paths > MAX_PATHS {
            return Err(Error::limit("An icon has at most 1024 paths."));
        }
        if attr(tag, "fill") == Some("none") {
            continue;
        }
        let Some(d) = attr(tag, "d") else { continue };
        let op = attr(tag, "opacity")
            .and_then(|o| numbers(o).first().copied())
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);
        acc.reset(n, n);
        {
            let mut f = Flattener::new(&mut acc, s, -s, ox, oy);
            path(d, &mut f)?;
            f.close();
        }
        acc.finish(&mut cov);
        let k = trunc_u32(op * 255.0 + 0.5);
        for (a, c) in alpha.iter_mut().zip(&cov) {
            let src = div255(u32::from(*c) * k);
            *a = to_u8(src + div255(u32::from(*a) * (255 - src)));
        }
    }
    let mut out = Vec::with_capacity(n * n * 4);
    for a in alpha {
        let a = div255(u32::from(a) * u32::from(color.a));
        let p = Color::rgba(color.r, color.g, color.b, to_u8(a)).premul();
        out.extend_from_slice(&p);
    }
    Ok(Level {
        w: px,
        h: px,
        px: out,
    })
}

struct PathReader<'a> {
    b: &'a [u8],
    i: usize,
}

impl PathReader<'_> {
    fn skip(&mut self) {
        while self
            .b
            .get(self.i)
            .is_some_and(|c| c.is_ascii_whitespace() || *c == b',')
        {
            self.i += 1;
        }
    }

    fn command(&mut self) -> Option<u8> {
        self.skip();
        let c = *self.b.get(self.i)?;
        if c.is_ascii_alphabetic() && c != b'e' && c != b'E' {
            self.i += 1;
            Some(c)
        } else {
            None
        }
    }

    fn number(&mut self) -> Option<f32> {
        self.skip();
        let start = self.i;
        let mut j = self.i;
        if matches!(self.b.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        let mut digits = false;
        let mut dot = false;
        while let Some(c) = self.b.get(j) {
            if c.is_ascii_digit() {
                digits = true;
            } else if *c == b'.' && !dot {
                dot = true;
            } else {
                break;
            }
            j += 1;
        }
        if !digits {
            return None;
        }
        if matches!(self.b.get(j), Some(b'e' | b'E')) {
            let mut k = j + 1;
            if matches!(self.b.get(k), Some(b'+' | b'-')) {
                k += 1;
            }
            if self.b.get(k).is_some_and(u8::is_ascii_digit) {
                while self.b.get(k).is_some_and(u8::is_ascii_digit) {
                    k += 1;
                }
                j = k;
            }
        }
        let v = std::str::from_utf8(&self.b[start..j])
            .ok()?
            .parse::<f32>()
            .ok()?;
        self.i = j;
        v.is_finite().then_some(v)
    }

    /// An arc flag: one `0` or `1`, which may touch the next number.
    fn flag(&mut self) -> Option<bool> {
        self.skip();
        let c = *self.b.get(self.i)?;
        self.i += 1;
        match c {
            b'0' => Some(false),
            b'1' => Some(true),
            _ => None,
        }
    }

    fn at_number(&mut self) -> bool {
        self.skip();
        self.b
            .get(self.i)
            .is_some_and(|c| c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.'))
    }
}

fn num(r: &mut PathReader<'_>) -> Result<f32> {
    r.number().ok_or_else(|| Error::invalid(BAD_SVG))
}

/// Feeds the path data `d` to `f`.
fn path(d: &str, f: &mut Flattener<'_>) -> Result<()> {
    let bad = || Error::invalid(BAD_SVG);
    let mut r = PathReader {
        b: d.as_bytes(),
        i: 0,
    };
    let (mut cx, mut cy) = (0.0f32, 0.0f32);
    let (mut sx, mut sy) = (0.0f32, 0.0f32);
    // The last control point, for the smooth curves (S, T).
    let mut last_c: Option<(f32, f32)> = None;
    let mut last_q: Option<(f32, f32)> = None;
    let mut count = 0;
    let mut cmd = r.command().ok_or_else(bad)?;
    loop {
        count += 1;
        if count > MAX_COMMANDS {
            return Err(Error::limit("An icon path has at most 65536 commands."));
        }
        let rel = cmd.is_ascii_lowercase();
        let (bx, by) = if rel { (cx, cy) } else { (0.0, 0.0) };
        let (mut nc, mut nq) = (None, None);
        match cmd.to_ascii_uppercase() {
            b'M' => {
                let (x, y) = (num(&mut r)? + bx, num(&mut r)? + by);
                f.move_to(x, y);
                (cx, cy, sx, sy) = (x, y, x, y);
                // Later pairs are lines.
                cmd = if rel { b'l' } else { b'L' };
            }
            b'L' => {
                let (x, y) = (num(&mut r)? + bx, num(&mut r)? + by);
                f.line_to(x, y);
                (cx, cy) = (x, y);
            }
            b'H' => {
                cx = num(&mut r)? + bx;
                f.line_to(cx, cy);
            }
            b'V' => {
                cy = num(&mut r)? + by;
                f.line_to(cx, cy);
            }
            b'C' | b'S' => {
                let (x1, y1) = if cmd.eq_ignore_ascii_case(&b'S') {
                    last_c.map_or((cx, cy), |(px, py)| (2.0 * cx - px, 2.0 * cy - py))
                } else {
                    (num(&mut r)? + bx, num(&mut r)? + by)
                };
                let (x2, y2) = (num(&mut r)? + bx, num(&mut r)? + by);
                let (x, y) = (num(&mut r)? + bx, num(&mut r)? + by);
                f.curve_to(x1, y1, x2, y2, x, y);
                nc = Some((x2, y2));
                (cx, cy) = (x, y);
            }
            b'Q' | b'T' => {
                let (x1, y1) = if cmd.eq_ignore_ascii_case(&b'T') {
                    last_q.map_or((cx, cy), |(px, py)| (2.0 * cx - px, 2.0 * cy - py))
                } else {
                    (num(&mut r)? + bx, num(&mut r)? + by)
                };
                let (x, y) = (num(&mut r)? + bx, num(&mut r)? + by);
                f.quad_to(x1, y1, x, y);
                nq = Some((x1, y1));
                (cx, cy) = (x, y);
            }
            b'A' => {
                let (rx, ry, rot) = (num(&mut r)?.abs(), num(&mut r)?.abs(), num(&mut r)?);
                let large = r.flag().ok_or_else(bad)?;
                let sweep = r.flag().ok_or_else(bad)?;
                let (x, y) = (num(&mut r)? + bx, num(&mut r)? + by);
                arc(f, (cx, cy), (rx, ry, rot), (large, sweep), (x, y));
                (cx, cy) = (x, y);
            }
            b'Z' => {
                f.close();
                (cx, cy) = (sx, sy);
                f.move_to(cx, cy);
            }
            _ => return Err(bad()),
        }
        last_c = nc;
        last_q = nq;
        if cmd.eq_ignore_ascii_case(&b'Z') || !r.at_number() {
            match r.command() {
                Some(c) => cmd = c,
                None if r.i >= r.b.len() => return Ok(()),
                None => return Err(bad()),
            }
        }
    }
}

/// An elliptical arc (SVG implementation notes, F.6.5) as cubic curves, with square roots only:
/// the angle is split by bisecting on the unit circle until each piece is at most a quarter
/// turn, and each piece's control distance comes from half-angle formulas. A rotated ellipse
/// (no symbolic icon has one) is drawn unrotated.
fn arc(
    f: &mut Flattener<'_>,
    p0: (f32, f32),
    (rx, ry, _rot): (f32, f32, f32),
    (large, sweep): (bool, bool),
    p1: (f32, f32),
) {
    if p0 == p1 {
        return;
    }
    if rx <= 0.0 || ry <= 0.0 {
        f.line_to(p1.0, p1.1);
        return;
    }
    let (hx, hy) = ((p0.0 - p1.0) / 2.0, (p0.1 - p1.1) / 2.0);
    let lambda = (hx * hx) / (rx * rx) + (hy * hy) / (ry * ry);
    let (rx, ry) = if lambda > 1.0 {
        let k = lambda.sqrt();
        (rx * k, ry * k)
    } else {
        (rx, ry)
    };
    let num = rx * rx * ry * ry - rx * rx * hy * hy - ry * ry * hx * hx;
    let den = rx * rx * hy * hy + ry * ry * hx * hx;
    let mut coef = if den > 0.0 {
        (num / den).max(0.0).sqrt()
    } else {
        0.0
    };
    if large == sweep {
        coef = -coef;
    }
    let (ccx, ccy) = (coef * rx * hy / ry, -coef * ry * hx / rx);
    let (cx, cy) = (ccx + (p0.0 + p1.0) / 2.0, ccy + (p0.1 + p1.1) / 2.0);
    let u = ((hx - ccx) / rx, (hy - ccy) / ry);
    let v = ((-hx - ccx) / rx, (-hy - ccy) / ry);
    let norm = |p: (f32, f32)| {
        let l = (p.0 * p.0 + p.1 * p.1).sqrt();
        if l > 0.0 {
            (p.0 / l, p.1 / l)
        } else {
            (1.0, 0.0)
        }
    };
    let mut out = |a: (f32, f32), b: (f32, f32)| {
        // A piece of at most a quarter turn from a to b on the unit circle.
        let c = (a.0 * b.0 + a.1 * b.1).clamp(-1.0, 1.0);
        let c2 = ((1.0 + c) / 2.0).sqrt();
        let s2 = ((1.0 - c) / 2.0).sqrt();
        let k = 4.0 / 3.0 * s2 / (1.0 + c2);
        let t = |p: (f32, f32)| if sweep { (-p.1, p.0) } else { (p.1, -p.0) };
        let (ta, tb) = (t(a), t(b));
        let c1 = (a.0 + k * ta.0, a.1 + k * ta.1);
        let c2p = (b.0 - k * tb.0, b.1 - k * tb.1);
        let m = |p: (f32, f32)| (cx + rx * p.0, cy + ry * p.1);
        let (q1, q2, q3) = (m(c1), m(c2p), m(b));
        f.curve_to(q1.0, q1.1, q2.0, q2.1, q3.0, q3.1);
    };
    split(norm(u), norm(v), sweep, 0, &mut out);
    // Land exactly on the end point.
    f.line_to(p1.0, p1.1);
}

/// Bisects the arc from `a` to `b` (unit vectors, going the `sweep` way) until each piece is at
/// most a quarter turn, and hands the pieces to `out` in order.
fn split(
    a: (f32, f32),
    b: (f32, f32),
    sweep: bool,
    depth: u32,
    out: &mut impl FnMut((f32, f32), (f32, f32)),
) {
    let dot = a.0 * b.0 + a.1 * b.1;
    let cross = a.0 * b.1 - a.1 * b.0;
    let short = (cross > 0.0) == sweep;
    if depth >= 6 || (dot >= 0.0 && (short || cross.abs() < 1e-6)) {
        out(a, b);
        return;
    }
    let (sx, sy) = (a.0 + b.0, a.1 + b.1);
    let l = (sx * sx + sy * sy).sqrt();
    let m = if l < 1e-6 {
        // Half a turn: the middle is a quarter turn from `a` the sweep way.
        if sweep { (-a.1, a.0) } else { (a.1, -a.0) }
    } else if short {
        (sx / l, sy / l)
    } else {
        (-sx / l, -sy / l)
    };
    split(a, m, sweep, depth + 1, out);
    split(m, b, sweep, depth + 1, out);
}

#[cfg(test)]
mod tests {
    use super::*;

    const RING: &str = concat!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" width="16" height="16"><g fill="currentColor">"#,
        r#"<path d="M12 8A4 4 0 0 1 4 8A4 4 0 0 1 12 8ZM10 8A2 2 0 0 0 6 8A2 2 0 0 0 10 8Z"/>"#,
        r#"<path opacity="0.5" d="M0 0H2V2H0Z"/><path fill="none" d="M14 14L16 14L16 16Z"/></g></svg>"#
    );

    fn alpha(l: &Level, x: u32, y: u32) -> u8 {
        l.texel(i32::try_from(x).unwrap(), i32::try_from(y).unwrap())[3]
    }

    #[test]
    fn a_ring_icon_fills_the_band_and_leaves_the_hole() {
        let l = raster(RING, 16, Color::rgb(10, 20, 30)).unwrap();
        assert_eq!((l.w, l.h), (16, 16));
        assert_eq!(alpha(&l, 2, 8), 0, "outside the ring");
        assert!(alpha(&l, 4, 7) > 200 || alpha(&l, 5, 7) > 200, "the band");
        assert_eq!(alpha(&l, 8, 8), 0, "the hole");
        let t = l.texel(5, 7);
        assert!(t[0] <= t[3] && t[2] <= t[3], "premultiplied {t:?}");
        assert_eq!(alpha(&l, 0, 0), 128, "opacity 0.5");
        assert_eq!(alpha(&l, 15, 15), 0, "fill none");
    }

    #[test]
    fn the_same_icon_is_the_same_bytes_and_scales() {
        let a = raster(RING, 32, Color::rgb(0, 0, 0)).unwrap();
        let b = raster(RING, 32, Color::rgb(0, 0, 0)).unwrap();
        assert_eq!(a, b);
        assert!(alpha(&a, 9, 15) > 200);
    }

    #[test]
    fn broken_icons_are_refused_not_panicked() {
        assert!(raster("<svg>", 16, Color::rgb(0, 0, 0)).is_err());
        assert!(
            raster(
                r#"<svg viewBox="0 0 16 16"><path d="M1 1 L"/></svg>"#,
                16,
                Color::rgb(0, 0, 0)
            )
            .is_err()
        );
        assert!(
            raster(
                r#"<svg viewBox="0 0 16 16"><path d="X1"/></svg>"#,
                16,
                Color::rgb(0, 0, 0)
            )
            .is_err()
        );
        let ok = raster(
            r#"<svg viewBox="0 0 16 16"><path d="M1 1h4v4h-4z m6 6 l2 0 0 2z"/></svg>"#,
            16,
            Color::rgb(0, 0, 0),
        );
        assert!(ok.is_ok());
        assert!(
            raster(
                r#"<svg viewBox="0 0 16 16"><path d="M2 8a6 6 0 1 1 12 0a6 6 0 1 1-12 0z"/></svg>"#,
                16,
                Color::rgb(0, 0, 0)
            )
            .is_ok()
        );
    }

    #[test]
    fn short_names_map_to_theme_names() {
        assert_eq!(candidates("back")[0], "go-previous-symbolic.svg");
        assert_eq!(candidates("zoom_in")[0], "zoom-in-symbolic.svg");
        assert_eq!(candidates("folder-new")[3], "folder-new.png");
        assert!(!ok_name("../etc/passwd") && !ok_name("") && ok_name("zoom_in"));
        assert!(is_icon("icon:trash") && !is_icon("/a.png"));
    }

    #[test]
    fn icon_folders_follow_the_environment() {
        let set = |k: &str| match k {
            "WOLKIT_ICON_DIR" => Some("/ci/icons".into()),
            _ => None,
        };
        assert_eq!(dirs_for(false, set), vec![PathBuf::from("/ci/icons")]);
        let none = |_: &str| None;
        let h = dirs_for(true, none);
        assert_eq!(
            h[0],
            PathBuf::from("/usr/share/icons/WolOS/symbolic/actions")
        );
        let home = |k: &str| (k == "HOME").then(|| "/h".into());
        let d = dirs_for(false, home);
        assert_eq!(
            d[0],
            PathBuf::from("/h/.local/share/icons/WolOS/symbolic/actions")
        );
        assert!(d.iter().any(|p| p.starts_with("/usr/share/icons/hicolor")));
    }
}

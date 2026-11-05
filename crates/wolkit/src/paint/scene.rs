// SPDX-License-Identifier: BSD-2-Clause
//! The scene as a display list: each node's resolved style (backgrounds, borders, radii, shadows,
//! opacity, focus rings, clips, scroll offsets) and its text, shaped through the window's
//! paragraph cache, with colours resolved against the live token [`Palette`]. Overlays become
//! their own chunks above the window. The CPU rasteriser draws the result for the headless
//! backend; the native backend hands the same list to the GPU batcher.

use crate::image::fit::Fit;
use crate::num::sat_u8;
use crate::paint::geom::Rect;
use crate::paint::list::{DisplayList, Fill, Radii, Shadow};
use crate::paint::palette::Palette;
use crate::paint::text::{push_layout, push_selection};
use crate::scene::prop::id;
use crate::scene::{Idx, Kind, MAX_DEPTH, NIL, SceneRead, state};
use crate::style::Styles;
use crate::style::tokens::Token;
use crate::style::value::{ColorRef, Display, Overflow, Paint, Props, Visibility, WhiteSpace};
use crate::text::area::caret_pos;
use crate::text::measure::TextCache;
use crate::text::system::TextSystem;

/// What lowering reads and the text caches it fills.
pub struct Lower<'a, S: SceneRead + ?Sized = dyn SceneRead + 'a> {
    pub scene: &'a S,
    pub styles: &'a Styles,
    pub palette: &'a Palette,
    pub text: &'a mut TextSystem,
    pub cache: &'a mut TextCache,
}

/// The drawings of canvas nodes (`wolkit_window_canvas_submit`), by slot, each with the full id
/// it was submitted for (a reused slot does not draw a stale canvas).
pub type Canvases =
    std::collections::HashMap<Idx, (crate::scene::NodeId, crate::gpu::canvas::Canvas)>;

/// Fill `list` (cleared first) with the scene: the window as chunk 0, then one chunk per overlay
/// in tree order above it.
pub fn build<S: SceneRead + ?Sized>(cx: &mut Lower<'_, S>, list: &mut DisplayList) {
    build_with(cx, None, list);
}

/// [`build`], drawing each canvas node's submitted drawing inside its rectangle (in tree order,
/// so later siblings and overlays stay on top).
pub fn build_with<S: SceneRead + ?Sized>(
    cx: &mut Lower<'_, S>,
    canvases: Option<&Canvases>,
    list: &mut DisplayList,
) {
    list.clear();
    if !cx.scene.is_live(0) {
        list.finish();
        return;
    }
    // A picture that finished decoding changes what a chunk shows without a patch.
    let version = cx
        .scene
        .version()
        .wrapping_add(crate::image::generation() << 32);
    let mut overlays = Vec::new();
    list.begin_chunk(0, version, to_paint(cx.scene.rect(0)), (0.0, 0.0), 0);
    node(cx, canvases, list, 0, (0.0, 0.0), 0, &mut overlays);
    let mut z = 1;
    let mut i = 0;
    // Overlays found inside overlays are appended while this runs, and drawn above their parent.
    while i < overlays.len() {
        let (n, off) = overlays[i];
        list.begin_chunk(
            cx.scene.id(n).0,
            version,
            to_paint(cx.scene.rect(n)),
            (0.0, 0.0),
            z,
        );
        let mut nested = Vec::new();
        paint_tree(cx, canvases, list, n, off, 0, &mut nested);
        overlays.extend(nested);
        z += 1;
        i += 1;
    }
    list.finish();
}

fn to_paint(r: crate::scene::Rect) -> Rect {
    Rect::new(r.x, r.y, r.w, r.h)
}

/// A node met while walking a chunk: an overlay is deferred to its own chunk.
fn node<S: SceneRead + ?Sized>(
    cx: &mut Lower<'_, S>,
    canvases: Option<&Canvases>,
    list: &mut DisplayList,
    n: Idx,
    off: (f32, f32),
    depth: usize,
    overlays: &mut Vec<(Idx, (f32, f32))>,
) {
    if n != 0 && cx.scene.kind(n) == Kind::Overlay {
        overlays.push((n, off));
        return;
    }
    paint_tree(cx, canvases, list, n, off, depth, overlays);
}

/// Node `n` and its subtree, translated by `off` (scroll offsets and transforms above it).
fn paint_tree<S: SceneRead + ?Sized>(
    cx: &mut Lower<'_, S>,
    canvases: Option<&Canvases>,
    list: &mut DisplayList,
    n: Idx,
    off: (f32, f32),
    depth: usize,
    overlays: &mut Vec<(Idx, (f32, f32))>,
) {
    if depth > MAX_DEPTH {
        return;
    }
    let st = cx.styles.get(cx.scene.computed(n));
    if st.layout.display == Display::None {
        return;
    }
    let t = st.paint.transform;
    let off = (off.0 + t.tx, off.1 + t.ty);
    let r = to_paint(cx.scene.rect(n)).translate(off.0, off.1);
    let alpha = sat_u8((st.paint.opacity.clamp(0.0, 1.0) * 255.0).round());
    if alpha == 0 {
        return;
    }
    let layer = alpha < 255;
    if layer {
        list.push_opacity(alpha, overlaps(cx, n, st));
    }
    let radii = Radii(st.paint.radius).fitted(r.w, r.h);
    if st.paint.visibility == Visibility::Visible {
        decorate(cx, list, n, st, r, radii);
    }
    let kind = cx.scene.kind(n);
    if kind == Kind::Canvas
        && st.paint.visibility == Visibility::Visible
        && let Some((id, c)) = canvases.and_then(|m| m.get(&n))
        && *id == cx.scene.id(n)
    {
        c.emit(r, list);
    }
    let scrolls = matches!(kind, Kind::Scroll | Kind::Vlist);
    let clips = scrolls || st.layout.overflow != Overflow::Visible;
    if cx.scene.first_child(n) != NIL {
        if clips {
            list.push_clip(r, radii.max());
        }
        let scroll = if scrolls {
            cx.scene.prop(n, id::SCROLL_OFFSET).map_or((0.0, 0.0), |v| {
                (f32::from_bits(v.w[0]), f32::from_bits(v.w[1]))
            })
        } else {
            (0.0, 0.0)
        };
        let inner = (off.0 - scroll.0, off.1 - scroll.1);
        let mut c = cx.scene.first_child(n);
        while c != NIL {
            node(cx, canvases, list, c, inner, depth + 1, overlays);
            c = cx.scene.next_sibling(c);
        }
        if clips {
            list.pop_clip();
        }
    }
    if st.paint.visibility == Visibility::Visible {
        focus_ring(cx, list, n, st, r, radii);
    }
    if layer {
        list.pop_opacity();
    }
}

/// Shadows, background, the look of a control, border and text of one node.
fn decorate<S: SceneRead + ?Sized>(
    cx: &mut Lower<'_, S>,
    list: &mut DisplayList,
    n: Idx,
    st: &Props,
    r: Rect,
    radii: Radii,
) {
    let pal = cx.palette;
    for sh in &st.paint.shadow {
        let visible = sh.blur > 0.0 || sh.dx != 0.0 || sh.dy != 0.0;
        if sh.color != ColorRef::Transparent && visible {
            let c = pal.resolve(sh.color);
            list.shadow(r, radii, Shadow::new(sh.blur, 0.0, sh.dx, sh.dy, c));
        }
    }
    if let Some(fill) = fill(pal, &st.paint.background) {
        list.rect(r, radii, fill);
    }
    let kind = cx.scene.kind(n);
    match kind {
        Kind::Divider if st.paint.background == Paint::None => {
            list.solid(r, pal.token(Token::Line));
        }
        Kind::Toggle => toggle(list, pal, cx.scene.state(n) & state::CHECKED != 0, r),
        Kind::Slider => slider(cx, list, n, r),
        Kind::Image => image(cx, list, n, r, radii),
        _ => {}
    }
    let bw = st.layout.border_width;
    if bw > 0.0 {
        list.border(r, radii, bw, pal.resolve(st.paint.border_color));
    }
    text(cx, list, n, kind, st, r);
}

/// Whether a translucent node needs a layer: anything with children or more than one drawing.
/// A bare picture (no background, border, shadow or focus ring) folds its opacity into the
/// picture instead, which keeps a faded thumbnail grid free of offscreen passes.
fn overlaps<S: SceneRead + ?Sized>(cx: &Lower<'_, S>, n: Idx, st: &Props) -> bool {
    let bare = cx.scene.kind(n) == Kind::Image
        && cx.scene.first_child(n) == NIL
        && st.paint.background == Paint::None
        && !st.paint.shadow.iter().any(|sh| {
            sh.color != ColorRef::Transparent && (sh.blur > 0.0 || sh.dx != 0.0 || sh.dy != 0.0)
        })
        && st.layout.border_width <= 0.0
        && (st.paint.outline_width <= 0.0 || cx.scene.state(n) & state::FOCUS_VISIBLE == 0);
    !bare
}

/// The `icon:<name>` source of an `image` node without an `image-id`, if it has one.
fn icon_source<S: SceneRead + ?Sized>(scene: &S, n: Idx) -> Option<&str> {
    if scene.prop(n, id::IMAGE_ID).is_some_and(|v| v.w[0] != 0) {
        return None;
    }
    let src = scene
        .prop(n, id::IMAGE_SOURCE)
        .map(|v| scene.string(v.w[0]))?;
    crate::image::icon::is_icon(src).then_some(src)
}

/// The picture an `image` node shows: `image-id` (a registered picture) first, else an
/// `image-source` path. `None` for a node with neither, or a source that is not a file.
fn picture<S: SceneRead + ?Sized>(
    scene: &S,
    n: Idx,
    r: Rect,
) -> Option<std::sync::Arc<crate::image::Entry>> {
    if let Some(v) = scene.prop(n, id::IMAGE_ID)
        && v.w[0] != 0
    {
        return crate::image::lookup(v.w[0]);
    }
    let src = scene
        .prop(n, id::IMAGE_SOURCE)
        .map(|v| scene.string(v.w[0]))?;
    crate::image::source::get(src, r.w, r.h)
}

/// Asks for the picture of every `image` node (paint does this lazily as it draws; the headless
/// backend calls this, then waits, so its first frame already shows the pictures).
pub fn request_images<S: SceneRead + ?Sized>(
    scene: &S,
    styles: &Styles,
    palette: &Palette,
) -> usize {
    let mut n = 0;
    if !scene.is_live(0) {
        return 0;
    }
    let mut stack = vec![0];
    while let Some(i) = stack.pop() {
        let r = to_paint(scene.rect(i));
        let asked = match (scene.kind(i), icon_source(scene, i)) {
            (Kind::Image, Some(src)) => {
                let color = palette.resolve(styles.get(scene.computed(i)).text.color);
                crate::image::icon::get(src, r.w, r.h, color).is_some()
            }
            (Kind::Image, None) => picture(scene, i, r).is_some(),
            _ => false,
        };
        if asked {
            n += 1;
        }
        let mut c = scene.first_child(i);
        while c != NIL && stack.len() < 1 << 16 {
            stack.push(c);
            c = scene.next_sibling(c);
        }
    }
    n
}

/// An `image` node: the picture placed by `image-fit` and cut to the node's rounded box, or a
/// placeholder fill while it is decoding (and when it could not be read).
fn image<S: SceneRead + ?Sized>(
    cx: &Lower<'_, S>,
    list: &mut DisplayList,
    n: Idx,
    r: Rect,
    radii: Radii,
) {
    let icon = icon_source(cx.scene, n);
    let entry = match icon {
        Some(src) => {
            let color = cx
                .palette
                .resolve(cx.styles.get(cx.scene.computed(n)).text.color);
            crate::image::icon::get(src, r.w, r.h, color)
        }
        None => picture(cx.scene, n, r),
    };
    let Some(entry) = entry else {
        return;
    };
    let Some(px) = entry.pixels() else {
        // An icon that is not drawn yet (or not found) shows nothing; a picture a placeholder.
        if icon.is_none() {
            list.rect(r, radii, Fill::Solid(cx.palette.token(Token::Control)));
        }
        return;
    };
    let fit = cx
        .scene
        .prop(n, id::IMAGE_FIT)
        .map_or(Fit::Contain, |v| Fit::from_index(v.w[0]));
    let dest = crate::image::fit::dest(fit, r, px.width, px.height);
    let shown = dest.intersect(&r);
    let round = !radii.is_zero();
    if round {
        list.push_clip(r, radii.max());
    }
    // The node's opacity is a layer, or folded in by the opacity stack (`overlaps`).
    list.image(shown, dest, crate::image::ImageRef(px), 255);
    if round {
        list.pop_clip();
    }
}

/// What `background` fills with. A three-stop gradient draws its first and last stops (the
/// display list's gradient has two).
fn fill(pal: &Palette, p: &Paint) -> Option<Fill> {
    match *p {
        Paint::None => None,
        Paint::Color(c) => Some(Fill::Solid(pal.resolve(c))),
        Paint::Linear { angle, stops, n } => {
            let last = usize::from(n.clamp(2, 3)) - 1;
            // CSS angles: 0 points up, 90 right, clockwise.
            let a = f32::from(angle).to_radians();
            let (dx, dy) = (a.sin() * 0.5, -a.cos() * 0.5);
            Some(Fill::Linear {
                from: pal.resolve(stops[0]),
                to: pal.resolve(stops[last]),
                p0: (0.5 - dx, 0.5 - dy),
                p1: (0.5 + dx, 0.5 + dy),
            })
        }
    }
}

fn toggle(list: &mut DisplayList, pal: &Palette, on: bool, r: Rect) {
    let track = if on { Token::Accent } else { Token::Control };
    list.rect(r, Radii::uniform(r.h / 2.0), Fill::Solid(pal.token(track)));
    let d = (r.h - 4.0).max(0.0);
    let x = if on { r.right() - 2.0 - d } else { r.x + 2.0 };
    list.rect(
        Rect::new(x, r.y + 2.0, d, d),
        Radii::uniform(d / 2.0),
        Fill::Solid(pal.token(Token::Card)),
    );
}

fn slider<S: SceneRead + ?Sized>(cx: &Lower<'_, S>, list: &mut DisplayList, n: Idx, r: Rect) {
    let f = |p: u16, d: f32| cx.scene.prop(n, p).map_or(d, |v| v.f32());
    let (min, max) = (f(id::SLIDER_MIN, 0.0), f(id::SLIDER_MAX, 1.0));
    let (min, max) = if max > min { (min, max) } else { (0.0, 1.0) };
    let k = ((f(id::SLIDER_VALUE, min) - min) / (max - min)).clamp(0.0, 1.0);
    let pal = cx.palette;
    let knob = r.h.min(16.0);
    let track = Rect::new(
        r.x + knob / 2.0,
        r.y + (r.h - 4.0) / 2.0,
        (r.w - knob).max(0.0),
        4.0,
    );
    list.rect(
        track,
        Radii::uniform(2.0),
        Fill::Solid(pal.token(Token::Control)),
    );
    let done = Rect::new(track.x, track.y, track.w * k, track.h);
    list.rect(
        done,
        Radii::uniform(2.0),
        Fill::Solid(pal.token(Token::Accent)),
    );
    let kx = track.x + track.w * k - knob / 2.0;
    let kr = Rect::new(kx, r.y + (r.h - knob) / 2.0, knob, knob);
    list.rect(
        kr,
        Radii::uniform(knob / 2.0),
        Fill::Solid(pal.token(Token::Card)),
    );
    list.border(kr, Radii::uniform(knob / 2.0), 1.0, pal.token(Token::Line));
}

/// The text a node shows and the colour it shows it in, if it shows any.
fn label<'s, S: SceneRead + ?Sized>(
    scene: &'s S,
    n: Idx,
    kind: Kind,
    st: &Props,
) -> Option<(std::borrow::Cow<'s, str>, ColorRef)> {
    let t = scene.text(n);
    match kind {
        Kind::TextInput if t.is_empty() => {
            let p = scene
                .prop(n, id::PLACEHOLDER)
                .map(|v| scene.string(v.w[0]))?;
            (!p.is_empty()).then(|| (p.into(), ColorRef::Token(Token::Dim)))
        }
        Kind::TextInput if scene.prop(n, id::SECURE).is_some_and(|v| v.w[0] == 1) => {
            Some(("\u{2022}".repeat(t.chars().count()).into(), st.text.color))
        }
        Kind::Text | Kind::TextInput => Some((t.into(), st.text.color)),
        Kind::Button | Kind::Box if scene.first_child(n) == NIL && !t.is_empty() => {
            Some((t.into(), st.text.color))
        }
        _ => None,
    }
}

fn text<S: SceneRead + ?Sized>(
    cx: &mut Lower<'_, S>,
    list: &mut DisplayList,
    n: Idx,
    kind: Kind,
    st: &Props,
    r: Rect,
) {
    // A text field the input layer has a view of: while it is being edited it draws the text with
    // the input method's preedit in it, the selection and the caret; scrolled, it draws shifted.
    let view = (kind == Kind::TextInput)
        .then(|| cx.scene.edit_view(n))
        .flatten();
    // A view whose text the app has since replaced is stale: the node's own text wins.
    let active = view.filter(|v| v.active && v.base == cx.scene.text(n));
    let (mut s, mut color) = match label(cx.scene, n, kind, st) {
        Some(l) => l,
        None if active.is_some() => ("".into(), st.text.color),
        None => return,
    };
    let secure =
        kind == Kind::TextInput && cx.scene.prop(n, id::SECURE).is_some_and(|v| v.w[0] == 1);
    // What byte offsets of the view mean in the string that is drawn (a secure field draws one
    // bullet, three bytes, per character).
    let mut map: Box<dyn Fn(usize) -> usize> = Box::new(|i| i);
    if let Some(v) = active
        && !v.text.is_empty()
    {
        color = st.text.color;
        if secure {
            let t = v.text.clone();
            s = "\u{2022}".repeat(t.chars().count()).into();
            map = Box::new(move |i| t[..i.min(t.len())].chars().count() * 3);
        } else {
            s = v.text.as_str().into();
        }
    }
    let editing_empty = active.is_some_and(|v| v.text.is_empty());
    if s.is_empty() && !editing_empty {
        return;
    }
    let bw = st.layout.border_width;
    let [pt, pr, pb, pl] = st.layout.padding;
    let content = Rect::new(
        r.x + pl + bw,
        r.y + pt + bw,
        (r.w - pl - pr - 2.0 * bw).max(0.0),
        (r.h - pt - pb - 2.0 * bw).max(0.0),
    );
    // Half a pixel of slack: the measure rounded the width up, rounding must not rewrap.
    // A text field is one line unless its white space is `pre-wrap`: then it is a text area.
    let area = kind == Kind::TextInput && st.text.white_space == WhiteSpace::PreWrap;
    let one_line = kind == Kind::TextInput && !area;
    let max_w = (!one_line).then_some(content.w + 0.5);
    let layout = cx.cache.layout(cx.text, &s, &st.text, max_w);
    // Controls centre their label; text sits at the top of its box.
    let centred = matches!(kind, Kind::Button) || one_line;
    let dy = if centred {
        ((content.h - layout.height) / 2.0).max(0.0)
    } else {
        0.0
    };
    // The scroll of a view made for other text means nothing now (the app replaced the text, as a
    // chat box does after Send), and a scroll past the end of this layout is brought back to it.
    let scroll = view
        .filter(|v| v.base == cx.scene.text(n))
        .map_or((0.0, 0.0), |v| {
            crate::text::area::clamp_scroll(
                v.scroll,
                (layout.width + 1.0, layout.height),
                (content.w, content.h),
            )
        });
    let (ox, oy) = (content.x - scroll.0, content.y + dy - scroll.1);
    let overflows = layout.width > content.w + 0.5 || layout.height > content.h + 0.5;
    let clipped = overflows || kind == Kind::TextInput;
    if clipped {
        list.push_clip(content, 0.0);
    }
    if let Some(v) = active {
        let sel = map(v.selection.start)..map(v.selection.end);
        let accent = cx
            .palette
            .token(Token::Accent)
            .with_opacity(SELECTION_ALPHA);
        push_selection(list, &layout.selection_rects(sel), ox, oy, accent);
    }
    push_layout(list, &layout, ox, oy, cx.palette.resolve(color));
    if let Some(v) = active {
        let ink = cx.palette.resolve(st.text.color);
        // The preedit is underlined: it is not text yet.
        if let Some(p) = &v.preedit {
            for u in layout.selection_rects(map(p.start)..map(p.end)) {
                list.solid(
                    Rect::new(u.x, u.y + u.h - 1.0, u.w, 1.0).translate(ox, oy),
                    ink,
                );
            }
        }
        if v.selection.is_empty() {
            let c = caret_pos(&layout, &s, map(v.caret));
            list.solid(
                Rect::new(c.x.round(), c.top, CARET_WIDTH, c.height).translate(ox, oy),
                ink,
            );
        }
    }
    if clipped {
        list.pop_clip();
    }
}

/// The selection is the accent colour at this opacity (of 255).
pub const SELECTION_ALPHA: u8 = 96;
/// The caret is a bar this wide, in logical pixels.
pub const CARET_WIDTH: f32 = 1.0;

/// The focus ring (`outline`) of a node that has keyboard focus, outside its border box.
fn focus_ring<S: SceneRead + ?Sized>(
    cx: &Lower<'_, S>,
    list: &mut DisplayList,
    n: Idx,
    st: &Props,
    r: Rect,
    radii: Radii,
) {
    let w = st.paint.outline_width;
    if w <= 0.0 || cx.scene.state(n) & state::FOCUS_VISIBLE == 0 {
        return;
    }
    let ring = Radii(radii.0.map(|v| v + w));
    list.border(
        r.inflate(w),
        ring,
        w,
        cx.palette.resolve(st.paint.outline_color),
    );
}

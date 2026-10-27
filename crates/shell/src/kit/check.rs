//! Layout check over live GTK allocations (rules: `crate::layout`). `WOL_LAYOUT_CHECK=exit` exits 1 on issues.
use crate::layout::{self, Node, Rect};
use gtk::{glib, prelude::*};

fn name(w: &gtk::Widget) -> String {
    let mut s = w.type_().name().trim_start_matches("Gtk").to_string();
    for c in w.css_classes() {
        s.push('.');
        s.push_str(&c);
    }
    if let Some(l) = w.downcast_ref::<gtk::Label>() {
        s.push_str(&format!(" {:?}", l.text().as_str()));
    }
    s
}

fn bounds(w: &gtk::Widget, root: &gtk::Widget) -> Rect {
    w.compute_bounds(root)
        .map(|b| Rect {
            x: b.x().round() as i32,
            y: b.y().round() as i32,
            w: b.width().round() as i32,
            h: b.height().round() as i32,
        })
        .unwrap_or_default()
}

fn shown(c: &gtk::Widget) -> bool {
    c.is_visible() && c.is_child_visible() && !c.is::<gtk::Popover>() && c.width() > 0
}

/// Kit classes that only lay out (no background, border or ink of their own).
const LAYOUT_ONLY: [&str; 8] = [
    "horizontal",
    "vertical",
    "no-row-check",
    "control",
    "device-row",
    "detail-head",
    "detail-page",
    "detail-status",
];
/// Kit buttons that draw nothing at rest and are not a row of their own: measured through.
/// Row buttons with a hover pill (device rows, "… Settings" links) and icon buttons count as their
/// box: that is the block the eye reads.
const FLAT_BUTTONS: [&str; 2] = ["control-detail", "row-button"];

/// True when `w` draws nothing itself, so a container's inset is measured through it.
fn layout_only(w: &gtk::Widget) -> bool {
    let wrapper = w.is::<gtk::Box>()
        || w.is::<gtk::Stack>()
        || w.is::<gtk::ScrolledWindow>()
        || w.is::<gtk::Viewport>()
        || w.is::<gtk::Revealer>()
        || (w.is::<gtk::Overlay>() && w.css_classes().is_empty());
    let flat = w.is::<gtk::Button>() && FLAT_BUTTONS.iter().any(|c| w.has_css_class(c));
    flat || (wrapper
        && w.css_classes()
            .iter()
            .all(|c| LAYOUT_ONLY.contains(&c.as_str())))
}

/// An icon draws its `pixel_size` square centred in a possibly larger allocation.
fn glyph(w: &gtk::Widget, r: Rect) -> Rect {
    let Some(px) = w
        .downcast_ref::<gtk::Image>()
        .map(|i| i.pixel_size())
        .filter(|p| *p > 0 && !w.has_css_class("icon-circle"))
    else {
        return r;
    };
    let (pw, ph) = (px.min(r.w), px.min(r.h));
    Rect {
        x: r.x + (r.w - pw) / 2,
        y: r.y + (r.h - ph) / 2,
        w: pw,
        h: ph,
    }
}

/// Union of what `w`'s content draws: leaves, and anything that draws, seen through layout-only
/// wrappers (a stretched box is not content; the rows centred in it are).
fn ink(w: &gtk::Widget, root: &gtk::Widget) -> Option<Rect> {
    let mut out: Option<Rect> = None;
    let mut c = w.first_child();
    while let Some(ch) = c {
        if shown(&ch) {
            let r = if layout_only(&ch) && ch.first_child().is_some() {
                ink(&ch, root)
            } else {
                Some(glyph(&ch, bounds(&ch, root))).filter(|r| r.w > 0 && r.h > 0)
            };
            if let Some(r) = r {
                out = Some(match out {
                    None => r,
                    Some(o) => {
                        let (x, y) = (o.x.min(r.x), o.y.min(r.y));
                        let (x1, y1) = ((o.x + o.w).max(r.x + r.w), (o.y + o.h).max(r.y + r.h));
                        Rect {
                            x,
                            y,
                            w: x1 - x,
                            h: y1 - y,
                        }
                    }
                });
            }
        }
        c = ch.next_sibling();
    }
    out
}

/// Measured tree rooted at `w`; coordinates relative to `root`.
pub fn tree(w: &gtk::Widget, root: &gtk::Widget, parent: &str) -> Node {
    let path = if parent.is_empty() {
        name(w)
    } else {
        format!("{parent} › {}", name(w))
    };
    let rect = bounds(w, root);
    let row = w
        .downcast_ref::<gtk::Box>()
        .is_some_and(|b| b.orientation() == gtk::Orientation::Horizontal)
        && !w.has_css_class("no-row-check");
    let clips = w.is::<gtk::ScrolledWindow>()
        || w.is::<gtk::Viewport>()
        || w.is::<gtk::Revealer>()
        || w.is::<gtk::Stack>()
        || w.is::<gtk::Popover>();
    let clipped_text = w
        .downcast_ref::<gtk::Label>()
        .is_some_and(|l| l.layout().is_ellipsized() && !l.has_css_class("may-ellipsize"));
    let mut children = Vec::new();
    let mut c = w.first_child();
    while let Some(ch) = c {
        if shown(&ch) {
            children.push(tree(&ch, root, &path));
        }
        c = ch.next_sibling();
    }
    Node {
        path,
        rect,
        row,
        clips,
        // List scrollers (class `whole-rows`) must show whole rows at rest; page scrollers may cut.
        viewport: w.is::<gtk::ScrolledWindow>() && w.has_css_class("whole-rows"),
        list_row: w.has_css_class("list-row"),
        clipped_text,
        square: w.has_css_class("avatar"),
        inset: if w.has_css_class("glass-panel") || w.has_css_class("module") {
            layout::Inset::Panel
        } else if ["card-list", "sheet", "widget-card"]
            .iter()
            .any(|c| w.has_css_class(c))
        {
            layout::Inset::Card
        } else {
            layout::Inset::None
        },
        ink: ["glass-panel", "module", "card-list", "sheet", "widget-card"]
            .iter()
            .any(|c| w.has_css_class(c))
            .then(|| ink(w, root))
            .flatten(),
        children,
    }
}

pub fn issues(win: &gtk::Window) -> Vec<layout::Issue> {
    let root: &gtk::Widget = win.upcast_ref();
    let Some(child) = win.child() else {
        return Vec::new();
    };
    let frame = Rect {
        x: 0,
        y: 0,
        w: win.width(),
        h: win.height(),
    };
    let mut out = layout::find_in(frame, &tree(&child, root, ""));
    out.extend(covered_cards(&child, root));
    out
}

/// A floating sheet (`widget-gallery`) must never cover a desktop card.
fn covered_cards(top: &gtk::Widget, root: &gtk::Widget) -> Vec<layout::Issue> {
    let (mut sheets, mut cards) = (Vec::new(), Vec::new());
    let mut stack = vec![top.clone()];
    while let Some(w) = stack.pop() {
        if !shown(&w) {
            continue;
        }
        if w.has_css_class("widget-gallery") {
            // Its own preview card is inside it, not under it.
            sheets.push(bounds(&w, root));
            continue;
        } else if w.has_css_class("widget-card") {
            cards.push((name(&w), bounds(&w, root)));
        }
        let mut c = w.first_child();
        while let Some(x) = c {
            c = x.next_sibling();
            stack.push(x);
        }
    }
    let hit = |a: &Rect, b: &Rect| {
        a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
    };
    sheets
        .iter()
        .flat_map(|s| cards.iter().filter(move |(_, c)| hit(s, c)))
        .map(|(n, _)| layout::Issue {
            path: n.clone(),
            message: format!("{n} is covered by the widget gallery"),
        })
        .collect()
}

/// After the first layout: log issues; exit 1 when `WOL_LAYOUT_CHECK=exit`.
pub fn on_map(win: &gtk::Window, name: &'static str) {
    win.connect_map(move |w| {
        let w = w.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(300), move || {
            let found = issues(&w);
            for i in &found {
                eprintln!("wol layout: {name}: {}", i.message);
            }
            if !found.is_empty() && std::env::var("WOL_LAYOUT_CHECK").as_deref() == Ok("exit") {
                std::process::exit(1);
            }
        });
    });
}

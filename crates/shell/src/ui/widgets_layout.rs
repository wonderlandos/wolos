//! Custom widgets: a package's layout tree mapped node by node onto Kit constructors
//! (`kit::data`, `kit::widgets`), so a custom widget looks like it shipped with WolOS. Data that
//! does not fit (missing paths, wrong types) renders empty, never panics; a package that does not
//! validate gets a tidy error card instead.
use crate::kit::data::{self as d, IconSize, Role, Tone};
use crate::kit::widgets as w;
use gtk::prelude::*;
use serde_json::Value;
use shared::widgets::Size;
use shared::widgets::manifest::{Action, Align, Manifest, Node, Problem};
use shared::widgets::template::Scope;
use std::path::{Path, PathBuf};

/// Points a chart draws at most.
const MAX_CHART: usize = 120;

/// Where a package's files are, and whether its buttons act (off in gallery previews).
pub struct Ctx<'a> {
    pub dir: &'a Path,
    /// Images a source downloaded: `<cache>/wolos/widgets/<id>/`.
    pub cache: &'a Path,
    pub interactive: bool,
}

/// The widget card for `m` at `size` with `data` (the source's JSON, or the package sample).
pub fn card(m: &Manifest, size: Size, data: &Value, cx: &Ctx) -> gtk::Box {
    let Some(n) = m.layout(size).or_else(|| m.layouts.first().map(|(_, n)| n)) else {
        return error_card(
            size,
            &m.name,
            &crate::copy::widget_problems(&m.name),
            &["The widget has no layout.".to_string()],
        );
    };
    let body = node(n, Scope::root(data), cx, true);
    body.set_vexpand(true);
    // Content that does not stretch (no spacer) sits centred: equal insets top and bottom.
    if !stretches(n) {
        body.set_valign(gtk::Align::Center);
    }
    d::card(size, &body)
}

/// A stack with a spacer fills the card's height; anything else keeps its natural height.
fn stretches(n: &Node) -> bool {
    match n {
        Node::Stack { children, .. } | Node::Card { children, .. } => children
            .iter()
            .any(|c| matches!(c, Node::Spacer) || stretches(c)),
        Node::Spacer => true,
        _ => false,
    }
}

fn halign(a: Align) -> gtk::Align {
    match a {
        Align::Start => gtk::Align::Start,
        Align::Center => gtk::Align::Center,
        Align::End => gtk::Align::End,
        Align::Fill => gtk::Align::Fill,
    }
}

/// Icon names come from data: only theme-name characters, else the generic widget icon.
pub fn icon_name(s: &str) -> &str {
    let ok = !s.is_empty()
        && s.len() <= 96
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    if ok {
        s
    } else {
        shared::widgets::manifest::DEFAULT_ICON
    }
}

/// A package image: relative to the package (never outside it), or a file the source saved in
/// the widget's cache dir. Anything else is not shown.
pub fn image_path(src: &str, dir: &Path, cache: &Path) -> Option<PathBuf> {
    if src.is_empty() {
        return None;
    }
    let p = Path::new(src);
    let inside = |base: &Path, rel: &Path| {
        rel.components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
            .then(|| base.join(rel))
    };
    if p.is_absolute() {
        let rel = p.strip_prefix(cache).ok()?;
        inside(cache, rel)
    } else {
        inside(dir, p)
    }
}

/// Number from a binding: JSON numbers and numeric strings.
fn num(t: &shared::widgets::template::Template, s: Scope) -> Option<f64> {
    t.number(s).filter(|x| x.is_finite())
}

fn short(x: f64) -> String {
    if x.fract() == 0.0 || x.abs() >= 100.0 {
        format!("{x:.0}")
    } else {
        format!("{x:.1}")
    }
}

/// One node. `vertical`: the parent stacks (a spacer then takes height, not width).
pub fn node(n: &Node, s: Scope, cx: &Ctx, vertical: bool) -> gtk::Widget {
    match n {
        Node::Stack {
            children,
            spacing,
            align,
        } => {
            let b = w::vbox(*spacing);
            for c in children {
                let ch = node(c, s, cx, true);
                if !matches!(c, Node::Spacer) {
                    ch.set_halign(halign(*align));
                }
                b.append(&ch);
            }
            b.upcast()
        }
        Node::Row {
            children,
            spacing,
            align,
        } => {
            let b = w::hbox(*spacing);
            if *align != Align::Center {
                b.add_css_class("no-row-check");
            }
            for c in children {
                let ch = node(c, s, cx, false);
                if !matches!(c, Node::Spacer) {
                    ch.set_valign(halign(*align));
                }
                b.append(&ch);
            }
            b.upcast()
        }
        Node::Card { children, spacing } => {
            let b = w::vbox(*spacing);
            for c in children {
                b.append(&node(c, s, cx, true));
            }
            w::module(&b).upcast()
        }
        Node::Label {
            text,
            role,
            tone,
            lines,
            align,
        } => {
            let l = d::text(&text.render(s), *role, *tone);
            if *lines > 1 {
                l.set_wrap(true);
                l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
                l.set_lines(*lines as i32);
            }
            l.set_xalign(match align {
                Align::Center => 0.5,
                Align::End => 1.0,
                _ => 0.0,
            });
            if *align == Align::Center {
                l.set_justify(gtk::Justification::Center);
            }
            l.upcast()
        }
        Node::Icon { name, size, tone } => {
            d::tinted_icon(icon_name(&name.render(s)), *size, *tone).upcast()
        }
        Node::Image { src, size } => {
            let px = d::image_px(*size);
            match image_path(&src.render(s), cx.dir, cx.cache).filter(|p| p.is_file()) {
                Some(p) => d::picture(&p, px).upcast(),
                None => {
                    let ph = d::tinted_icon("image-x-generic-symbolic", *size, Tone::Dim);
                    ph.set_size_request(px, px);
                    ph.upcast()
                }
            }
        }
        Node::List {
            items,
            item,
            max,
            empty,
            spacing,
        } => {
            let b = w::vbox(*spacing);
            let list = items.array(s).unwrap_or_default();
            if list.is_empty() && !empty.is_empty() {
                b.append(&d::text(empty, Role::Secondary, Tone::Dim));
            }
            for it in list.iter().take(*max as usize) {
                b.append(&node(item, s.with_item(it), cx, true));
            }
            b.upcast()
        }
        Node::Button {
            label,
            icon,
            action,
            open_src,
        } => button(
            label.as_ref().map(|t| t.render(s)),
            icon.as_ref().map(|t| t.render(s)),
            action,
            open_src,
            s,
            cx,
        ),
        Node::Gauge {
            value,
            max,
            label,
            icon,
            tone,
            size,
        } => {
            let v = num(value, s).unwrap_or(0.0);
            let icon = icon.as_ref().map(|t| t.render(s));
            let caption = label.as_ref().map(|t| t.render(s));
            d::Gauge::new(
                v / max,
                &short(v),
                icon.as_deref().map(icon_name),
                caption.as_deref(),
                *tone,
                *size,
            )
            .root
            .upcast()
        }
        Node::Chart {
            kind,
            values,
            max,
            tone,
            size,
        } => {
            let v: Vec<f64> = values
                .array(s)
                .unwrap_or_default()
                .iter()
                .filter_map(|x| match x {
                    Value::Number(n) => n.as_f64(),
                    Value::String(t) => t.trim().parse().ok(),
                    _ => None,
                })
                .filter(|x: &f64| x.is_finite())
                .take(MAX_CHART)
                .collect();
            d::Chart::new(*kind, v, *max, *tone, *size).area.upcast()
        }
        Node::Progress { value, max, tone } => {
            d::meter(num(value, s).unwrap_or(0.0) / max, *tone).upcast()
        }
        Node::Badge { text, tone } => d::badge(&text.render(s), *tone).upcast(),
        Node::Spacer => {
            if vertical {
                d::fill().upcast()
            } else {
                w::spacer().upcast()
            }
        }
    }
}

fn button(
    label: Option<String>,
    icon: Option<String>,
    action: &Action,
    open_src: &str,
    s: Scope,
    cx: &Ctx,
) -> gtk::Widget {
    let tip = match action {
        Action::Run(argv) => argv.first().map(|t| t.render(s)).unwrap_or_default(),
        Action::Open(t) => t.render(s),
        Action::App(id) => id.clone(),
    };
    let act: Box<dyn Fn()> = match action {
        Action::Run(argv) => {
            let mut v: Vec<String> = argv.iter().map(|t| t.render(s)).collect();
            if let Some(rel) = v.first().and_then(|a| a.strip_prefix("./")) {
                v[0] = cx.dir.join(rel).to_string_lossy().into_owned();
            }
            Box::new(move || crate::kit::spawn(&v))
        }
        Action::Open(t) => {
            let url = t.render(s);
            let ok = shared::widgets::manifest::open_allowed(&url, Action::open_prefix(open_src));
            Box::new(move || {
                if ok {
                    d::open_uri(&url);
                } else {
                    eprintln!("wol-shell: widget link not opened: {url}");
                }
            })
        }
        Action::App(id) => {
            let id = id.clone();
            Box::new(move || d::launch_app(&id))
        }
    };
    let b = match (label, icon) {
        (Some(l), _) => w::button(&l, w::Kind::Default, act),
        (None, Some(i)) => w::icon_button(icon_name(&i), &tip, act),
        (None, None) => w::button("", w::Kind::Default, act),
    };
    b.set_sensitive(cx.interactive);
    b.set_valign(gtk::Align::Center);
    b.upcast()
}

/// A widget that can't show its data: one friendly sentence (`crate::copy`) and, when there is
/// something a developer can act on, Show Details flips the card to those lines (JSON paths,
/// exit codes), scrollable, with Hide Details to flip back.
pub fn error_card(size: Size, name: &str, message: &str, details: &[String]) -> gtk::Box {
    let b = w::vbox(w::S1);
    b.set_vexpand(true);
    let head = w::hbox(w::S2);
    head.append(&d::tinted_icon(
        "dialog-warning-symbolic",
        IconSize::Small,
        Tone::Warning,
    ));
    head.append(&d::text(name, Role::Title, Tone::Text));
    b.append(&head);
    let wrapped = |s: &str, tone: Tone, lines: i32| {
        let t = d::text(s, Role::Secondary, tone);
        t.set_wrap(true);
        t.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        t.set_lines(lines);
        t.set_xalign(0.0);
        t
    };
    let front = w::vbox(w::S1);
    front.set_vexpand(true);
    front.append(&wrapped(
        message,
        Tone::Dim,
        if size == Size::Small { 4 } else { 3 },
    ));
    b.append(&front);
    if !details.is_empty() {
        let list = w::vbox(w::S1);
        for l in details {
            list.append(&wrapped(l, Tone::Dim, 4));
        }
        let back = d::scroller(&list);
        back.set_visible(false);
        b.append(&back);
        front.append(&d::fill());
        let toggle = w::button("Show Details", w::Kind::Plain, || {});
        toggle.set_halign(gtk::Align::Start);
        let (f, bk) = (front.clone(), back.clone());
        toggle.connect_clicked(move |t| {
            let open = !bk.is_visible();
            bk.set_visible(open);
            f.set_visible(!open);
            t.set_label(if open { "Hide Details" } else { "Show Details" });
        });
        b.append(&toggle);
    }
    d::card(size, &b)
}

/// Validation problems as detail lines: "layouts.small.type: unknown type \"stak\"".
pub fn problem_lines(problems: &[Problem]) -> Vec<String> {
    problems.iter().map(|p| p.to_string()).collect()
}

/// A package whose source reads from the network, before the user allowed it.
pub fn consent_card(size: Size, name: &str, host: &str, allow: impl Fn() + 'static) -> gtk::Box {
    let b = w::vbox(w::S1);
    b.set_vexpand(true);
    let head = w::hbox(w::S2);
    head.append(&d::tinted_icon(
        "network-server-symbolic",
        IconSize::Small,
        Tone::Accent,
    ));
    head.append(&d::text(name, Role::Title, Tone::Text));
    b.append(&head);
    let t = d::text(
        &format!("Wants to read data from {host}."),
        Role::Secondary,
        Tone::Dim,
    );
    t.set_wrap(true);
    t.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    t.set_lines(3);
    t.set_xalign(0.0);
    b.append(&t);
    b.append(&d::fill());
    let a = w::button("Allow", w::Kind::Primary, allow);
    a.set_halign(gtk::Align::Start);
    b.append(&a);
    d::card(size, &b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_icon_names_and_image_paths_are_contained() {
        assert_eq!(
            icon_name("user-available-symbolic"),
            "user-available-symbolic"
        );
        for bad in ["", "../../etc/passwd", "a b", "x/y"] {
            assert_eq!(
                icon_name(bad),
                shared::widgets::manifest::DEFAULT_ICON,
                "{bad}"
            );
        }
        let (dir, cache) = (
            Path::new("/pkg"),
            Path::new("/home/a/.cache/wolos/widgets/x"),
        );
        for (src, want) in [
            ("avatar.png", Some("/pkg/avatar.png")),
            ("img/a.png", Some("/pkg/img/a.png")),
            ("../secret.png", None),
            ("/etc/shadow", None),
            (
                "/home/a/.cache/wolos/widgets/x/av/1.png",
                Some("/home/a/.cache/wolos/widgets/x/av/1.png"),
            ),
            ("/home/a/.cache/wolos/widgets/x/../../y.png", None),
            ("", None),
        ] {
            assert_eq!(
                image_path(src, dir, cache).as_deref(),
                want.map(Path::new),
                "{src}"
            );
        }
    }

    #[test]
    fn numbers_read_short() {
        for (x, want) in [
            (72.0, "72"),
            (72.44, "72.4"),
            (1234.4, "1234"),
            (0.5, "0.5"),
        ] {
            assert_eq!(short(x), want);
        }
    }
}

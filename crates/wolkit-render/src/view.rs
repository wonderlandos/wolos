// SPDX-License-Identifier: BSD-2-Clause
//! Raising a `shared::view` tree into a wolkit scene: the inverse of the kit backend's lowering
//! (`wolkit::window::kit::lower`). A libwol app's view tree, the one the shell's Kit host draws in
//! GTK (`crates/shell/src/ui/app_view.rs`, golden `app-window-parts`), becomes a patch stream of
//! wolkit parts with the kit look, so the same tree draws in wolkit on every render target.
//!
//! Part ids: a wire id `n` of an interactive part is scene node `n`; structural nodes take ids
//! from 10 000 up, so events map straight back to the app's ids.

use shared::view::{Node, Role};
use wolkit::event::{self, kind as ev};
use wolkit::scene::Kind;
use wolkit::scene::patch::{APPEND, Writer};
use wolkit::scene::prop::id;
use wolkit::scene::state;

/// The default look (`wolos-crystal/shards/wolkit/styles/kit.wolcss`), compiled first.
pub const KIT_SHEET: &str = include_str!("../../../wolos-crystal/shards/wolkit/styles/kit.wolcss");
/// The view parts on top of it.
pub const VIEW_SHEET: &str = include_str!("view.wolcss");

/// Class ids: the kit sheet's eight, then `view.wolcss` in order of first appearance.
pub mod class {
    pub const PAGE: u32 = 8;
    pub const TITLEBAR: u32 = 9;
    pub const LIST: u32 = 10;
    pub const LIST_ROW: u32 = 11;
    pub const LIST_FIELD: u32 = 12;
    pub const ICON: u32 = 13;
    pub const ROLE_DISPLAY: u32 = 14;
}

/// First id of structural nodes (wire ids of interactive parts stay below it).
pub const FIRST_STRUCT: u32 = 10_000;

/// The compiled style table of the two sheets.
pub fn style_table() -> Result<wolkit::style::Table, String> {
    wolkit::style::compile_sources(&[("00-kit.wolcss", KIT_SHEET), ("view.wolcss", VIEW_SHEET)])
        .map(|c| wolkit::style::Table::new(c.table))
        .map_err(|d| wolkit::style::render_all(&d))
}

fn role_class(r: Role) -> u32 {
    class::ROLE_DISPLAY
        + match r {
            Role::Display => 0,
            Role::Large => 1,
            Role::Title => 2,
            Role::Body => 3,
            Role::Secondary => 4,
            Role::Caption => 5,
        }
}

const ROW: u32 = 0;
const COLUMN: u32 = 1;

struct Raise<'a> {
    w: &'a mut Writer,
    next: u32,
}

impl Raise<'_> {
    fn fresh(&mut self, kind: Kind) -> u32 {
        let n = self.next;
        self.next += 1;
        self.w.create(n, kind);
        n
    }

    fn container(&mut self, dir: u32, spacing: u8) -> u32 {
        let n = self.fresh(Kind::Box);
        self.w
            .set_prop(n, id::FLEX_DIRECTION, &[dir])
            .set_f32(n, id::GAP, f32::from(spacing));
        n
    }

    fn label(&mut self, text: &str, role: Role) -> u32 {
        let n = self.fresh(Kind::Text);
        self.w.set_text(n, text).set_style(n, &[role_class(role)]);
        n
    }

    fn node(&mut self, node: &Node, parent: u32) {
        match node {
            Node::Stack { spacing, children } | Node::Row { spacing, children } => {
                let row = matches!(node, Node::Row { .. });
                let n = self.container(if row { ROW } else { COLUMN }, *spacing);
                if row {
                    self.w.set_prop(n, id::ALIGN_ITEMS, &[1]);
                }
                self.w.insert(parent, n, APPEND);
                for c in children {
                    self.node(c, n);
                }
            }
            Node::Label { text, role } => {
                let n = self.label(text, *role);
                self.w.insert(parent, n, APPEND);
            }
            Node::Button { id: wire, label } => {
                self.w.create(*wire, Kind::Button);
                self.w
                    .set_text(*wire, label)
                    .bind_event(*wire, event::bit(ev::CLICK))
                    .set_a11y(*wire, 3, label, "");
                self.w.insert(parent, *wire, APPEND);
            }
            Node::Spacer => {
                let n = self.fresh(Kind::Spacer);
                self.w.set_f32(n, id::GROW, 1.0);
                self.w.insert(parent, n, APPEND);
            }
            Node::TextField {
                id: wire,
                text,
                placeholder,
            } => {
                self.w.create(*wire, Kind::TextInput);
                let p = self.w.string(placeholder);
                self.w
                    .set_text(*wire, text)
                    .set_prop(*wire, id::PLACEHOLDER, &[p])
                    .bind_event(*wire, event::bit(ev::TEXT_CHANGED))
                    .set_a11y(*wire, 7, placeholder, "");
                self.w.insert(parent, *wire, APPEND);
            }
            Node::Toggle {
                id: wire,
                on,
                label,
            } => {
                // A labelled switch is a row: the label, the space, the switch (Kit host).
                let row = self.container(ROW, 8);
                self.w
                    .set_prop(row, id::ALIGN_ITEMS, &[1])
                    .set_style(row, &[class::LIST_ROW]);
                let l = self.label(label, Role::Body);
                let gap = self.fresh(Kind::Spacer);
                self.w.set_f32(gap, id::GROW, 1.0);
                self.w.create(*wire, Kind::Toggle);
                self.w
                    .set_state(*wire, if *on { state::CHECKED } else { 0 })
                    .bind_event(*wire, event::bit(ev::TOGGLE))
                    .set_a11y(*wire, 4, label, "");
                self.w.insert(parent, row, APPEND);
                for c in [l, gap, *wire] {
                    self.w.insert(row, c, APPEND);
                }
            }
            Node::Slider {
                id: wire,
                value,
                min,
                max,
            } => {
                self.w.create(*wire, Kind::Slider);
                self.w
                    .set_f32(*wire, id::SLIDER_MIN, *min as f32)
                    .set_f32(*wire, id::SLIDER_MAX, *max as f32)
                    .set_f32(*wire, id::SLIDER_VALUE, *value as f32)
                    .bind_event(*wire, event::bit(ev::SLIDER))
                    .set_a11y(*wire, 6, "", "");
                self.w.insert(parent, *wire, APPEND);
            }
            Node::Image { source, size } => {
                let n = self.fresh(Kind::Image);
                // A bare name is an icon of the theme (`icon:<name>`); a path or `icon:` stays.
                let named;
                let source = if source.contains('/') || source.starts_with("icon:") {
                    source.as_str()
                } else {
                    named = format!("icon:{source}");
                    named.as_str()
                };
                let s = self.w.string(source);
                self.w
                    .set_prop(n, id::IMAGE_SOURCE, &[s])
                    .set_prop(
                        n,
                        id::WIDTH,
                        &[wolkit::scene::prop::DIM_PX, f32::from(*size).to_bits()],
                    )
                    .set_prop(
                        n,
                        id::HEIGHT,
                        &[wolkit::scene::prop::DIM_PX, f32::from(*size).to_bits()],
                    )
                    .set_style(n, &[class::ICON]);
                self.w.insert(parent, n, APPEND);
            }
            Node::List { children } => {
                // A card of rows with a divider between them.
                let n = self.container(COLUMN, 0);
                self.w.set_style(n, &[class::LIST]);
                self.w.insert(parent, n, APPEND);
                for (k, c) in children.iter().enumerate() {
                    if k > 0 {
                        let d = self.fresh(Kind::Divider);
                        self.w.insert(n, d, APPEND);
                    }
                    // A switch row is itself the list row; other parts sit in a row-high cell.
                    let row = self.container(COLUMN, 0);
                    self.w.set_style(row, &[class::LIST_FIELD]);
                    self.w.insert(n, row, APPEND);
                    self.node(c, row);
                }
            }
        }
    }
}

/// Writes the patch that shows `tree` as an app window's content: a page with an empty title
/// bar line (as the Kit host frames app windows), then the tree.
pub fn raise(tree: &Node, w: &mut Writer) {
    let mut r = Raise {
        w,
        next: FIRST_STRUCT,
    };
    let page = r.container(COLUMN, 12);
    let full = [wolkit::scene::prop::DIM_PERCENT, 100f32.to_bits()];
    r.w.set_style(page, &[class::PAGE])
        .set_prop(page, id::WIDTH, &full)
        .set_prop(page, id::HEIGHT, &full);
    r.w.insert(0, page, APPEND);
    let bar = r.fresh(Kind::Box);
    r.w.set_style(bar, &[class::TITLEBAR]);
    let line = r.fresh(Kind::Divider);
    r.w.insert(page, bar, APPEND).insert(page, line, APPEND);
    r.node(tree, page);
}

/// The icons of the sample tree, held in memory (the web build has no icon folder): the WolOS
/// symbolic set (`design/icons/symbolic`), file name and SVG.
pub const ICONS: &[(&str, &str)] = &[(
    "weather-clear-symbolic.svg",
    include_str!("../../../design/icons/symbolic/weather-clear-symbolic.svg"),
)];

/// Hands [`ICONS`] to the image module (once per process is enough, more is harmless).
pub fn register_icons() {
    for (file, svg) in ICONS {
        wolkit::image::icon::register(file, svg);
    }
}

/// The libwol sample window (`crates/shell/tests/golden_cases/app_view.rs` `sample()`, the tree
/// `tests/c/parts.c` builds), as JSON so this crate does not depend on the shell.
pub const SAMPLE_JSON: &str = include_str!("../fixtures/app-window-parts.json");

/// The sample tree.
pub fn sample() -> Node {
    serde_json::from_str(SAMPLE_JSON).unwrap_or(Node::Spacer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_class_ids_follow_the_sheets() {
        let c = wolkit::style::compile_sources(&[
            ("00-kit.wolcss", KIT_SHEET),
            ("view.wolcss", VIEW_SHEET),
        ])
        .unwrap_or_else(|d| panic!("{}", wolkit::style::render_all(&d)));
        let names: Vec<&str> = c.table.classes.iter().map(|k| k.name.as_str()).collect();
        let want = [
            (class::PAGE, "page"),
            (class::TITLEBAR, "titlebar"),
            (class::LIST, "list"),
            (class::LIST_ROW, "list-row"),
            (class::LIST_FIELD, "list-field"),
            (class::ICON, "icon"),
            (class::ROLE_DISPLAY, "role-display"),
            (role_class(Role::Caption), "role-caption"),
        ];
        for (id, name) in want {
            assert_eq!(
                names.get(usize::try_from(id).unwrap()).copied(),
                Some(name),
                "{names:?}"
            );
        }
        assert_eq!(
            &names[..8],
            [
                "primary",
                "danger",
                "flat",
                "card",
                "heading",
                "menu-item",
                "tab",
                "sheet"
            ]
        );
    }

    #[test]
    fn the_sample_is_the_libwol_parts_tree() {
        let s = sample();
        let Node::Stack { children, .. } = &s else {
            panic!("{s:?}")
        };
        assert_eq!(children.len(), 3);
        let json = serde_json::to_string(&s).unwrap();
        for part in [
            "Sunny",
            "Lisbon, 21°",
            "Ada",
            "Wi-Fi",
            "Cancel",
            "Save",
            "weather-clear-symbolic",
        ] {
            assert!(json.contains(part), "{part}");
        }
    }

    #[test]
    fn a_raised_tree_is_a_valid_patch() {
        let mut w = Writer::new();
        raise(&sample(), &mut w);
        let bytes = w.finish(1).to_vec();
        let mut scratch = wolkit::scene::patch::Scratch::default();
        let p = wolkit::scene::patch::Patch::parse(&bytes, &mut scratch).expect("parses");
        let mut scene = wolkit::scene::Scene::new();
        let mut j = wolkit::scene::Journal::default();
        scene.try_apply(&p, &mut j).expect("applies");
    }
}

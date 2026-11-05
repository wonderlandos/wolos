// SPDX-License-Identifier: BSD-2-Clause
//! Raising a `shared::view` tree into a wolkit scene: the inverse of the kit backend's lowering
//! (`wolkit::window::kit::lower`). A libwol app's view tree, the one the shell's Kit host draws in
//! GTK (`crates/shell/src/ui/app_view.rs`, golden `app-window-parts`), becomes a patch stream of
//! wolkit parts with the kit look, so the same tree draws in wolkit on every render target.
//!
//! Part ids: a wire id `n` of an interactive part is scene node `n`; structural nodes take ids
//! from 10 000 up, so events map straight back to the app's ids.

use shared::view::{CardStyle, Chip, ChipKind, Choice, Node, Role};
use wolkit::event::{self, kind as ev};
use wolkit::scene::Kind;
use wolkit::scene::patch::{APPEND, Writer};
use wolkit::scene::prop::id;
use wolkit::scene::state;

/// The default look (`wolos-crystal/shards/wolkit/styles/kit.wolcss`), compiled first.
pub const KIT_SHEET: &str = include_str!("../../../wolos-crystal/shards/wolkit/styles/kit.wolcss");
/// The view parts on top of it.
pub const VIEW_SHEET: &str = include_str!("view.wolcss");
/// The design-system parts (checkbox, radio, chips, tabs, progress, card, disclosure); their
/// internals use pixel values off the spacing scale, so the sheet carries the off-scale pragma.
pub const PARTS_SHEET: &str = include_str!("parts.wolcss");

/// Class ids: the kit sheet's eight, then `view.wolcss` in order of first appearance.
pub mod class {
    pub const PAGE: u32 = 8;
    pub const TITLEBAR: u32 = 9;
    pub const LIST: u32 = 10;
    pub const LIST_ROW: u32 = 11;
    pub const LIST_FIELD: u32 = 12;
    pub const ICON: u32 = 13;
    pub const ROLE_DISPLAY: u32 = 14;
    // `view.wolcss`, the design-system parts (after the six `role-*` classes, 14 to 19).
    pub const PART_ROW: u32 = 20;
    pub const CHECK: u32 = 21;
    pub const CHECK_ON: u32 = 22;
    pub const CHECK_DASH: u32 = 23;
    pub const RADIO: u32 = 24;
    pub const RADIO_ON: u32 = 25;
    pub const RADIO_DOT: u32 = 26;
    pub const CHIP: u32 = 27;
    pub const CHIP_ON: u32 = 28;
    pub const CHIP_GHOST: u32 = 29;
    pub const CHIP_SET: u32 = 30;
    pub const TAB_CELL: u32 = 31;
    pub const VIEW_TAB: u32 = 32;
    pub const VIEW_TAB_ON: u32 = 33;
    pub const TAB_LINE: u32 = 34;
    pub const TAB_LINE_ON: u32 = 35;
    pub const TAB_PANEL: u32 = 36;
    pub const PROGRESS: u32 = 37;
    pub const PROGRESS_FILL: u32 = 38;
    pub const SURFACE_CARD: u32 = 39;
    pub const CARD_ELEVATED: u32 = 40;
    pub const CARD_FILLED: u32 = 41;
    pub const CARD_OUTLINED: u32 = 42;
    pub const DISCLOSURE_HEAD: u32 = 43;
    pub const DISCLOSURE_BODY: u32 = 44;
    pub const ICON_DIM: u32 = 45;
    pub const ICON_INK: u32 = 46;
    pub const ICON_ACCENT: u32 = 47;
    pub const GROUP: u32 = 48;
}

/// What a click on a raised part of the design-system nodes means to the app: these parts keep
/// their state in the app's tree, so a click does not flip them in the scene; the host turns the
/// click into the act the GTK parts send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pick {
    /// A checkbox or disclosure: the state it asks for.
    Toggled { id: u32, on: bool },
    /// An option, chip or tab: its key.
    Picked { id: u32, key: String },
}

/// A clickable scene part and its [`Pick`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bind {
    pub node: u32,
    pub pick: Pick,
}

/// First id of structural nodes (wire ids of interactive parts stay below it).
pub const FIRST_STRUCT: u32 = 10_000;

/// The compiled style table of the two sheets.
pub fn style_table() -> Result<wolkit::style::Table, String> {
    wolkit::style::compile_sources(&[
        ("00-kit.wolcss", KIT_SHEET),
        ("view.wolcss", VIEW_SHEET),
        ("parts.wolcss", PARTS_SHEET),
    ])
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
    binds: Vec<Bind>,
}

impl Raise<'_> {
    fn fresh(&mut self, kind: Kind) -> u32 {
        let n = self.next;
        self.next += 1;
        self.w.create(n, kind);
        n
    }

    fn container(&mut self, dir: u32, spacing: u8) -> u32 {
        let n = self.bare(dir);
        self.w.set_f32(n, id::GAP, f32::from(spacing));
        n
    }

    /// A box whose gap comes from its class (a number the specs hold, `view.wolcss`).
    fn bare(&mut self, dir: u32) -> u32 {
        let n = self.fresh(Kind::Box);
        self.w.set_prop(n, id::FLEX_DIRECTION, &[dir]);
        n
    }

    fn label(&mut self, text: &str, role: Role) -> u32 {
        let n = self.fresh(Kind::Text);
        self.w.set_text(n, text).set_style(n, &[role_class(role)]);
        n
    }

    /// A named symbolic icon of `size` px, tinted by `tint` (an `ICON_*` class).
    fn icon(&mut self, name: &str, size: u16, tint: u32) -> u32 {
        let n = self.fresh(Kind::Image);
        let s = self.w.string(&format!("icon:{name}"));
        let px = f32::from(size).to_bits();
        self.w
            .set_prop(n, id::IMAGE_SOURCE, &[s])
            .set_prop(n, id::WIDTH, &[wolkit::scene::prop::DIM_PX, px])
            .set_prop(n, id::HEIGHT, &[wolkit::scene::prop::DIM_PX, px])
            .set_style(n, &[class::ICON, tint]);
        n
    }

    /// A box of `classes` whose children are `kids`.
    fn boxed(&mut self, dir: u32, classes: &[u32], kids: &[u32]) -> u32 {
        let n = self.bare(dir);
        self.w.set_style(n, classes);
        for k in kids {
            self.w.insert(n, *k, APPEND);
        }
        n
    }

    /// A clickable row of `kids` (a checkbox, radio option, disclosure header): a button without
    /// the button look, named for assistive technology; the click is `pick`.
    fn press(&mut self, role: u32, name: &str, st: u32, kids: &[u32], pick: Pick) -> u32 {
        let n = self.fresh(Kind::Button);
        self.w
            .set_style(n, &[class::PART_ROW])
            .set_state(n, st)
            .bind_event(n, event::bit(ev::CLICK))
            .set_a11y(n, role, name, "");
        for k in kids {
            self.w.insert(n, *k, APPEND);
        }
        self.binds.push(Bind { node: n, pick });
        n
    }

    fn checkbox(&mut self, wire: u32, label: &str, checked: bool, mixed: bool) -> u32 {
        let on = checked || mixed;
        let mut inner = Vec::new();
        if mixed {
            inner.push(self.boxed(ROW, &[class::CHECK_DASH], &[]));
        } else if checked {
            inner.push(self.icon("object-select-symbolic", 12, class::ICON_INK));
        }
        let b = self.boxed(
            ROW,
            &[if on { class::CHECK_ON } else { class::CHECK }],
            &inner,
        );
        let l = self.label(label, Role::Body);
        let st = if checked { state::CHECKED } else { 0 };
        self.press(
            5,
            label,
            st,
            &[b, l],
            // A click on a mixed box turns them all on.
            Pick::Toggled {
                id: wire,
                on: mixed || !checked,
            },
        )
    }

    fn radio_group(&mut self, wire: u32, options: &[Choice], selected: &str) -> u32 {
        let mut rows = Vec::new();
        for o in options {
            let on = o.key == selected;
            let dot = if on {
                vec![self.boxed(ROW, &[class::RADIO_DOT], &[])]
            } else {
                Vec::new()
            };
            let ring = self.boxed(
                ROW,
                &[if on { class::RADIO_ON } else { class::RADIO }],
                &dot,
            );
            let l = self.label(&o.label, Role::Body);
            rows.push(self.press(
                // The ARIA radio role has no wolkit id yet: the group role names it.
                5,
                &o.label,
                if on { state::CHECKED } else { 0 },
                &[ring, l],
                Pick::Picked {
                    id: wire,
                    key: o.key.clone(),
                },
            ));
        }
        let g = self.boxed(COLUMN, &[class::GROUP], &rows);
        self.w.set_a11y(g, 1, "", "");
        g
    }

    fn chips(&mut self, wire: u32, kind: ChipKind, chips: &[Chip]) -> u32 {
        let mut parts = Vec::new();
        for c in chips {
            let on = kind == ChipKind::Filter && c.on;
            let class = match (kind, on) {
                (ChipKind::Suggestion, _) => class::CHIP_GHOST,
                (_, true) => class::CHIP_ON,
                _ => class::CHIP,
            };
            let mut kids = Vec::new();
            if on {
                kids.push(self.icon("object-select-symbolic", 12, class::ICON_ACCENT));
            }
            kids.push(self.label(&c.label, Role::Title));
            if kind == ChipKind::Input {
                kids.push(self.icon("window-close-symbolic", 12, class::ICON_DIM));
            }
            let n = self.fresh(Kind::Button);
            self.w
                .set_style(n, &[class])
                .set_state(n, if on { state::CHECKED } else { 0 })
                .bind_event(n, event::bit(ev::CLICK))
                .set_a11y(n, 3, &c.label, "");
            for k in kids {
                self.w.insert(n, k, APPEND);
            }
            self.binds.push(Bind {
                node: n,
                pick: Pick::Picked {
                    id: wire,
                    key: c.key.clone(),
                },
            });
            parts.push(n);
        }
        self.boxed(ROW, &[class::CHIP_SET], &parts)
    }

    fn tabs(&mut self, wire: u32, tabs: &[Choice], selected: &str, children: &[Node]) -> u32 {
        let at = tabs.iter().position(|t| t.key == selected).unwrap_or(0);
        let mut cells = Vec::new();
        for (k, t) in tabs.iter().enumerate() {
            let on = k == at;
            let b = self.fresh(Kind::Button);
            self.w
                .set_text(b, &t.label)
                .set_style(
                    b,
                    &[if on {
                        class::VIEW_TAB_ON
                    } else {
                        class::VIEW_TAB
                    }],
                )
                .set_state(b, if on { state::SELECTED } else { 0 })
                .bind_event(b, event::bit(ev::CLICK))
                .set_a11y(b, 12, &t.label, "");
            self.binds.push(Bind {
                node: b,
                pick: Pick::Picked {
                    id: wire,
                    key: t.key.clone(),
                },
            });
            let line = self.boxed(
                ROW,
                &[if on {
                    class::TAB_LINE_ON
                } else {
                    class::TAB_LINE
                }],
                &[],
            );
            cells.push(self.boxed(COLUMN, &[class::TAB_CELL], &[b, line]));
        }
        let list = self.boxed(ROW, &[class::GROUP], &cells);
        self.w.set_a11y(list, 13, "", "");
        let rule = self.fresh(Kind::Divider);
        let panel = self.boxed(COLUMN, &[class::TAB_PANEL], &[]);
        self.w.set_a11y(panel, 14, "", "");
        if let Some(c) = children.get(at) {
            self.node(c, panel);
        }
        self.boxed(COLUMN, &[class::GROUP], &[list, rule, panel])
    }

    fn progress(&mut self, value: Option<f64>, label: &str) -> u32 {
        let fill = self.boxed(ROW, &[class::PROGRESS_FILL], &[]);
        // No known amount: a short bar at the start (the GTK bar pulses; a picture is still).
        let part = value.map_or(0.3, |v| v.clamp(0.0, 1.0)) as f32;
        self.w.set_prop(
            fill,
            id::WIDTH,
            &[wolkit::scene::prop::DIM_PERCENT, (part * 100.0).to_bits()],
        );
        let track = self.boxed(ROW, &[class::PROGRESS], &[fill]);
        self.w.set_a11y(track, 22, label, "");
        track
    }

    fn card(&mut self, style: CardStyle, children: &[Node]) -> u32 {
        let look = match style {
            CardStyle::Elevated => class::CARD_ELEVATED,
            CardStyle::Filled => class::CARD_FILLED,
            CardStyle::Outlined => class::CARD_OUTLINED,
        };
        let n = self.boxed(COLUMN, &[class::SURFACE_CARD, look], &[]);
        self.w.set_a11y(n, 1, "", "");
        for c in children {
            self.node(c, n);
        }
        n
    }

    fn disclosure(&mut self, wire: u32, title: &str, open: bool, children: &[Node]) -> u32 {
        let chevron = self.icon(
            if open {
                "pan-down-symbolic"
            } else {
                "pan-end-symbolic"
            },
            12,
            class::ICON_DIM,
        );
        let t = self.label(title, Role::Title);
        let head = self.press(
            3,
            title,
            if open { state::EXPANDED } else { 0 },
            &[chevron, t],
            Pick::Toggled {
                id: wire,
                on: !open,
            },
        );
        self.w
            .set_style(head, &[class::PART_ROW, class::DISCLOSURE_HEAD]);
        let mut parts = vec![head];
        if open {
            let body = self.boxed(COLUMN, &[class::DISCLOSURE_BODY], &[]);
            for c in children {
                self.node(c, body);
            }
            parts.push(body);
        }
        self.boxed(COLUMN, &[class::GROUP], &parts)
    }

    /// The design-system nodes (DS-COMPONENTS), raised from their own rules (`parts.wolcss`).
    fn design_node(&mut self, node: &Node, parent: u32) {
        let n = match node {
            Node::Checkbox {
                id: wire,
                label,
                checked,
                mixed,
            } => self.checkbox(*wire, label, *checked, *mixed),
            Node::RadioGroup {
                id: wire,
                options,
                selected,
            } => self.radio_group(*wire, options, selected),
            Node::Chips {
                id: wire,
                kind,
                chips,
            } => self.chips(*wire, *kind, chips),
            Node::Tabs {
                id: wire,
                tabs,
                selected,
                children,
            } => self.tabs(*wire, tabs, selected, children),
            Node::Progress { value, label } => self.progress(*value, label),
            Node::Divider => self.fresh(Kind::Divider),
            Node::Card { style, children } => self.card(*style, children),
            Node::Disclosure {
                id: wire,
                title,
                open,
                children,
            } => self.disclosure(*wire, title, *open, children),
            _ => return,
        };
        self.w.insert(parent, n, APPEND);
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
                let row = self.bare(ROW);
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
            Node::Checkbox { .. }
            | Node::RadioGroup { .. }
            | Node::Chips { .. }
            | Node::Tabs { .. }
            | Node::Progress { .. }
            | Node::Divider
            | Node::Card { .. }
            | Node::Disclosure { .. } => self.design_node(node, parent),
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
    let _ = raise_bound(tree, w);
}

/// [`raise`], and what a click on each clickable part of the design-system nodes means.
pub fn raise_bound(tree: &Node, w: &mut Writer) -> Vec<Bind> {
    let mut r = Raise {
        w,
        next: FIRST_STRUCT,
        binds: Vec::new(),
    };
    let page = r.bare(COLUMN);
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
    r.binds
}

/// The icons of the sample tree, held in memory (the web build has no icon folder): the WolOS
/// symbolic set (`design/icons/symbolic`), file name and SVG.
pub const ICONS: &[(&str, &str)] = &[
    (
        "weather-clear-symbolic.svg",
        include_str!("../../../design/icons/symbolic/weather-clear-symbolic.svg"),
    ),
    // The marks of the design-system parts: check (checkbox, filter chip), chevrons (disclosure),
    // remove (input chip).
    (
        "object-select-symbolic.svg",
        include_str!("../../../design/icons/symbolic/object-select-symbolic.svg"),
    ),
    (
        "pan-end-symbolic.svg",
        include_str!("../../../design/icons/symbolic/pan-end-symbolic.svg"),
    ),
    (
        "pan-down-symbolic.svg",
        include_str!("../../../design/icons/symbolic/pan-down-symbolic.svg"),
    ),
    (
        "window-close-symbolic.svg",
        include_str!("../../../design/icons/symbolic/window-close-symbolic.svg"),
    ),
];

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

/// Settings > Look as a view tree (`fixtures/settings-look.json`, the shell's golden
/// `settings-look-w*` cases draw the same file): long text that wraps, switches, a slider.
pub const SETTINGS_LOOK_JSON: &str = include_str!("../fixtures/settings-look.json");

/// The component cases (`fixtures/components.json`): one tree per part the Kit page draws, the
/// shell's golden cases `kit-view-<name>` draw the same file.
pub const COMPONENTS_JSON: &str = include_str!("../fixtures/components.json");

/// The Settings > Look tree.
pub fn settings_look() -> Node {
    serde_json::from_str(SETTINGS_LOOK_JSON).unwrap_or(Node::Spacer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_class_ids_follow_the_sheets() {
        let c = wolkit::style::compile_sources(&[
            ("00-kit.wolcss", KIT_SHEET),
            ("view.wolcss", VIEW_SHEET),
            ("parts.wolcss", PARTS_SHEET),
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
            (class::PART_ROW, "part-row"),
            (class::CHECK, "check"),
            (class::CHECK_ON, "check-on"),
            (class::CHECK_DASH, "check-dash"),
            (class::RADIO, "radio"),
            (class::RADIO_ON, "radio-on"),
            (class::RADIO_DOT, "radio-dot"),
            (class::CHIP, "chip"),
            (class::CHIP_ON, "chip-on"),
            (class::CHIP_GHOST, "chip-ghost"),
            (class::CHIP_SET, "chip-set"),
            (class::TAB_CELL, "tab-cell"),
            (class::VIEW_TAB, "view-tab"),
            (class::VIEW_TAB_ON, "view-tab-on"),
            (class::TAB_LINE, "tab-line"),
            (class::TAB_LINE_ON, "tab-line-on"),
            (class::TAB_PANEL, "tab-panel"),
            (class::PROGRESS, "progress"),
            (class::PROGRESS_FILL, "progress-fill"),
            (class::SURFACE_CARD, "surface-card"),
            (class::CARD_ELEVATED, "card-elevated"),
            (class::CARD_FILLED, "card-filled"),
            (class::CARD_OUTLINED, "card-outlined"),
            (class::DISCLOSURE_HEAD, "disclosure-head"),
            (class::DISCLOSURE_BODY, "disclosure-body"),
            (class::ICON_DIM, "icon-dim"),
            (class::ICON_INK, "icon-ink"),
            (class::ICON_ACCENT, "icon-accent"),
            (class::GROUP, "group"),
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

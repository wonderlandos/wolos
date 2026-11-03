// SPDX-License-Identifier: BSD-2-Clause
//! Shell layout (ADR 0031): which edge the bar and the Dock sit on, what the bar's three slots
//! hold and what the Dock shows. Saved in `shell.json` (`wol_core::prefs`), read by the shell's
//! `Spec::bar` / `Spec::dock` and by the menu bar and Dock builders. [`Layout::default`] is
//! today's desktop; a Windows-like desktop is [`Layout::taskbar`]: one struct, no other surface
//! changes. Plugin bar items and Dock entries are [`BarItem::Plugin`] and [`DockItem::Plugin`],
//! the seam `shared::plugins` entries plug into.
use serde::{Deserialize, Serialize};

/// Screen edge a surface is anchored to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Edge {
    #[default]
    Top,
    Bottom,
    Left,
    Right,
}

/// How a bar or Dock on an edge lays its items out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Orientation {
    Horizontal,
    Vertical,
}

impl Edge {
    pub const ALL: [Edge; 4] = [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right];

    pub fn id(self) -> &'static str {
        match self {
            Edge::Top => "top",
            Edge::Bottom => "bottom",
            Edge::Left => "left",
            Edge::Right => "right",
        }
    }

    pub fn parse(s: &str) -> Option<Edge> {
        Edge::ALL.into_iter().find(|e| e.id() == s)
    }

    pub fn opposite(self) -> Edge {
        match self {
            Edge::Top => Edge::Bottom,
            Edge::Bottom => Edge::Top,
            Edge::Left => Edge::Right,
            Edge::Right => Edge::Left,
        }
    }

    /// Items on a top or bottom edge run left to right; on a side edge, top to bottom.
    pub fn orientation(self) -> Orientation {
        match self {
            Edge::Top | Edge::Bottom => Orientation::Horizontal,
            Edge::Left | Edge::Right => Orientation::Vertical,
        }
    }
}

/// Menubar: logo, app menus, status, clock (WolOS order). Taskbar: start item, tasks, tray, clock.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BarStyle {
    #[default]
    Menubar,
    Taskbar,
}

/// One item in a bar slot. The shell draws the built-in ones itself; `Plugin` is a
/// `[[menubar]]` entry of an enabled plugin, rendered through the Lua sandbox.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum BarItem {
    /// The WolOS menu.
    Logo,
    /// Frontmost app's name with its app menu.
    AppName,
    /// The frontmost app's remaining menus (Window, ...).
    AppMenus,
    /// A start item (taskbar style): opens Search.
    Start,
    /// Workspace switcher (taskbar style, or `workspaces_in_bar`).
    Workspaces,
    /// Open windows as buttons (taskbar style).
    Tasks,
    /// Status-notifier tray icons.
    Tray,
    Focus,
    Battery,
    Network,
    Volume,
    Search,
    ControlCenter,
    /// Opens Notification Center.
    Clock,
    /// `[[menubar]]` entry `id` of plugin `plugin`.
    Plugin {
        plugin: String,
        id: String,
    },
}

/// One item in the Dock, in order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum DockItem {
    /// The pinned apps (the shell's list) followed by running apps that are not pinned.
    Apps,
    /// One pinned app by desktop entry id.
    App {
        id: String,
    },
    Separator,
    /// The Downloads stack.
    Downloads,
    Trash,
    /// `[[dock]]` entry `id` of plugin `plugin`.
    Plugin {
        plugin: String,
        id: String,
    },
}

/// Where everything sits. Unknown keys are ignored and missing ones take today's values, so an
/// older `shell.json` still loads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    pub bar: Edge,
    pub bar_style: BarStyle,
    /// None: no Dock surface (taskbar style merges it into the bar).
    pub dock: Option<Edge>,
    pub dock_merged_into_bar: bool,
    pub workspaces_in_bar: bool,
    pub tray_in_bar: bool,
    pub left: Vec<BarItem>,
    pub centre: Vec<BarItem>,
    pub right: Vec<BarItem>,
    pub dock_items: Vec<DockItem>,
}

impl Default for Layout {
    /// Today's desktop: bar on top, Dock at the bottom, the WolOS menu bar order.
    fn default() -> Self {
        use BarItem as B;
        Self {
            bar: Edge::Top,
            bar_style: BarStyle::Menubar,
            dock: Some(Edge::Bottom),
            dock_merged_into_bar: false,
            workspaces_in_bar: false,
            tray_in_bar: false,
            left: vec![B::Logo, B::AppName, B::AppMenus],
            centre: Vec::new(),
            right: vec![
                B::Focus,
                B::Battery,
                B::Network,
                B::Volume,
                B::Search,
                B::ControlCenter,
                B::Clock,
            ],
            dock_items: vec![
                DockItem::Apps,
                DockItem::Separator,
                DockItem::Downloads,
                DockItem::Trash,
            ],
        }
    }
}

impl Layout {
    /// Windows-like: one bar at the bottom with start, tasks, tray and clock; no Dock.
    pub fn taskbar() -> Self {
        use BarItem as B;
        Self {
            bar: Edge::Bottom,
            bar_style: BarStyle::Taskbar,
            dock: None,
            dock_merged_into_bar: true,
            workspaces_in_bar: true,
            tray_in_bar: true,
            left: vec![B::Start, B::Workspaces, B::Tasks],
            centre: Vec::new(),
            right: vec![
                B::Tray,
                B::Network,
                B::Volume,
                B::Battery,
                B::Clock,
                B::ControlCenter,
            ],
            dock_items: Vec::new(),
        }
    }

    pub fn parse(json: &str) -> Result<Layout, String> {
        let l: Layout = serde_json::from_str(json).map_err(|e| format!("layout: {e}"))?;
        l.validate()?;
        Ok(l)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default() + "\n"
    }

    /// Bar and Dock on different edges; every item once; a Dock only when there is an edge for it.
    pub fn validate(&self) -> Result<(), String> {
        if self.dock == Some(self.bar) {
            return Err("The bar and the Dock cannot share an edge.".into());
        }
        if self.dock.is_none() && !self.dock_items.is_empty() {
            return Err("There is no Dock edge for the Dock items.".into());
        }
        let mut seen: Vec<&BarItem> = Vec::new();
        for item in self.bar_items() {
            if seen.contains(&item) {
                return Err(format!("The bar item {} is listed twice.", item.id()));
            }
            seen.push(item);
        }
        let mut seen: Vec<&DockItem> = Vec::new();
        for item in &self.dock_items {
            if *item != DockItem::Separator && seen.contains(&item) {
                return Err(format!("The Dock item {} is listed twice.", item.id()));
            }
            seen.push(item);
        }
        Ok(())
    }

    /// Every bar item, left slot first.
    pub fn bar_items(&self) -> impl Iterator<Item = &BarItem> {
        self.left.iter().chain(&self.centre).chain(&self.right)
    }

    /// Takes every item of `plugin` out of the bar and the Dock (disable, remove).
    pub fn remove_plugin(&mut self, plugin: &str) {
        let keep = |i: &BarItem| !matches!(i, BarItem::Plugin { plugin: p, .. } if p == plugin);
        self.left.retain(keep);
        self.centre.retain(keep);
        self.right.retain(keep);
        self.dock_items
            .retain(|i| !matches!(i, DockItem::Plugin { plugin: p, .. } if p == plugin));
    }
}

impl BarItem {
    /// Stable name for messages and `shell.json` (`plugin:<plugin>/<id>` for plugin items).
    pub fn id(&self) -> String {
        match self {
            BarItem::Plugin { plugin, id } => format!("plugin:{plugin}/{id}"),
            other => serde_json::to_value(other)
                .ok()
                .and_then(|v| v.get("type")?.as_str().map(str::to_owned))
                .unwrap_or_default(),
        }
    }
}

impl DockItem {
    pub fn id(&self) -> String {
        match self {
            DockItem::Plugin { plugin, id } => format!("plugin:{plugin}/{id}"),
            DockItem::App { id } => format!("app:{id}"),
            other => serde_json::to_value(other)
                .ok()
                .and_then(|v| v.get("type")?.as_str().map(str::to_owned))
                .unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_todays_desktop_and_round_trips() {
        let l = Layout::default();
        assert_eq!((l.bar, l.dock), (Edge::Top, Some(Edge::Bottom)));
        assert_eq!(l.bar_style, BarStyle::Menubar);
        assert_eq!(l.left[0], BarItem::Logo);
        assert_eq!(l.right.last(), Some(&BarItem::Clock));
        assert_eq!(l.dock_items.last(), Some(&DockItem::Trash));
        assert_eq!(Layout::parse(&l.to_json()).unwrap(), l);
        assert_eq!(Layout::parse("{}").unwrap(), l, "missing keys = today");
        assert_eq!(
            Layout::parse(r#"{"unknown": 1}"#).unwrap(),
            l,
            "older or newer files load"
        );
        let t = Layout::taskbar();
        assert_eq!((t.bar, t.dock), (Edge::Bottom, None));
        assert!(t.bar_items().any(|i| *i == BarItem::Tasks));
        assert_eq!(Layout::parse(&t.to_json()).unwrap(), t);
    }

    #[test]
    fn items_serialize_by_type_and_plugins_can_be_removed() {
        let p = BarItem::Plugin {
            plugin: "org.wolos.stats".into(),
            id: "cpu".into(),
        };
        assert_eq!(
            serde_json::to_string(&p).unwrap(),
            r#"{"type":"plugin","plugin":"org.wolos.stats","id":"cpu"}"#
        );
        assert_eq!(
            serde_json::to_string(&BarItem::Clock).unwrap(),
            r#"{"type":"clock"}"#
        );
        assert_eq!(BarItem::ControlCenter.id(), "control-center");
        assert_eq!(p.id(), "plugin:org.wolos.stats/cpu");
        assert_eq!(
            DockItem::App {
                id: "org.gnome.Nautilus".into()
            }
            .id(),
            "app:org.gnome.Nautilus"
        );
        let mut l = Layout::default();
        l.right.insert(0, p.clone());
        l.dock_items.push(DockItem::Plugin {
            plugin: "org.wolos.stats".into(),
            id: "top".into(),
        });
        l.validate().unwrap();
        l.remove_plugin("org.wolos.stats");
        assert_eq!(l, Layout::default());
    }

    #[test]
    fn validate_names_the_problem() {
        let l = Layout {
            dock: Some(Edge::Top),
            ..Layout::default()
        };
        assert_eq!(
            l.validate().unwrap_err(),
            "The bar and the Dock cannot share an edge."
        );
        let mut l = Layout::default();
        l.left.push(BarItem::Clock);
        assert_eq!(
            l.validate().unwrap_err(),
            "The bar item clock is listed twice."
        );
        let mut l = Layout {
            dock: None,
            ..Layout::default()
        };
        assert!(l.validate().unwrap_err().contains("Dock"));
        l.dock_items.clear();
        l.validate().unwrap();
        for e in Edge::ALL {
            assert_eq!(Edge::parse(e.id()), Some(e));
            assert_eq!(e.opposite().opposite(), e);
        }
        assert_eq!(Edge::Left.orientation(), Orientation::Vertical);
    }
}

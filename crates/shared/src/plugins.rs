// SPDX-License-Identifier: BSD-2-Clause
//! Shell plugin manifest, schema 1 (ADR 0031, docs/modules/plugins.md). A plugin is a folder with
//! a `plugin.toml` and the files it names; this module is the frozen contract: the [`Manifest`]
//! structs, a closed enum for every choice, and [`parse`], which turns the TOML into a validated
//! manifest or one sentence saying what to fix. `plugin.toml` uses a small, fixed part of TOML
//! (strings, integers, booleans, arrays, `[table]`, `[[table]]`, comments), read here without a
//! dependency; anything else is a load error with a sentence. The same rules are published as
//! JSON Schema in [`SCHEMA_JSON`] (`docs/plugins/plugin.schema.json`).
use serde::{Deserialize, Serialize};

pub use crate::widgets::Size;

pub const SCHEMA: u32 = 1;
pub const MANIFEST_FILE: &str = "plugin.toml";
/// User plugins under `$XDG_DATA_HOME` (default `~/.local/share`): `<dir>/<id>/plugin.toml`.
pub const USER_DIR: &str = "wolos/plugins";
/// Plugins shipped with the system or installed by wopkg.
pub const SYSTEM_DIR: &str = "/usr/share/wolos/plugins";
/// Per-plugin writable state under `$XDG_STATE_HOME`: `<dir>/<id>/state.json`.
pub const STATE_DIR: &str = "wolos/plugins";
/// Toggle flag files under `$XDG_STATE_HOME`: `<dir>/<plugin id>.<toggle id>`.
pub const TOGGLES_DIR: &str = "wolos/toggles";
/// Where the schema is installed for editors and agents.
pub const SCHEMA_PATH: &str = "/usr/share/wolos/plugins.schema.json";
/// Reserved for plugins shipped by WolOS itself.
pub const RESERVED_PREFIX: &str = "org.wolos.";
/// Whole folder, every file counted.
pub const MAX_TOTAL_SIZE: u64 = 1 << 20;
/// `wol.state` JSON per plugin.
pub const MAX_STATE: usize = 64 * 1024;
/// Largest output of one `wol.run` or `wol.http_get`, in the shell and in `wol plugin run` alike
/// (QA-PLUG-35); the manual says 64 KB.
pub const MAX_RUN_OUTPUT: usize = 64 * 1024;
pub const MAX_ID: usize = 64;
pub const MAX_ENTRY_ID: usize = 32;
pub const MAX_NAME: usize = 40;
pub const MAX_DESCRIPTION: usize = 120;
pub const MAX_PATH: usize = 128;
pub const MAX_PREFIX: usize = 8;
/// Renders that may fail in a row before the plugin is disabled with a toast.
pub const MAX_FAILURES: u32 = 3;
/// JSON Schema of `plugin.toml`, the file installed at [`SCHEMA_PATH`].
pub const SCHEMA_JSON: &str = include_str!("../../../docs/plugins/plugin.schema.json");

/// What a plugin may ask for. Shown before enabling; stored as grants under the plugin id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Permission {
    /// `wol.notify`.
    Notifications,
    /// `wol.run { argv }` (argv only, limited, through the helper policy for `wol` verbs).
    Run,
    /// `wol.http_get`; each host is granted separately at first use.
    Http,
    /// `wol.clipboard.text`.
    Clipboard,
    /// `wol.open`.
    Open,
    /// `wol.toggle_set`.
    Toggles,
}

impl Permission {
    pub const ALL: [Permission; 6] = [
        Permission::Notifications,
        Permission::Run,
        Permission::Http,
        Permission::Clipboard,
        Permission::Open,
        Permission::Toggles,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Permission::Notifications => "notifications",
            Permission::Run => "run",
            Permission::Http => "http",
            Permission::Clipboard => "clipboard",
            Permission::Open => "open",
            Permission::Toggles => "toggles",
        }
    }

    /// Settings and Store label.
    pub fn label(self) -> &'static str {
        match self {
            Permission::Notifications => "Show notifications",
            Permission::Run => "Run commands as you",
            Permission::Http => "Read data from the web (each site asks once)",
            Permission::Clipboard => "Read the clipboard",
            Permission::Open => "Open apps, files and links",
            Permission::Toggles => "Switch its own toggles",
        }
    }

    pub fn parse(s: &str) -> Option<Permission> {
        Permission::ALL.into_iter().find(|p| p.id() == s)
    }
}

/// The capability tables a manifest may have; the table name is the id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Menubar,
    Dock,
    Widget,
    Panel,
    Search,
    Tile,
    Toggle,
    Hotkey,
    Webapp,
    Theme,
    Look,
    Hypr,
    /// `[[ipc]]`: a Lua function `wol shell call <plugin> <id> <json>` runs.
    Ipc,
}

impl Kind {
    pub const ALL: [Kind; 13] = [
        Kind::Menubar,
        Kind::Dock,
        Kind::Widget,
        Kind::Panel,
        Kind::Search,
        Kind::Tile,
        Kind::Toggle,
        Kind::Hotkey,
        Kind::Webapp,
        Kind::Theme,
        Kind::Look,
        Kind::Hypr,
        Kind::Ipc,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Kind::Menubar => "menubar",
            Kind::Dock => "dock",
            Kind::Widget => "widget",
            Kind::Panel => "panel",
            Kind::Search => "search",
            Kind::Tile => "tile",
            Kind::Toggle => "toggle",
            Kind::Hotkey => "hotkey",
            Kind::Webapp => "webapp",
            Kind::Theme => "theme",
            Kind::Look => "look",
            Kind::Hypr => "hypr",
            Kind::Ipc => "ipc",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.id() == s)
    }

    /// Keys an entry of this kind may have (`id` always). The schema lists the same.
    pub fn keys(self) -> &'static [&'static str] {
        match self {
            Kind::Menubar => &["id", "script", "side"],
            Kind::Dock => &["id", "name", "icon", "exec"],
            Kind::Widget => &["id", "script", "sizes"],
            Kind::Panel => &["id", "script", "anchor"],
            Kind::Search => &["id", "script", "kind", "prefix"],
            Kind::Tile => &["id", "icon", "label", "toggle"],
            Kind::Toggle => &["id", "on", "off"],
            Kind::Hotkey => &["id", "default", "exec", "group"],
            Kind::Webapp => &["id", "name", "url", "icon", "profile"],
            Kind::Theme | Kind::Look | Kind::Hypr => &["id", "file"],
            Kind::Ipc => &["id", "script", "args"],
        }
    }

    /// Kinds the manifest format knows but no part of WolOS applies yet (audit X6-1): `check`
    /// refuses them, `new` and Plugin Maker do not offer them. `[[panel]]` and `[[search]]` run
    /// in `wol plugin run` and the Lua sandbox, but the shell hosts neither yet (QA-PLUG-4); the
    /// shell's `every_offered_kind_has_a_host` keeps this list honest.
    pub const PLANNED: [Kind; 4] = [Kind::Panel, Kind::Search, Kind::Look, Kind::Hypr];

    /// See [`Kind::PLANNED`].
    pub fn is_planned(self) -> bool {
        Kind::PLANNED.contains(&self)
    }

    /// The sentence `check` and `new` give for a planned kind.
    pub fn planned_sentence(self) -> String {
        format!(
            "[[{}]] is planned: WolOS does not apply it yet, so remove the section for now.",
            self.id()
        )
    }

    /// Entries of this kind run Lua.
    pub fn has_script(self) -> bool {
        matches!(
            self,
            Kind::Menubar | Kind::Widget | Kind::Panel | Kind::Search | Kind::Ipc
        )
    }
}

/// Header keys, in file order.
pub const HEADER_KEYS: [&str; 14] = [
    "schema",
    "id",
    "name",
    "version",
    "author",
    "license",
    "description",
    "min_wolos",
    "permissions",
    "icon",
    "screenshots",
    "categories",
    "homepage",
    "about",
];

/// The header keys that only describe the plugin for the hub's store page (hubcheck `facts`):
/// new in WolOS 0.13, so a manifest that sets one says `min_wolos = "0.13"` or newer (a 0.12
/// reader refuses unknown header keys).
pub const LISTING_KEYS: [&str; 5] = ["icon", "screenshots", "categories", "homepage", "about"];
/// The WolOS that reads [`LISTING_KEYS`]: (major, minor).
pub const LISTING_MIN_WOLOS: (u32, u32) = (0, 13);
/// Most screenshots in a listing.
pub const MAX_SCREENSHOTS: usize = 8;
/// Longest `about` text (characters), the hub's limit for a listing description.
pub const MAX_ABOUT: usize = 4000;
/// Longest `homepage` address.
pub const MAX_HOMEPAGE: usize = 200;

/// Bar slot a `[[menubar]]` item goes to (`shared::layout::Layout` slots).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Side {
    Left,
    Centre,
    #[default]
    Right,
}

impl Side {
    pub const ALL: [Side; 3] = [Side::Left, Side::Centre, Side::Right];
    pub fn id(self) -> &'static str {
        match self {
            Side::Left => "left",
            Side::Centre => "centre",
            Side::Right => "right",
        }
    }
    pub fn parse(s: &str) -> Option<Side> {
        Side::ALL.into_iter().find(|x| x.id() == s)
    }
}

/// Where a `[[panel]]` opens.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Anchor {
    #[default]
    Top,
    Bottom,
    Left,
    Right,
    Centre,
}

impl Anchor {
    pub const ALL: [Anchor; 5] = [
        Anchor::Top,
        Anchor::Bottom,
        Anchor::Left,
        Anchor::Right,
        Anchor::Centre,
    ];
    pub fn id(self) -> &'static str {
        match self {
            Anchor::Top => "top",
            Anchor::Bottom => "bottom",
            Anchor::Left => "left",
            Anchor::Right => "right",
            Anchor::Centre => "centre",
        }
    }
    pub fn parse(s: &str) -> Option<Anchor> {
        Anchor::ALL.into_iter().find(|x| x.id() == s)
    }
}

/// `results`: adds rows to every query. `command`: handles queries that start with `prefix`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SearchKind {
    #[default]
    Results,
    Command,
}

impl SearchKind {
    pub const ALL: [SearchKind; 2] = [SearchKind::Results, SearchKind::Command];
    pub fn id(self) -> &'static str {
        match self {
            SearchKind::Results => "results",
            SearchKind::Command => "command",
        }
    }
    pub fn parse(s: &str) -> Option<SearchKind> {
        SearchKind::ALL.into_iter().find(|x| x.id() == s)
    }
}

/// Browser profile of a web app: the user's (`shared`) or one of its own (`own`, isolated cookies).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Profile {
    #[default]
    Shared,
    Own,
}

impl Profile {
    pub const ALL: [Profile; 2] = [Profile::Shared, Profile::Own];
    pub fn id(self) -> &'static str {
        match self {
            Profile::Shared => "shared",
            Profile::Own => "own",
        }
    }
    pub fn parse(s: &str) -> Option<Profile> {
        Profile::ALL.into_iter().find(|x| x.id() == s)
    }
}

/// Keywords a `[[hypr]]` fragment may set; everything else (`bind`, `exec`, `plugin`, `env`,
/// `source`, categories) is refused.
pub const HYPR_KEYWORDS: [&str; 4] = ["windowrule", "layerrule", "bezier", "animation"];

/// Effects a plugin's `windowrule` may have (audit L7): placement and looks. Refused, among
/// others: `nofocus`, `noinitialfocus`, `fullscreen`, `maximize`, `pin`, `stayfocused`,
/// `suppressevent`, `dimaround`, `nodim`: they take windows away from the person.
pub const HYPR_WINDOW_EFFECTS: [&str; 14] = [
    "float",
    "tile",
    "pseudo",
    "size",
    "minsize",
    "maxsize",
    "move",
    "center",
    "workspace",
    "rounding",
    "bordersize",
    "animation",
    "opacity",
    "idleinhibit",
];

/// Effects a plugin's `layerrule` may have: blur and animation of its own surfaces (not `xray`,
/// `noanim` or `dimaround`).
pub const HYPR_LAYER_EFFECTS: [&str; 5] = [
    "blur",
    "blurpopups",
    "ignorezero",
    "ignorealpha",
    "animation",
];

/// Lowest `opacity` a plugin's rule may give a window: below it the window is hard to see.
pub const HYPR_MIN_OPACITY: f64 = 0.5;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MenubarEntry {
    pub id: String,
    /// Lua file in the plugin folder.
    pub script: String,
    pub side: Side,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockEntry {
    pub id: String,
    pub name: String,
    /// Icon name, or an image file in the plugin folder.
    pub icon: String,
    pub exec: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WidgetEntry {
    pub id: String,
    pub script: String,
    pub sizes: Vec<Size>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanelEntry {
    pub id: String,
    pub script: String,
    pub anchor: Anchor,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchEntry {
    pub id: String,
    pub script: String,
    pub kind: SearchKind,
    /// Only for `command`: what a query starts with.
    pub prefix: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TileEntry {
    pub id: String,
    pub icon: String,
    pub label: String,
    /// A `[[toggle]]` id of the same plugin.
    pub toggle: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToggleEntry {
    pub id: String,
    pub on: Vec<String>,
    pub off: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotkeyEntry {
    pub id: String,
    /// `super+shift+x`: modifiers from super, ctrl, alt, shift, then a key. The user may rebind.
    pub default: String,
    pub exec: Vec<String>,
    /// Heading in Settings > Keyboard; empty = the plugin's name.
    pub group: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebappEntry {
    pub id: String,
    pub name: String,
    pub url: String,
    pub icon: String,
    pub profile: Profile,
}

/// What a `[[ipc]]` function takes: the JSON value after `call <plugin> <id>`, checked before
/// the Lua runs (`any`: whatever was sent; `none`: nothing, `null` on the wire).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum ArgType {
    #[default]
    Any,
    None,
    Text,
    Number,
    Bool,
    List,
    Object,
}

impl ArgType {
    pub const ALL: [ArgType; 7] = [
        ArgType::Any,
        ArgType::None,
        ArgType::Text,
        ArgType::Number,
        ArgType::Bool,
        ArgType::List,
        ArgType::Object,
    ];
    /// The `args` value in `plugin.toml`.
    pub fn id(self) -> &'static str {
        match self {
            ArgType::Any => "any",
            ArgType::None => "none",
            ArgType::Text => "text",
            ArgType::Number => "number",
            ArgType::Bool => "bool",
            ArgType::List => "list",
            ArgType::Object => "object",
        }
    }
    /// See [`ArgType::id`].
    pub fn parse(s: &str) -> Option<ArgType> {
        ArgType::ALL.into_iter().find(|x| x.id() == s)
    }

    /// The value sent fits.
    pub fn accepts(self, v: &serde_json::Value) -> bool {
        use serde_json::Value as J;
        match self {
            ArgType::Any => true,
            ArgType::None => v.is_null(),
            ArgType::Text => matches!(v, J::String(_)),
            ArgType::Number => matches!(v, J::Number(_)),
            ArgType::Bool => matches!(v, J::Bool(_)),
            ArgType::List => matches!(v, J::Array(_)),
            ArgType::Object => matches!(v, J::Object(_)),
        }
    }

    /// What `call` answers when the value does not fit.
    pub fn sentence(self, plugin: &str, id: &str) -> String {
        match self {
            ArgType::None => format!("{plugin} {id} takes no arguments."),
            t => format!("{plugin} {id} takes one JSON {}.", t.id()),
        }
    }
}

/// `[[ipc]]`: one Lua function other programs call through `wol shell call <plugin> <id> <json>`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IpcEntry {
    /// Also the name of the function in the table the script returns (`wol.ipc { <id> = ... }`).
    pub id: String,
    pub script: String,
    pub args: ArgType,
}

/// `[[theme]]`, `[[look]]`, `[[hypr]]`: a file in the plugin folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub id: String,
    pub file: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub license: String,
    pub description: String,
    /// Lowest WolOS version that has every API the plugin uses; empty = any.
    pub min_wolos: String,
    pub permissions: Vec<Permission>,
    /// Store-page picture: a `.png` or `.webp` in the folder (needs 0.13, [`LISTING_KEYS`]); empty = none.
    #[serde(default)]
    pub icon: String,
    /// Store-page screenshots: `.png` or `.webp` files in the folder, at most [`MAX_SCREENSHOTS`].
    #[serde(default)]
    pub screenshots: Vec<String>,
    /// App Store categories ([`crate::package::CATEGORIES`]).
    #[serde(default)]
    pub categories: Vec<String>,
    /// `https://` address of the project; empty = none.
    #[serde(default)]
    pub homepage: String,
    /// Longer description for the store page, with line breaks; empty = none.
    #[serde(default)]
    pub about: String,
    pub menubar: Vec<MenubarEntry>,
    pub dock: Vec<DockEntry>,
    pub widget: Vec<WidgetEntry>,
    pub panel: Vec<PanelEntry>,
    pub search: Vec<SearchEntry>,
    pub tile: Vec<TileEntry>,
    pub toggle: Vec<ToggleEntry>,
    pub hotkey: Vec<HotkeyEntry>,
    pub webapp: Vec<WebappEntry>,
    pub theme: Vec<FileEntry>,
    pub look: Vec<FileEntry>,
    pub hypr: Vec<FileEntry>,
    #[serde(default)]
    pub ipc: Vec<IpcEntry>,
}

impl Manifest {
    /// Kinds with at least one entry, in table order.
    pub fn kinds(&self) -> Vec<Kind> {
        Kind::ALL
            .into_iter()
            .filter(|k| self.count(*k) > 0)
            .collect()
    }

    /// How many entries of kind `k` the manifest declares.
    pub fn count(&self, k: Kind) -> usize {
        match k {
            Kind::Menubar => self.menubar.len(),
            Kind::Dock => self.dock.len(),
            Kind::Widget => self.widget.len(),
            Kind::Panel => self.panel.len(),
            Kind::Search => self.search.len(),
            Kind::Tile => self.tile.len(),
            Kind::Toggle => self.toggle.len(),
            Kind::Hotkey => self.hotkey.len(),
            Kind::Webapp => self.webapp.len(),
            Kind::Theme => self.theme.len(),
            Kind::Look => self.look.len(),
            Kind::Hypr => self.hypr.len(),
            Kind::Ipc => self.ipc.len(),
        }
    }

    /// Every (kind, entry id), in table order.
    pub fn entries(&self) -> Vec<(Kind, &str)> {
        let mut v: Vec<(Kind, &str)> = Vec::new();
        v.extend(self.menubar.iter().map(|e| (Kind::Menubar, e.id.as_str())));
        v.extend(self.dock.iter().map(|e| (Kind::Dock, e.id.as_str())));
        v.extend(self.widget.iter().map(|e| (Kind::Widget, e.id.as_str())));
        v.extend(self.panel.iter().map(|e| (Kind::Panel, e.id.as_str())));
        v.extend(self.search.iter().map(|e| (Kind::Search, e.id.as_str())));
        v.extend(self.tile.iter().map(|e| (Kind::Tile, e.id.as_str())));
        v.extend(self.toggle.iter().map(|e| (Kind::Toggle, e.id.as_str())));
        v.extend(self.hotkey.iter().map(|e| (Kind::Hotkey, e.id.as_str())));
        v.extend(self.webapp.iter().map(|e| (Kind::Webapp, e.id.as_str())));
        v.extend(self.theme.iter().map(|e| (Kind::Theme, e.id.as_str())));
        v.extend(self.look.iter().map(|e| (Kind::Look, e.id.as_str())));
        v.extend(self.hypr.iter().map(|e| (Kind::Hypr, e.id.as_str())));
        v.extend(self.ipc.iter().map(|e| (Kind::Ipc, e.id.as_str())));
        v
    }

    /// Lua scripts by (kind, entry id), in table order.
    pub fn scripts(&self) -> Vec<(Kind, &str, &str)> {
        let mut v: Vec<(Kind, &str, &str)> = Vec::new();
        v.extend(
            self.menubar
                .iter()
                .map(|e| (Kind::Menubar, e.id.as_str(), e.script.as_str())),
        );
        v.extend(
            self.widget
                .iter()
                .map(|e| (Kind::Widget, e.id.as_str(), e.script.as_str())),
        );
        v.extend(
            self.panel
                .iter()
                .map(|e| (Kind::Panel, e.id.as_str(), e.script.as_str())),
        );
        v.extend(
            self.search
                .iter()
                .map(|e| (Kind::Search, e.id.as_str(), e.script.as_str())),
        );
        v.extend(
            self.ipc
                .iter()
                .map(|e| (Kind::Ipc, e.id.as_str(), e.script.as_str())),
        );
        v
    }

    /// Every file the manifest names, relative to the folder (scripts, theme/look/hypr files,
    /// image icons, the listing icon and screenshots), without duplicates and in table order.
    pub fn files(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.scripts().iter().map(|s| s.2).collect();
        let icons = self
            .dock
            .iter()
            .map(|e| e.icon.as_str())
            .chain(self.tile.iter().map(|e| e.icon.as_str()))
            .chain(self.webapp.iter().map(|e| e.icon.as_str()))
            .filter(|i| is_icon_file(i));
        v.extend(icons);
        v.extend(self.theme.iter().map(|e| e.file.as_str()));
        v.extend(self.look.iter().map(|e| e.file.as_str()));
        v.extend(self.hypr.iter().map(|e| e.file.as_str()));
        v.extend(Some(self.icon.as_str()).filter(|i| !i.is_empty()));
        v.extend(self.screenshots.iter().map(String::as_str));
        let mut seen: Vec<&str> = Vec::new();
        v.retain(|f| {
            let new = !seen.contains(f);
            seen.push(*f);
            new
        });
        v
    }

    /// Whether the manifest asks for permission `p`.
    pub fn has(&self, p: Permission) -> bool {
        self.permissions.contains(&p)
    }

    /// `org.wolos` and `org.wolos.*` are reserved for WolOS. The bare id is reserved too: a
    /// plugin `org.wolos` with `[[toggle]] id = "stay-awake"` would share the flag file
    /// `org.wolos.stay-awake` of the built-in toggle.
    pub fn is_reserved(&self) -> bool {
        is_reserved_id(&self.id)
    }

    /// Every `[[toggle]]`, `[[dock]]` and `[[hotkey]]` argv that is not a `wol` verb, as
    /// (section, entry id, argv). These run as the user, so they need `run` (declared and
    /// granted); `wol` verbs go through the helper policy.
    pub fn commands(&self) -> Vec<(Kind, &str, &[String])> {
        let mut v: Vec<(Kind, &str, &[String])> = Vec::new();
        for t in &self.toggle {
            v.push((Kind::Toggle, &t.id, &t.on));
            v.push((Kind::Toggle, &t.id, &t.off));
        }
        for d in &self.dock {
            v.push((Kind::Dock, &d.id, &d.exec));
        }
        for h in &self.hotkey {
            v.push((Kind::Hotkey, &h.id, &h.exec));
        }
        v.retain(|(_, _, a)| !is_wol_argv(a));
        v
    }

    /// `~/.local/share/wolos/plugins/<id>/plugin.toml` text with the same content.
    pub fn to_toml(&self) -> String {
        let mut out = String::new();
        let s = |v: &str| format!("\"{}\"", escape(v));
        let list = |v: &[String]| {
            format!(
                "[{}]",
                v.iter().map(|x| s(x)).collect::<Vec<_>>().join(", ")
            )
        };
        out.push_str(&format!("schema = {}\n", self.schema));
        out.push_str(&format!("id = {}\n", s(&self.id)));
        out.push_str(&format!("name = {}\n", s(&self.name)));
        out.push_str(&format!("version = {}\n", s(&self.version)));
        if !self.author.is_empty() {
            out.push_str(&format!("author = {}\n", s(&self.author)));
        }
        out.push_str(&format!("license = {}\n", s(&self.license)));
        if !self.description.is_empty() {
            out.push_str(&format!("description = {}\n", s(&self.description)));
        }
        if !self.min_wolos.is_empty() {
            out.push_str(&format!("min_wolos = {}\n", s(&self.min_wolos)));
        }
        if !self.permissions.is_empty() {
            let p: Vec<String> = self.permissions.iter().map(|p| p.id().into()).collect();
            out.push_str(&format!("permissions = {}\n", list(&p)));
        }
        if !self.icon.is_empty() {
            out.push_str(&format!("icon = {}\n", s(&self.icon)));
        }
        if !self.screenshots.is_empty() {
            out.push_str(&format!("screenshots = {}\n", list(&self.screenshots)));
        }
        if !self.categories.is_empty() {
            out.push_str(&format!("categories = {}\n", list(&self.categories)));
        }
        if !self.homepage.is_empty() {
            out.push_str(&format!("homepage = {}\n", s(&self.homepage)));
        }
        if !self.about.is_empty() {
            out.push_str(&format!("about = {}\n", s(&self.about)));
        }
        for e in &self.menubar {
            out.push_str(&format!(
                "\n[[menubar]]\nid = {}\nscript = {}\nside = {}\n",
                s(&e.id),
                s(&e.script),
                s(e.side.id())
            ));
        }
        for e in &self.dock {
            out.push_str(&format!(
                "\n[[dock]]\nid = {}\nname = {}\nicon = {}\nexec = {}\n",
                s(&e.id),
                s(&e.name),
                s(&e.icon),
                list(&e.exec)
            ));
        }
        for e in &self.widget {
            let sizes: Vec<String> = e.sizes.iter().map(|z| z.id().into()).collect();
            out.push_str(&format!(
                "\n[[widget]]\nid = {}\nscript = {}\nsizes = {}\n",
                s(&e.id),
                s(&e.script),
                list(&sizes)
            ));
        }
        for e in &self.panel {
            out.push_str(&format!(
                "\n[[panel]]\nid = {}\nscript = {}\nanchor = {}\n",
                s(&e.id),
                s(&e.script),
                s(e.anchor.id())
            ));
        }
        for e in &self.search {
            out.push_str(&format!(
                "\n[[search]]\nid = {}\nscript = {}\nkind = {}\n",
                s(&e.id),
                s(&e.script),
                s(e.kind.id())
            ));
            if !e.prefix.is_empty() {
                out.push_str(&format!("prefix = {}\n", s(&e.prefix)));
            }
        }
        for e in &self.tile {
            out.push_str(&format!(
                "\n[[tile]]\nid = {}\nicon = {}\nlabel = {}\ntoggle = {}\n",
                s(&e.id),
                s(&e.icon),
                s(&e.label),
                s(&e.toggle)
            ));
        }
        for e in &self.toggle {
            out.push_str(&format!(
                "\n[[toggle]]\nid = {}\non = {}\noff = {}\n",
                s(&e.id),
                list(&e.on),
                list(&e.off)
            ));
        }
        for e in &self.hotkey {
            out.push_str(&format!(
                "\n[[hotkey]]\nid = {}\ndefault = {}\nexec = {}\n",
                s(&e.id),
                s(&e.default),
                list(&e.exec)
            ));
            if !e.group.is_empty() {
                out.push_str(&format!("group = {}\n", s(&e.group)));
            }
        }
        for e in &self.webapp {
            out.push_str(&format!(
                "\n[[webapp]]\nid = {}\nname = {}\nurl = {}\nicon = {}\nprofile = {}\n",
                s(&e.id),
                s(&e.name),
                s(&e.url),
                s(&e.icon),
                s(e.profile.id())
            ));
        }
        for (k, es) in [
            (Kind::Theme, &self.theme),
            (Kind::Look, &self.look),
            (Kind::Hypr, &self.hypr),
        ] {
            for e in es {
                out.push_str(&format!(
                    "\n[[{}]]\nid = {}\nfile = {}\n",
                    k.id(),
                    s(&e.id),
                    s(&e.file)
                ));
            }
        }
        for e in &self.ipc {
            out.push_str(&format!(
                "\n[[ipc]]\nid = {}\nscript = {}\nargs = {}\n",
                s(&e.id),
                s(&e.script),
                s(e.args.id())
            ));
        }
        out
    }
}

fn escape(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            c => o.push(c),
        }
    }
    o
}

/// `org.example.stats` or `alice.stats`: 2 or more lowercase labels of letters, digits and `-`,
/// joined by dots, up to [`MAX_ID`] bytes. Also the folder name.
pub fn is_plugin_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_ID
        && s.split('.').count() >= 2
        && s.split('.').all(|p| {
            !p.is_empty()
                && !p.starts_with('-')
                && !p.ends_with('-')
                && p.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

/// Entry ids: `[a-z0-9_-]{1,32}` (`shared::view::key_ok`).
pub fn is_entry_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_ENTRY_ID
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// A file inside the plugin folder: relative, no `..`, no hidden parts, letters, digits, `.`,
/// `_`, `-` and `/`.
pub fn is_plugin_path(p: &str) -> bool {
    !p.is_empty()
        && p.len() <= MAX_PATH
        && !p.starts_with('/')
        && !p.ends_with('/')
        && p.split('/').all(|c| {
            !c.is_empty()
                && !c.starts_with('.')
                && c.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-')
        })
}

/// Icon given as an image file in the folder (else it is an icon theme name).
pub fn is_icon_file(s: &str) -> bool {
    [".png", ".svg"].iter().any(|e| s.ends_with(e))
}

/// A picture file of the store page: a plugin path ending `.png` or `.webp`.
pub fn is_listing_picture(s: &str) -> bool {
    [".png", ".webp"].iter().any(|e| s.ends_with(e)) && is_plugin_path(s)
}

/// An `https://` address of up to [`MAX_HOMEPAGE`] characters with a host and no user name:
/// the same rule as the hub's check of `homepage`.
pub fn is_homepage(s: &str) -> bool {
    s.len() <= MAX_HOMEPAGE
        && s.strip_prefix("https://").is_some_and(|rest| {
            let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
            !host.is_empty()
                && !host.contains('@')
                && host
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b".-:".contains(&c))
        })
        && s.bytes().all(|c| c.is_ascii_graphic())
}

/// Whether the WolOS version `v` (`0.13`, `0.13.1`) is at least `(major, minor)`.
pub fn wolos_at_least(v: &str, (major, minor): (u32, u32)) -> bool {
    let mut p = v.split('.').map(|x| x.parse::<u32>().ok());
    matches!((p.next().flatten(), p.next().flatten()), (Some(a), Some(b)) if (a, b) >= (major, minor))
}

fn is_icon(s: &str) -> bool {
    if is_icon_file(s) {
        return is_plugin_path(s);
    }
    !s.is_empty()
        && s.len() <= MAX_PATH
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-')
}

fn is_label(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= MAX_NAME && !s.chars().any(char::is_control)
}

/// `0.12` or `0.12.1`.
pub fn is_wolos_version(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    (2..=3).contains(&parts.len())
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 4 && p.bytes().all(|b| b.is_ascii_digit()))
}

const MODIFIERS: [&str; 4] = ["super", "ctrl", "alt", "shift"];

/// `super+shift+s`, `super+f5`, `super+space`: each modifier at most once, then one key (a letter
/// or digit, `f1`..`f24`, or a key name of lowercase letters). A plain key needs a modifier.
pub fn is_combo(s: &str) -> bool {
    let parts: Vec<&str> = s.split('+').collect();
    let Some((key, mods)) = parts.split_last() else {
        return false;
    };
    let mut seen: Vec<&str> = Vec::new();
    for m in mods {
        if !MODIFIERS.contains(m) || seen.contains(m) {
            return false;
        }
        seen.push(m);
    }
    let function = key
        .strip_prefix('f')
        .and_then(|n| n.parse::<u8>().ok())
        .is_some_and(|n| (1..=24).contains(&n));
    let plain = key.len() == 1
        && key
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
    let named = key.len() > 1
        && key.len() <= 16
        && key.bytes().all(|b| b.is_ascii_lowercase() || b == b'_');
    function || ((plain || named) && !mods.is_empty())
}

impl HotkeyEntry {
    /// (modifiers, key) of `default`.
    pub fn combo(&self) -> (Vec<&str>, &str) {
        let mut parts: Vec<&str> = self.default.split('+').collect();
        let key = parts.pop().unwrap_or_default();
        (parts, key)
    }
}

/// Checks a `[[hypr]]` fragment: `keyword = value` lines with keywords from [`HYPR_KEYWORDS`],
/// comments and blank lines. Returns the (keyword, value) pairs for `hyprprefs`.
pub fn check_hypr(text: &str) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let n = i + 1;
        let Some((k, v)) = line.split_once('=') else {
            return Err(format!(
                "Line {n} of the compositor fragment is not a keyword = value line."
            ));
        };
        let k = k.trim();
        let v = v.trim();
        if !HYPR_KEYWORDS.contains(&k) {
            return Err(format!(
                "Compositor fragments may only set {}; line {n} sets \"{k}\".",
                HYPR_KEYWORDS.join(", ")
            ));
        }
        // `$name` is a compositor variable (never from a plugin); a regex `$` anchor is fine.
        let variable = v
            .match_indices('$')
            .any(|(i, _)| v[i + 1..].starts_with(|c: char| c.is_ascii_alphabetic() || c == '_'));
        if v.is_empty() || v.contains(['{', '}']) || variable || v.chars().any(char::is_control) {
            return Err(format!(
                "Line {n} of the compositor fragment needs a plain value without $variables or braces."
            ));
        }
        if k == "windowrule" || k == "layerrule" {
            check_rule(n, k, v)?;
        }
        out.push((k.to_owned(), v.to_owned()));
    }
    Ok(out)
}

/// One `windowrule`/`layerrule` value, `effect [args], match, ...`: a listed effect, an opacity
/// the person can still see, and at least one match that names something (not `.*` or `.+`).
fn check_rule(n: usize, k: &str, v: &str) -> Result<(), String> {
    let mut fields = v.split(',').map(str::trim);
    let effect = fields.next().unwrap_or_default();
    let mut words = effect.split_whitespace();
    let name = words.next().unwrap_or_default().to_ascii_lowercase();
    let allowed: &[&str] = if k == "windowrule" {
        &HYPR_WINDOW_EFFECTS
    } else {
        &HYPR_LAYER_EFFECTS
    };
    if !allowed.contains(&name.as_str()) {
        return Err(format!(
            "Line {n} of the compositor fragment uses the {k} \"{name}\"; plugins may use {}.",
            allowed.join(", ")
        ));
    }
    if name == "opacity"
        && words
            .filter_map(|w| w.parse::<f64>().ok())
            .any(|o| o.is_nan() || o < HYPR_MIN_OPACITY)
    {
        return Err(format!(
            "Line {n} of the compositor fragment makes windows almost invisible; use an opacity from {HYPR_MIN_OPACITY} to 1."
        ));
    }
    let matches: Vec<&str> = fields.filter(|f| !f.is_empty()).collect();
    if matches.is_empty() || matches.iter().any(|m| matches_everything(m)) {
        return Err(format!(
            "Line {n} of the compositor fragment applies to every window; name the app, like class:^(org.example.App)$."
        ));
    }
    Ok(())
}

/// `class:.*`, `title:^(.+)$`, `.*`: a match that every window or surface passes.
fn matches_everything(m: &str) -> bool {
    // `class:`, `title:`, `initialClass:`...; a v1 rule or a layer namespace has no field.
    let pattern = match m.split_once(':') {
        Some((field, rest)) if field.chars().all(|c| c.is_ascii_alphanumeric()) => rest,
        _ => m,
    };
    let core: String = pattern
        .trim()
        .chars()
        .filter(|c| !matches!(c, '^' | '$' | '(' | ')'))
        .collect();
    core.is_empty() || core.chars().all(|c| matches!(c, '.' | '*' | '+' | '?'))
}

// ---- The TOML part plugin.toml uses ----

#[derive(Clone, Debug, PartialEq)]
enum Val {
    Str(String),
    Int(i64),
    Bool(bool),
    Arr(Vec<Val>),
    Table(Vec<(String, Val)>),
}

impl Val {
    fn kind(&self) -> &'static str {
        match self {
            Val::Str(_) => "text",
            Val::Int(_) => "a number",
            Val::Bool(_) => "true or false",
            Val::Arr(_) => "a list",
            Val::Table(_) => "a table",
        }
    }
}

struct Toml<'a> {
    s: &'a [u8],
    at: usize,
    line: usize,
    /// Lists open around the value being read (bounded by [`MAX_NESTING`]).
    depth: usize,
    /// The value being read is the header's `about`: `\n` may stand for a line break there.
    lines: bool,
}

/// Deepest list nesting plugin.toml may use (`sizes = [[2, 2]]` is 2): without a bound a file of
/// `[` recurses until the stack runs out (audit A25).
pub const MAX_NESTING: usize = 8;

type Line = usize;

impl<'a> Toml<'a> {
    fn err(&self, m: &str) -> String {
        format!("plugin.toml line {}: {m}", self.line)
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.at).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let c = self.peek()?;
        self.at += 1;
        if c == b'\n' {
            self.line += 1;
        }
        Some(c)
    }

    fn skip_blanks(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r')) {
            self.at += 1;
        }
    }

    /// Blanks, newlines and comments.
    fn skip_space(&mut self) {
        loop {
            self.skip_blanks();
            match self.peek() {
                Some(b'\n') => {
                    self.bump();
                }
                Some(b'#') => {
                    while !matches!(self.peek(), None | Some(b'\n')) {
                        self.at += 1;
                    }
                }
                _ => return,
            }
        }
    }

    /// End of a `key = value` or header line: blanks, an optional comment, newline or end.
    fn end_line(&mut self) -> Result<(), String> {
        self.skip_blanks();
        if self.peek() == Some(b'#') {
            while !matches!(self.peek(), None | Some(b'\n')) {
                self.at += 1;
            }
        }
        match self.peek() {
            None => Ok(()),
            Some(b'\n') => {
                self.bump();
                Ok(())
            }
            _ => Err(self.err("Only one key = value per line, then a newline.")),
        }
    }

    fn key(&mut self) -> Result<String, String> {
        let start = self.at;
        while self
            .peek()
            .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            self.at += 1;
        }
        if start == self.at {
            return Err(self.err(
                "Expected a key of letters, digits, _ or - (quoted and dotted keys are not used in plugin.toml).",
            ));
        }
        Ok(String::from_utf8_lossy(&self.s[start..self.at]).into_owned())
    }

    fn string(&mut self, quote: u8) -> Result<String, String> {
        self.at += 1;
        if self.s[self.at..].starts_with(&[quote, quote]) {
            return Err(self.err("Multi-line strings are not used in plugin.toml; use one line."));
        }
        let mut out = String::new();
        let mut buf = Vec::new();
        loop {
            match self.peek() {
                None | Some(b'\n') => {
                    return Err(self.err("A string is missing its closing quote."));
                }
                Some(c) if c == quote => {
                    self.at += 1;
                    break;
                }
                Some(b'\\') if quote == b'"' => {
                    self.at += 1;
                    out.push_str(&String::from_utf8_lossy(&buf));
                    buf.clear();
                    let e = self.bump().unwrap_or(b' ');
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'n' => out.push('\n'),
                        b't' => out.push('\t'),
                        b'r' => out.push('\r'),
                        b'u' | b'U' => {
                            let n = if e == b'u' { 4 } else { 8 };
                            let hex = self.s.get(self.at..self.at + n).ok_or_else(|| {
                                self.err("A \\u escape needs 4 hex digits (\\U needs 8).")
                            })?;
                            let hex = std::str::from_utf8(hex).unwrap_or("x");
                            let c = u32::from_str_radix(hex, 16)
                                .ok()
                                .and_then(char::from_u32)
                                .ok_or_else(|| self.err("A \\u escape needs hex digits."))?;
                            self.at += n;
                            out.push(c);
                        }
                        other => {
                            return Err(self.err(&format!(
                                "Unknown escape \\{}; use \\\" \\\\ \\n \\t or \\uXXXX.",
                                other as char
                            )));
                        }
                    }
                }
                Some(c) => {
                    buf.push(c);
                    self.at += 1;
                }
            }
        }
        out.push_str(&String::from_utf8_lossy(&buf));
        if out
            .chars()
            .any(|c| c.is_control() && c != '\t' && !(self.lines && c == '\n'))
        {
            return Err(self.err("Strings cannot contain control characters."));
        }
        Ok(out)
    }

    /// Items after a `[`, through the closing `]`.
    fn list(&mut self) -> Result<Vec<Val>, String> {
        let mut items = Vec::new();
        loop {
            self.skip_space();
            match self.peek() {
                None => return Err(self.err("A list is missing its closing ].")),
                Some(b']') => {
                    self.at += 1;
                    break;
                }
                Some(b',') => {
                    return Err(self.err("A list has an extra comma."));
                }
                _ => {}
            }
            items.push(self.value()?);
            self.skip_space();
            match self.peek() {
                Some(b',') => {
                    self.at += 1;
                }
                Some(b']') => {}
                _ => return Err(self.err("List items are separated by commas.")),
            }
        }
        Ok(items)
    }

    fn value(&mut self) -> Result<Val, String> {
        match self.peek() {
            Some(q @ (b'"' | b'\'')) => Ok(Val::Str(self.string(q)?)),
            Some(b'[') => {
                if self.depth >= MAX_NESTING {
                    return Err(
                        self.err(&format!("Lists are nested more than {MAX_NESTING} deep."))
                    );
                }
                self.at += 1;
                self.depth += 1;
                let items = self.list();
                self.depth -= 1;
                Ok(Val::Arr(items?))
            }
            Some(b'{') => {
                Err(self
                    .err("Inline tables are not used in plugin.toml; write a [[section]] instead."))
            }
            Some(b't') if self.s[self.at..].starts_with(b"true") => {
                self.at += 4;
                Ok(Val::Bool(true))
            }
            Some(b'f') if self.s[self.at..].starts_with(b"false") => {
                self.at += 5;
                Ok(Val::Bool(false))
            }
            Some(b'0'..=b'9' | b'-' | b'+') => {
                let start = self.at;
                self.at += 1;
                while self.peek().is_some_and(|b| {
                    b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b':' || b == b'-'
                }) {
                    self.at += 1;
                }
                let text = String::from_utf8_lossy(&self.s[start..self.at]).replace('_', "");
                text.parse::<i64>().map(Val::Int).map_err(|_| {
                    self.err(&format!(
                        "\"{text}\" is not a whole number (floats and dates are not used in plugin.toml)."
                    ))
                })
            }
            _ => {
                Err(self
                    .err("Expected a value: \"text\", a whole number, true, false or a [list]."))
            }
        }
    }

    /// The whole document as a root table; `[[name]]` sections become lists of tables.
    fn document(&mut self) -> Result<Vec<(String, Val, Line)>, String> {
        let mut root: Vec<(String, Val, Line)> = Vec::new();
        // Index into `root` of the section new keys go to; None = the header.
        let mut current: Option<usize> = None;
        loop {
            self.skip_space();
            let Some(c) = self.peek() else {
                return Ok(root);
            };
            if c == b'[' {
                self.at += 1;
                let array = self.peek() == Some(b'[');
                if array {
                    self.at += 1;
                }
                self.skip_blanks();
                let name = self.key()?;
                self.skip_blanks();
                if self.peek() == Some(b'.') {
                    return Err(self.err(
                        "Sections inside sections are not used in plugin.toml; use [[menubar]], [[dock]], ...",
                    ));
                }
                let close: &[u8] = if array { b"]]" } else { b"]" };
                if !self.s[self.at..].starts_with(close) {
                    return Err(
                        self.err(&format!("A section header is [[{name}]] on its own line."))
                    );
                }
                self.at += close.len();
                self.end_line()?;
                if !array {
                    return Err(self.err(&format!(
                        "Write [[{name}]] (two brackets): capability sections are lists."
                    )));
                }
                if Kind::parse(&name).is_none() {
                    return Err(self.err(&format!(
                        "Unknown section [[{name}]]; plugin.toml has {}.",
                        Kind::ALL.map(|k| format!("[[{}]]", k.id())).join(", ")
                    )));
                }
                let i = match root.iter().position(|(k, _, _)| *k == name) {
                    Some(i) => i,
                    None => {
                        root.push((name, Val::Arr(Vec::new()), self.line));
                        root.len() - 1
                    }
                };
                if let Val::Arr(list) = &mut root[i].1 {
                    list.push(Val::Table(Vec::new()));
                }
                current = Some(i);
                continue;
            }
            let line = self.line;
            let key = self.key()?;
            self.skip_blanks();
            if self.peek() != Some(b'=') {
                return Err(self.err(&format!("Expected = after \"{key}\".")));
            }
            self.at += 1;
            self.skip_blanks();
            self.lines = current.is_none() && key == "about";
            let value = self.value();
            self.lines = false;
            let value = value?;
            self.end_line()?;
            match current {
                None => {
                    if root.iter().any(|(k, _, _)| *k == key) {
                        return Err(format!("plugin.toml line {line}: \"{key}\" is set twice."));
                    }
                    if Kind::parse(&key).is_some() {
                        return Err(format!(
                            "plugin.toml line {line}: \"{key}\" is a section: write [[{key}]] on its own line."
                        ));
                    }
                    root.push((key, value, line));
                }
                Some(i) => {
                    let Val::Arr(list) = &mut root[i].1 else {
                        unreachable!("sections are lists")
                    };
                    let Some(Val::Table(t)) = list.last_mut() else {
                        unreachable!("a section was just opened")
                    };
                    if t.iter().any(|(k, _)| *k == key) {
                        return Err(format!("plugin.toml line {line}: \"{key}\" is set twice."));
                    }
                    t.push((key, value));
                }
            }
        }
    }
}

// ---- Validation ----

struct Field<'a> {
    where_: String,
    table: &'a [(String, Val)],
}

impl Field<'_> {
    fn get(&self, key: &str) -> Option<&Val> {
        self.table.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    fn missing(&self, key: &str) -> String {
        format!("{} needs {key}.", self.where_)
    }

    fn text(&self, key: &str) -> Result<Option<&str>, String> {
        match self.get(key) {
            None => Ok(None),
            Some(Val::Str(s)) => Ok(Some(s)),
            Some(v) => Err(format!(
                "{}: {key} is text in quotes, not {}.",
                self.where_,
                v.kind()
            )),
        }
    }

    fn required(&self, key: &str) -> Result<&str, String> {
        self.text(key)?.ok_or_else(|| self.missing(key))
    }

    fn list(&self, key: &str) -> Result<Option<Vec<String>>, String> {
        match self.get(key) {
            None => Ok(None),
            Some(Val::Arr(items)) => items
                .iter()
                .map(|v| match v {
                    Val::Str(s) => Ok(s.clone()),
                    other => Err(format!(
                        "{}: {key} is a list of text in quotes, not {}.",
                        self.where_,
                        other.kind()
                    )),
                })
                .collect::<Result<Vec<_>, _>>()
                .map(Some),
            Some(v) => Err(format!(
                "{}: {key} is a list like [\"a\", \"b\"], not {}.",
                self.where_,
                v.kind()
            )),
        }
    }

    /// argv: at least a program, nothing empty.
    fn argv(&self, key: &str) -> Result<Vec<String>, String> {
        let v = self.list(key)?.ok_or_else(|| self.missing(key))?;
        if v.is_empty()
            || v.iter()
                .any(|a| a.is_empty() || a.chars().any(char::is_control))
        {
            return Err(format!(
                "{}: {key} is the command as a list, like [\"wol\", \"focus\", \"on\"].",
                self.where_
            ));
        }
        Ok(v)
    }

    fn label(&self, key: &str) -> Result<String, String> {
        let s = self.required(key)?;
        if !is_label(s) {
            return Err(format!(
                "{}: {key} is one line of 1 to {MAX_NAME} characters.",
                self.where_
            ));
        }
        Ok(s.to_owned())
    }

    fn path(&self, key: &str, ext: &str) -> Result<String, String> {
        let s = self.required(key)?;
        if !is_plugin_path(s) || !s.ends_with(ext) {
            return Err(format!(
                "{}: {key} is a {ext} file inside the plugin folder, like \"{}{ext}\".",
                self.where_,
                key.replace("script", "item")
            ));
        }
        Ok(s.to_owned())
    }

    fn icon(&self, key: &str) -> Result<String, String> {
        let s = self.required(key)?;
        if !is_icon(s) {
            return Err(format!(
                "{}: {key} is a symbolic icon name or a .png or .svg file in the plugin folder.",
                self.where_
            ));
        }
        Ok(s.to_owned())
    }

    fn choice<T: Copy>(
        &self,
        key: &str,
        parse: fn(&str) -> Option<T>,
        all: &[&str],
        default: T,
    ) -> Result<T, String> {
        match self.text(key)? {
            None => Ok(default),
            Some(s) => parse(s)
                .ok_or_else(|| format!("{}: {key} is one of {}.", self.where_, all.join(", "))),
        }
    }

    fn only(&self, keys: &[&str]) -> Result<(), String> {
        if let Some((k, _)) = self.table.iter().find(|(k, _)| !keys.contains(&k.as_str())) {
            return Err(format!(
                "{}: unknown key \"{k}\"; it takes {}.",
                self.where_,
                keys.join(", ")
            ));
        }
        Ok(())
    }
}

/// The raw `plugin.toml` document as JSON (sections as arrays of objects), before validation:
/// what [`SCHEMA_JSON`] describes, for validators and the Plugin Maker.
pub fn to_json(text: &str) -> Result<serde_json::Value, String> {
    fn conv(v: &Val) -> serde_json::Value {
        match v {
            Val::Str(s) => serde_json::Value::String(s.clone()),
            Val::Int(n) => serde_json::Value::from(*n),
            Val::Bool(b) => serde_json::Value::Bool(*b),
            Val::Arr(a) => serde_json::Value::Array(a.iter().map(conv).collect()),
            Val::Table(t) => {
                serde_json::Value::Object(t.iter().map(|(k, v)| (k.clone(), conv(v))).collect())
            }
        }
    }
    let mut t = Toml {
        s: text.as_bytes(),
        at: 0,
        line: 1,
        depth: 0,
        lines: false,
    };
    let root = t.document()?;
    Ok(serde_json::Value::Object(
        root.iter().map(|(k, v, _)| (k.clone(), conv(v))).collect(),
    ))
}

/// `plugin.toml` text to a validated manifest, or one sentence to fix.
/// The store-page keys of the header ([`LISTING_KEYS`]), checked as the hub's check reads them.
#[derive(Default)]
struct Listing {
    icon: String,
    screenshots: Vec<String>,
    categories: Vec<String>,
    homepage: String,
    about: String,
}

fn parse_listing(h: &Field, min_wolos: &str) -> Result<Listing, String> {
    let mut l = Listing::default();
    let used: Vec<&str> = LISTING_KEYS
        .into_iter()
        .filter(|k| h.get(k).is_some())
        .collect();
    if used.is_empty() {
        return Ok(l);
    }
    if !wolos_at_least(min_wolos, LISTING_MIN_WOLOS) {
        return Err(format!(
            "plugin.toml: {} need WolOS {}.{}; add min_wolos = \"{}.{}\" (older systems refuse these keys).",
            used.join(", "),
            LISTING_MIN_WOLOS.0,
            LISTING_MIN_WOLOS.1,
            LISTING_MIN_WOLOS.0,
            LISTING_MIN_WOLOS.1
        ));
    }
    if let Some(i) = h.text("icon")? {
        if !is_listing_picture(i) {
            return Err("plugin.toml: icon is a .png or .webp file inside the plugin folder, like \"icon.png\".".into());
        }
        l.icon = i.to_owned();
    }
    for p in h.list("screenshots")?.unwrap_or_default() {
        if !is_listing_picture(&p) {
            return Err(format!(
                "plugin.toml: screenshots are .png or .webp files inside the plugin folder; \"{p}\" is not."
            ));
        }
        if l.screenshots.contains(&p) {
            return Err(format!(
                "plugin.toml: the screenshot \"{p}\" is listed twice."
            ));
        }
        l.screenshots.push(p);
    }
    if l.screenshots.len() > MAX_SCREENSHOTS {
        return Err(format!(
            "plugin.toml: screenshots has {} files; the store shows at most {MAX_SCREENSHOTS}.",
            l.screenshots.len()
        ));
    }
    for c in h.list("categories")?.unwrap_or_default() {
        if !crate::package::CATEGORIES.contains(&c.as_str()) {
            return Err(format!(
                "plugin.toml: categories are any of {}; \"{c}\" is not.",
                crate::package::CATEGORIES.join(", ")
            ));
        }
        if !l.categories.contains(&c) {
            l.categories.push(c);
        }
    }
    if let Some(u) = h.text("homepage")? {
        if !is_homepage(u) {
            return Err(format!(
                "plugin.toml: homepage is an https:// address of up to {MAX_HOMEPAGE} characters."
            ));
        }
        l.homepage = u.to_owned();
    }
    if let Some(a) = h.text("about")? {
        if a.chars().count() > MAX_ABOUT {
            return Err(format!(
                "plugin.toml: about is up to {MAX_ABOUT} characters (line breaks written \\n)."
            ));
        }
        l.about = a.to_owned();
    }
    Ok(l)
}

pub fn parse(text: &str) -> Result<Manifest, String> {
    let mut t = Toml {
        s: text.as_bytes(),
        at: 0,
        line: 1,
        depth: 0,
        lines: false,
    };
    let root = t.document()?;
    let header: Vec<(String, Val)> = root
        .iter()
        .filter(|(k, _, _)| Kind::parse(k).is_none())
        .map(|(k, v, _)| (k.clone(), v.clone()))
        .collect();
    let h = Field {
        where_: "plugin.toml".into(),
        table: &header,
    };
    h.only(&HEADER_KEYS)?;
    let schema = match h.get("schema") {
        Some(Val::Int(n)) if *n == i64::from(SCHEMA) => SCHEMA,
        Some(Val::Int(n)) if *n > i64::from(SCHEMA) => {
            return Err(format!(
                "This plugin needs manifest schema {n}; this WolOS reads schema {SCHEMA}."
            ));
        }
        _ => return Err(format!("plugin.toml starts with schema = {SCHEMA}.")),
    };
    let id = h.required("id")?;
    if !is_plugin_id(id) {
        return Err(format!(
            "id is a reverse-DNS or user.name id of lowercase labels, like \"org.example.stats\", up to {MAX_ID} characters."
        ));
    }
    let name = h.label("name")?;
    let version = h.required("version")?;
    if !crate::package::is_version(version) {
        return Err("version is letters, digits and . _ + ~ -, like \"1.0\".".into());
    }
    let license = h.required("license")?;
    if !is_label(license) {
        return Err("license is an SPDX id, like \"MIT\" or \"GPL-2.0-or-later\".".into());
    }
    let author = h.text("author")?.unwrap_or_default();
    if author.len() > MAX_NAME || author.chars().any(char::is_control) {
        return Err(format!(
            "author is one line of up to {MAX_NAME} characters."
        ));
    }
    let description = h.text("description")?.unwrap_or_default();
    if description.len() > MAX_DESCRIPTION || description.chars().any(char::is_control) {
        return Err(format!(
            "description is one line of up to {MAX_DESCRIPTION} characters."
        ));
    }
    let min_wolos = h.text("min_wolos")?.unwrap_or_default();
    if !min_wolos.is_empty() && !is_wolos_version(min_wolos) {
        return Err("min_wolos is a WolOS version like \"0.12\".".into());
    }
    let mut permissions = Vec::new();
    for p in h.list("permissions")?.unwrap_or_default() {
        let Some(perm) = Permission::parse(&p) else {
            return Err(format!(
                "Plugins cannot ask for the permission \"{p}\"; known: {}.",
                Permission::ALL.map(Permission::id).join(", ")
            ));
        };
        if !permissions.contains(&perm) {
            permissions.push(perm);
        }
    }
    permissions.sort();
    let listing = parse_listing(&h, min_wolos)?;
    let mut m = Manifest {
        schema,
        id: id.to_owned(),
        name,
        version: version.to_owned(),
        author: author.to_owned(),
        license: license.to_owned(),
        description: description.to_owned(),
        min_wolos: min_wolos.to_owned(),
        permissions,
        icon: listing.icon,
        screenshots: listing.screenshots,
        categories: listing.categories,
        homepage: listing.homepage,
        about: listing.about,
        ..Manifest::default()
    };
    let mut ids: Vec<String> = Vec::new();
    for (key, val, _) in &root {
        let Some(kind) = Kind::parse(key) else {
            continue;
        };
        let Val::Arr(list) = val else {
            unreachable!("sections are lists")
        };
        for (i, item) in list.iter().enumerate() {
            let Val::Table(table) = item else {
                unreachable!("section items are tables")
            };
            let f = Field {
                where_: format!("[[{}]] {}", kind.id(), i + 1),
                table,
            };
            f.only(kind.keys())?;
            let id = f.required("id")?;
            if !is_entry_id(id) {
                return Err(format!(
                    "{}: id is 1 to {MAX_ENTRY_ID} lowercase letters, digits, - or _.",
                    f.where_
                ));
            }
            if ids.contains(&id.to_owned()) {
                return Err(format!(
                    "{}: the id \"{id}\" is used twice in this plugin.",
                    f.where_
                ));
            }
            ids.push(id.to_owned());
            let id = id.to_owned();
            match kind {
                Kind::Menubar => m.menubar.push(MenubarEntry {
                    id,
                    script: f.path("script", ".lua")?,
                    side: f.choice(
                        "side",
                        Side::parse,
                        &Side::ALL.map(Side::id),
                        Side::default(),
                    )?,
                }),
                Kind::Dock => m.dock.push(DockEntry {
                    id,
                    name: f.label("name")?,
                    icon: f.icon("icon")?,
                    exec: f.argv("exec")?,
                }),
                Kind::Widget => {
                    let mut sizes = Vec::new();
                    for s in f.list("sizes")?.unwrap_or_default() {
                        let Some(z) = Size::parse(&s) else {
                            return Err(format!(
                                "{}: sizes are any of small, medium, large.",
                                f.where_
                            ));
                        };
                        if !sizes.contains(&z) {
                            sizes.push(z);
                        }
                    }
                    if sizes.is_empty() {
                        sizes = Size::ALL.to_vec();
                    }
                    sizes.sort();
                    m.widget.push(WidgetEntry {
                        id,
                        script: f.path("script", ".lua")?,
                        sizes,
                    });
                }
                Kind::Panel => m.panel.push(PanelEntry {
                    id,
                    script: f.path("script", ".lua")?,
                    anchor: f.choice(
                        "anchor",
                        Anchor::parse,
                        &Anchor::ALL.map(Anchor::id),
                        Anchor::default(),
                    )?,
                }),
                Kind::Search => {
                    let kind = f.choice(
                        "kind",
                        SearchKind::parse,
                        &SearchKind::ALL.map(SearchKind::id),
                        SearchKind::default(),
                    )?;
                    let prefix = f.text("prefix")?.unwrap_or_default();
                    match kind {
                        SearchKind::Command
                            if prefix.is_empty()
                                || prefix.len() > MAX_PREFIX
                                || prefix.chars().any(|c| c.is_whitespace() || c.is_control()) =>
                        {
                            return Err(format!(
                                "{}: a command search needs a prefix of 1 to {MAX_PREFIX} characters without spaces, like \"=\".",
                                f.where_
                            ));
                        }
                        SearchKind::Results if !prefix.is_empty() => {
                            return Err(format!(
                                "{}: prefix is only for kind = \"command\".",
                                f.where_
                            ));
                        }
                        _ => {}
                    }
                    m.search.push(SearchEntry {
                        id,
                        script: f.path("script", ".lua")?,
                        kind,
                        prefix: prefix.to_owned(),
                    });
                }
                Kind::Tile => {
                    let toggle = f.required("toggle")?;
                    if !is_entry_id(toggle) {
                        return Err(format!(
                            "{}: toggle is the id of a [[toggle]] in this plugin.",
                            f.where_
                        ));
                    }
                    m.tile.push(TileEntry {
                        id,
                        icon: f.icon("icon")?,
                        label: f.label("label")?,
                        toggle: toggle.to_owned(),
                    });
                }
                Kind::Toggle => m.toggle.push(ToggleEntry {
                    id,
                    on: f.argv("on")?,
                    off: f.argv("off")?,
                }),
                Kind::Hotkey => {
                    let default = f.required("default")?.to_ascii_lowercase();
                    if !is_combo(&default) {
                        return Err(format!(
                            "{}: default is a key combo like \"super+shift+s\" (modifiers: super, ctrl, alt, shift).",
                            f.where_
                        ));
                    }
                    let group = f.text("group")?.unwrap_or_default();
                    if !group.is_empty() && !is_label(group) {
                        return Err(format!(
                            "{}: group is one line of 1 to {MAX_NAME} characters.",
                            f.where_
                        ));
                    }
                    m.hotkey.push(HotkeyEntry {
                        id,
                        default,
                        exec: f.argv("exec")?,
                        group: group.to_owned(),
                    });
                }
                Kind::Webapp => {
                    let url = f.required("url")?;
                    if !(url.starts_with("https://") || url.starts_with("http://"))
                        || crate::widgets::manifest::url_host(url).is_none()
                        || url.chars().any(|c| c.is_whitespace() || c.is_control())
                    {
                        return Err(format!(
                            "{}: url is an https:// or http:// address.",
                            f.where_
                        ));
                    }
                    m.webapp.push(WebappEntry {
                        id,
                        name: f.label("name")?,
                        url: url.to_owned(),
                        icon: f.icon("icon")?,
                        profile: f.choice(
                            "profile",
                            Profile::parse,
                            &Profile::ALL.map(Profile::id),
                            Profile::default(),
                        )?,
                    });
                }
                Kind::Theme => m.theme.push(FileEntry {
                    id,
                    file: f.path("file", ".json")?,
                }),
                Kind::Look => m.look.push(FileEntry {
                    id,
                    file: f.path("file", ".json")?,
                }),
                Kind::Hypr => m.hypr.push(FileEntry {
                    id,
                    file: f.path("file", ".conf")?,
                }),
                Kind::Ipc => m.ipc.push(IpcEntry {
                    id,
                    script: f.path("script", ".lua")?,
                    args: f.choice(
                        "args",
                        ArgType::parse,
                        &ArgType::ALL.map(ArgType::id),
                        ArgType::default(),
                    )?,
                }),
            }
        }
    }
    for t in &m.tile {
        if !m.toggle.iter().any(|g| g.id == t.toggle) {
            return Err(format!(
                "[[tile]] \"{}\" uses the toggle \"{}\", which this plugin does not define.",
                t.id, t.toggle
            ));
        }
    }
    if !m.has(Permission::Run)
        && let Some((k, id, argv)) = m.commands().first()
    {
        return Err(format!(
            "[[{}]] \"{id}\" runs \"{}\", which is not a wol command: add \"run\" to permissions so the person is asked first.",
            k.id(),
            argv.first().map(String::as_str).unwrap_or_default()
        ));
    }
    if m.entries().is_empty() {
        return Err(format!(
            "The plugin does nothing yet: add a section such as {}.",
            Kind::ALL.map(|k| format!("[[{}]]", k.id())).join(", ")
        ));
    }
    Ok(m)
}

/// `org.wolos` itself or `org.wolos.*`.
pub fn is_reserved_id(id: &str) -> bool {
    id == RESERVED_PREFIX.trim_end_matches('.') || id.starts_with(RESERVED_PREFIX)
}

/// argv that calls a `wol` verb (argv[0] is `wol`), so the helper policy applies.
pub fn is_wol_argv(argv: &[String]) -> bool {
    argv.first().is_some_and(|a| a == "wol")
}

/// The argv that launches a plugin command (`wol.run`, `[[toggle]]`, `[[dock]]`, `[[hotkey]]`):
/// a `wol` verb runs as the plugin's helper (`env WOL_HELPER=<plugin> wol ...`), so the helper
/// policy, the approval sheet and the audit log apply; anything else runs unchanged. Every
/// launcher builds its argv here (QA-PLUG-5: the Dock spawned `wol` verbs without it).
pub fn helper_argv(plugin: &str, argv: &[String]) -> Vec<String> {
    let mut cmd: Vec<String> = Vec::new();
    if is_wol_argv(argv) {
        cmd.extend(["env".to_owned(), format!("WOL_HELPER={plugin}")]);
    }
    cmd.extend(argv.iter().cloned());
    cmd
}

#[cfg(test)]
mod tests;

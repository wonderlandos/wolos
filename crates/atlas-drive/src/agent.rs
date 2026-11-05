// SPDX-License-Identifier: GPL-2.0-or-later
//! The agent view of a page: the accessibility tree flattened to one line per interactive or
//! textual node, with stable refs (`e12`), deterministic truncation by priority, a diff against
//! the previous snapshot and a token estimate. Pure functions over the CDP `AXNode` list, so the
//! whole thing is tested without a browser (tests/fixtures).
//!
//! Line grammar (one node per line, indentation = landmark depth):
//!   `@ <url> "<title>"`                       the first line
//!   `[e12] button "Save" disabled`            interactive node: ref, role, name, state words
//!   `[e13] textbox "Email" value="" required`
//!   `[e2] navigation "Main"`                  landmark / region / dialog (children indented)
//!   `[e5] h2 "Pricing"`                       heading (level in the role word)
//!   `[e9] alert "Saved"`                      live region read in one line
//!   `img "Logo"`, `"a text run"`              no ref: nothing to act on
use crate::util::{fnv, tokens};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

pub const MAX_NAME: usize = 120;
pub const MAX_TEXT: usize = 240;
pub const MAX_DEPTH: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// can be acted on (button, link, textbox...)
    Interactive,
    /// landmark, region, dialog, table, form: a container with a ref
    Structure,
    Heading,
    /// alert, status, log: read in one line
    Live,
    Text,
    Image,
}

#[derive(Debug, Clone)]
pub struct Line {
    pub kind: Kind,
    pub depth: usize,
    /// The text after the ref, e.g. `button "Save" disabled`.
    pub body: String,
    pub backend: Option<i64>,
    pub ref_n: Option<u32>,
    /// Nearest ancestor with a ref (for text lines found by `find`).
    pub parent_ref: Option<u32>,
    /// Boilerplate group id (0 = none): index into the structure line that opened it.
    pub boiler: u32,
    pub focused: bool,
    /// Set when geometry was fetched: Some(true) if any part is in the viewport.
    pub in_view: Option<bool>,
    /// Role word (lowercase) for `find role=`.
    pub role: String,
    pub name: String,
    pub value: String,
    pub node_id: String,
    /// Ancestor chain of structure node ids (nearest last), for the focus region.
    pub region: String,
    pub href: String,
}

impl Line {
    pub fn render(&self) -> String {
        let pad = " ".repeat(self.depth.min(MAX_DEPTH));
        match self.ref_n {
            Some(n) => format!("{pad}[e{n}] {}", self.body),
            None => format!("{pad}{}", self.body),
        }
    }
    /// The diff key: the ref when there is one, else a hash of the text.
    pub fn key(&self, occurrence: u32) -> String {
        match self.ref_n {
            Some(n) => format!("e{n}"),
            None => format!("t{:x}.{occurrence}", fnv(self.body.as_bytes())),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RefInfo {
    pub backend: i64,
    pub role: String,
    pub name: String,
}

/// Per-tab memory of the agent view. Refs are valid until the next navigation (`nav_gen` changes).
#[derive(Default)]
pub struct AgentState {
    pub generation: u64,
    pub by_backend: HashMap<i64, u32>,
    pub info: HashMap<u32, RefInfo>,
    pub next: u32,
    /// Last rendered snapshot (key, rendered text) for `--diff`, and the options it was made with.
    pub last: Vec<(String, String)>,
    pub last_sig: String,
    /// Boilerplate seen on earlier pages: hash of the section's text.
    pub boiler_seen: HashSet<u64>,
    /// The trap scan of the current document and the fingerprint it was made at.
    pub traps_cache: Option<(u64, String, serde_json::Value)>,
    pub boiler_gen: HashMap<u64, u64>,
    /// Drive's isolated world in this document (src/mirror.rs).
    pub world: Option<i64>,
    /// Mirror keys already turned into backend node ids.
    pub real: HashMap<i64, i64>,
}

impl AgentState {
    /// Forget the refs when the document changed.
    pub fn sync_gen(&mut self, generation: u64) {
        if self.generation != generation {
            self.generation = generation;
            self.by_backend.clear();
            self.info.clear();
            self.next = 0;
            self.last.clear();
            self.last_sig.clear();
            self.world = None;
            self.real.clear();
        }
    }
    pub fn assign(&mut self, backend: i64, role: &str, name: &str) -> u32 {
        if let Some(n) = self.by_backend.get(&backend) {
            if let Some(i) = self.info.get_mut(n) {
                i.role = role.into();
                i.name = name.into();
            }
            return *n;
        }
        self.next += 1;
        let n = self.next;
        self.by_backend.insert(backend, n);
        self.info.insert(
            n,
            RefInfo {
                backend,
                role: role.into(),
                name: name.into(),
            },
        );
        n
    }
}

#[derive(Debug, Clone, Default)]
pub struct Options {
    pub interactive_only: bool,
    pub depth: Option<usize>,
    pub max_tokens: Option<usize>,
    pub diff: bool,
    pub region: Option<i64>,
    pub full: bool,
    pub viewport_only: bool,
    /// Show every link's address (default: only for links without a name).
    pub hrefs: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Rendered {
    pub text: String,
    pub tokens: usize,
    pub nodes: usize,
    pub omitted: usize,
    pub collapsed: usize,
}

// ------------------------------------------------------------------------- AX tree access

struct Ax<'a> {
    nodes: HashMap<&'a str, &'a Value>,
    roots: Vec<&'a str>,
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

fn val_str(v: &Value) -> String {
    match v.get("value") {
        Some(Value::String(t)) => t.clone(),
        Some(Value::Null) | None => String::new(),
        Some(o) => o.to_string(),
    }
}

fn prop<'a>(n: &'a Value, name: &str) -> Option<&'a Value> {
    n.get("properties")?
        .as_array()?
        .iter()
        .find(|p| p["name"] == name)
        .map(|p| &p["value"])
}

fn prop_bool(n: &Value, name: &str) -> bool {
    prop(n, name)
        .map(|v| v["value"] == true || v["value"] == "true")
        .unwrap_or(false)
}

fn index(nodes: &[Value]) -> Ax<'_> {
    let mut map = HashMap::new();
    for n in nodes {
        map.insert(s(n, "nodeId"), n);
    }
    let mut roots = vec![];
    for n in nodes {
        let p = s(n, "parentId");
        if p.is_empty() || !map.contains_key(p) {
            roots.push(s(n, "nodeId"));
        }
    }
    Ax { nodes: map, roots }
}

const INTERACTIVE: &[&str] = &[
    "button",
    "link",
    "textbox",
    "searchbox",
    "combobox",
    "checkbox",
    "radio",
    "switch",
    "slider",
    "spinbutton",
    "menuitem",
    "menuitemcheckbox",
    "menuitemradio",
    "tab",
    "option",
    "treeitem",
    "textarea",
    "listbox",
    "scrollbar",
    "colorwell",
    "DisclosureTriangle",
    "PopUpButton",
    "ComboBoxSelect",
    "ListBoxOption",
    "TextField",
    "SearchBox",
    "Switch",
    "ToggleButton",
    "MenuListOption",
    "MenuListPopup",
    "ComboBoxGrouping",
    "ComboBoxMenuButton",
    "RadioButton",
    "CheckBox",
    "Button",
    "Link",
    "TextFieldWithComboBox",
    "SpinButton",
    "Slider",
    "Tab",
    "TreeItem",
    "MenuItem",
];
const LANDMARK: &[&str] = &[
    "banner",
    "navigation",
    "main",
    "complementary",
    "contentinfo",
    "search",
    "form",
    "region",
    "dialog",
    "alertdialog",
    "article",
    "table",
    "grid",
    "tablist",
    "toolbar",
    "menu",
    "menubar",
    "tree",
    "radiogroup",
    "figure",
    "Section",
    "Dialog",
];
const BOILER: &[&str] = &["banner", "navigation", "contentinfo", "complementary"];
const LIVE: &[&str] = &[
    "alert", "status", "log", "marquee", "timer", "tooltip", "Alert", "Status",
];
const SKIP_DESCEND: &[&str] = &[
    "button",
    "link",
    "textbox",
    "searchbox",
    "checkbox",
    "radio",
    "switch",
    "slider",
    "spinbutton",
    "menuitem",
    "menuitemcheckbox",
    "menuitemradio",
    "tab",
    "option",
    "treeitem",
    "textarea",
    "scrollbar",
    "colorwell",
    "DisclosureTriangle",
    "PopUpButton",
    "TextField",
    "SearchBox",
    "Switch",
    "ToggleButton",
    "RadioButton",
    "CheckBox",
    "Button",
    "Link",
    "SpinButton",
    "Slider",
    "Tab",
    "TreeItem",
    "MenuItem",
    "ListBoxOption",
    "MenuListOption",
    "TextFieldWithComboBox",
];

fn canon_role(raw: &str) -> String {
    match raw {
        "TextField" | "textbox" | "TextFieldWithComboBox" => "textbox",
        "SearchBox" => "searchbox",
        "Button" | "PopUpButton" | "ToggleButton" | "DisclosureTriangle" | "ComboBoxMenuButton" => {
            "button"
        }
        "Link" => "link",
        "CheckBox" => "checkbox",
        "RadioButton" => "radio",
        "Switch" => "switch",
        "Slider" => "slider",
        "SpinButton" => "spinbutton",
        "Tab" => "tab",
        "TreeItem" => "treeitem",
        "MenuItem" => "menuitem",
        "ListBoxOption" | "MenuListOption" => "option",
        "ComboBoxSelect" => "combobox",
        "Dialog" => "dialog",
        "Alert" => "alert",
        "Status" => "status",
        "Section" => "region",
        other => other,
    }
    .to_string()
}

fn squeeze(t: &str, max: usize) -> String {
    let mut out = String::new();
    let mut last_space = true;
    for ch in t.chars() {
        if ch.is_whitespace() || ch == '\u{a0}' {
            if !last_space {
                out.push(' ');
            }
            last_space = true;
        } else {
            out.push(ch);
            last_space = false;
        }
    }
    let out = out.trim_end().to_string();
    if out.chars().count() > max {
        let cut: String = out.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    } else {
        out
    }
}

fn quote(t: &str) -> String {
    format!("\"{}\"", t.replace('\\', "\\\\").replace('"', "\\\""))
}

// ------------------------------------------------------------------------- building lines

struct Builder<'a> {
    ax: Ax<'a>,
    lines: Vec<Line>,
    opts: &'a Options,
    text: String,
    text_depth: usize,
    text_region: String,
    /// The first text node of the run being collected: `find "words"` can point at it.
    text_backend: Option<i64>,
    struct_stack: Vec<u32>,
    region_stack: Vec<String>,
    boiler_stack: Vec<u32>,
    boiler_id: u32,
    depth_limit: usize,
    ax_depth: usize,
    started: bool,
    /// Nodes already walked: a malformed (cyclic) tree is walked once, not forever.
    seen: HashSet<&'a str>,
}

/// Deeper than this is not walked (a stack of 2 MiB must survive a hostile page).
pub const MAX_WALK_DEPTH: usize = 500;

impl<'a> Builder<'a> {
    fn flush(&mut self) {
        let t = squeeze(&self.text, MAX_TEXT);
        self.text.clear();
        if t.is_empty() || t == "|" {
            return;
        }
        self.lines.push(Line {
            kind: Kind::Text,
            depth: self.text_depth,
            body: quote(&t),
            backend: self.text_backend.take(),
            ref_n: None,
            parent_ref: self.struct_stack.last().copied(),
            boiler: self.boiler_stack.last().copied().unwrap_or(0),
            focused: false,
            in_view: None,
            role: "text".into(),
            name: t,
            value: String::new(),
            node_id: String::new(),
            region: self.text_region.clone(),
            href: String::new(),
        });
    }

    fn add_text(&mut self, t: &str, backend: Option<i64>) {
        if self.text.is_empty() {
            self.text_depth = self.region_stack.len();
            self.text_region = self.region_stack.last().cloned().unwrap_or_default();
            self.text_backend = backend;
        }
        self.text.push_str(t);
    }

    fn collect_options(&self, id: &str, out: &mut Vec<String>) {
        self.collect_options_at(id, out, 0);
    }

    fn collect_options_at(&self, id: &str, out: &mut Vec<String>, depth: usize) {
        let Some(n) = self.ax.nodes.get(id) else {
            return;
        };
        if depth > 50 {
            return;
        }
        let role = val_str(&n["role"]);
        if matches!(role.as_str(), "option" | "ListBoxOption" | "MenuListOption") {
            let nm = squeeze(&val_str(&n["name"]), 40);
            let sel = prop_bool(n, "selected");
            out.push(if sel { format!("{nm}*") } else { nm });
            return;
        }
        if out.len() > 60 {
            return;
        }
        if let Some(ch) = n.get("childIds").and_then(Value::as_array) {
            for c in ch {
                if let Some(cid) = c.as_str() {
                    self.collect_options_at(cid, out, depth + 1);
                }
            }
        }
    }

    fn descendant_text(&self, id: &str, out: &mut String, budget: usize) {
        self.descendant_text_at(id, out, budget, 0);
    }

    fn descendant_text_at(&self, id: &str, out: &mut String, budget: usize, depth: usize) {
        if out.len() > budget || depth > 200 {
            return;
        }
        let Some(n) = self.ax.nodes.get(id) else {
            return;
        };
        let role = val_str(&n["role"]);
        if role == "StaticText" || role == "text" || role == "InlineTextBox" {
            let t = val_str(&n["name"]);
            if !out.is_empty() && !out.ends_with(' ') {
                out.push(' ');
            }
            out.push_str(&t);
            return;
        }
        if let Some(ch) = n.get("childIds").and_then(Value::as_array) {
            for c in ch {
                if let Some(cid) = c.as_str() {
                    self.descendant_text_at(cid, out, budget, depth + 1);
                }
            }
        }
    }

    fn walk(&mut self, id: &str, depth: usize) {
        let Some((&key, &n)) = self.ax.nodes.get_key_value(id) else {
            return;
        };
        if depth > MAX_WALK_DEPTH || !self.seen.insert(key) {
            return;
        }
        let ignored = n["ignored"] == true;
        let role_raw = val_str(&n["role"]);
        let children: Vec<&str> = n
            .get("childIds")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        if ignored {
            for c in children {
                self.walk(c, depth);
            }
            return;
        }
        if depth > self.depth_limit {
            return;
        }
        let name = squeeze(&val_str(&n["name"]), MAX_NAME);
        match role_raw.as_str() {
            "RootWebArea" | "WebArea" => {
                for c in children {
                    self.walk(c, depth + 1);
                }
                self.flush();
                return;
            }
            "StaticText" | "text" | "InlineTextBox" => {
                let t = val_str(&n["name"]);
                self.add_text(&t, n.get("backendDOMNodeId").and_then(Value::as_i64));
                return;
            }
            "LineBreak" => {
                self.flush();
                return;
            }
            "none"
            | "presentation"
            | "generic"
            | "paragraph"
            | "group"
            | "list"
            | "listitem"
            | "ListMarker"
            | "blockquote"
            | "row"
            | "rowgroup"
            | "cell"
            | "gridcell"
            | "columnheader"
            | "rowheader"
            | "Pre"
            | "DescriptionList"
            | "DescriptionListTerm"
            | "DescriptionListDetail"
            | "LayoutTable"
            | "LayoutTableRow"
            | "LayoutTableCell"
            | "Details"
            | "Abbr"
            | "time"
            | "mark"
            | "emphasis"
            | "strong"
            | "code"
            | "subscript"
            | "superscript" => {
                let cell = matches!(
                    role_raw.as_str(),
                    "cell" | "gridcell" | "columnheader" | "rowheader" | "LayoutTableCell"
                );
                let block = !matches!(
                    role_raw.as_str(),
                    "emphasis"
                        | "strong"
                        | "code"
                        | "subscript"
                        | "superscript"
                        | "Abbr"
                        | "time"
                        | "mark"
                        | "ListMarker"
                );
                if block && !cell {
                    self.flush();
                }
                if cell && !self.text.is_empty() {
                    self.text.push_str(" | ");
                }
                // a named generic group (aria-label) is context, but we keep it flat
                for c in children {
                    self.walk(c, depth + 1);
                }
                if block && !cell {
                    self.flush();
                }
                return;
            }
            _ => {}
        }
        let role = canon_role(&role_raw);
        let backend = n.get("backendDOMNodeId").and_then(Value::as_i64);
        let focused = prop_bool(n, "focused");
        let node_id = s(n, "nodeId").to_string();
        let mut state = String::new();
        let mut push = |w: &str| {
            state.push(' ');
            state.push_str(w);
        };
        if prop_bool(n, "disabled") {
            push("disabled");
        }
        match prop(n, "checked").map(val_str) {
            Some(c) if c == "true" => push("checked"),
            Some(c) if c == "mixed" => push("mixed"),
            // unchecked is the default: no word for it
            _ => {}
        }
        if prop_bool(n, "selected") {
            push("selected");
        }
        match prop(n, "expanded").map(val_str) {
            Some(c) if c == "true" => push("expanded"),
            Some(c) if c == "false" => push("collapsed"),
            _ => {}
        }
        if prop_bool(n, "required") {
            push("required");
        }
        if prop_bool(n, "readonly") {
            push("readonly");
        }
        if prop(n, "invalid")
            .map(val_str)
            .is_some_and(|v| v != "false")
        {
            push("invalid");
        }
        if let Some(p) = prop(n, "pressed").map(val_str)
            && p == "true"
        {
            push("pressed");
        }
        if prop_bool(n, "modal") {
            push("modal");
        }
        if focused {
            push("focused");
        }
        let href = prop(n, "url").map(val_str).unwrap_or_default();

        if LIVE.contains(&role_raw.as_str()) {
            self.flush();
            let mut t = name.clone();
            if t.is_empty() {
                let mut buf = String::new();
                self.descendant_text(id, &mut buf, 2000);
                t = squeeze(&buf, MAX_TEXT);
            }
            if !t.is_empty() {
                self.push_line(
                    Kind::Live,
                    &role,
                    &format!("{role} {}", quote(&t)),
                    backend,
                    focused,
                    &node_id,
                    &name,
                    "",
                    "",
                );
            }
            return;
        }
        if role == "heading" {
            self.flush();
            let level = prop(n, "level").map(val_str).unwrap_or_else(|| "2".into());
            let mut t = name.clone();
            if t.is_empty() {
                let mut buf = String::new();
                self.descendant_text(id, &mut buf, 1000);
                t = squeeze(&buf, MAX_NAME);
            }
            if !t.is_empty() {
                self.push_line(
                    Kind::Heading,
                    &format!("h{level}"),
                    &format!("h{level} {}", quote(&t)),
                    backend,
                    focused,
                    &node_id,
                    &t,
                    "",
                    "",
                );
            }
            return;
        }
        if role == "img" || role == "image" || role == "Image" {
            if !name.is_empty() && !self.opts.interactive_only {
                self.flush();
                self.push_line(
                    Kind::Image,
                    "img",
                    &format!("img {}", quote(&name)),
                    None,
                    false,
                    &node_id,
                    &name,
                    "",
                    "",
                );
            }
            return;
        }
        if INTERACTIVE.contains(&role_raw.as_str()) || INTERACTIVE.contains(&role.as_str()) {
            // an unnamed option/menu popup wrapper is only a container
            if matches!(role.as_str(), "listbox") || role_raw == "MenuListPopup" {
                self.flush();
                for c in children {
                    self.walk(c, depth + 1);
                }
                return;
            }
            self.flush();
            let mut body = role.to_string();
            let mut nm = name.clone();
            if nm.is_empty()
                && matches!(
                    role.as_str(),
                    "button" | "link" | "tab" | "menuitem" | "option" | "treeitem"
                )
            {
                let mut buf = String::new();
                self.descendant_text(id, &mut buf, 1000);
                nm = squeeze(&buf, MAX_NAME);
            }
            if !nm.is_empty() {
                body.push(' ');
                body.push_str(&quote(&nm));
            }
            let val = prop_value(n);
            // a select lists its options with the chosen one starred: the value would say it twice
            let mut opts: Vec<String> = vec![];
            if role == "combobox" {
                self.collect_options(id, &mut opts);
            }
            // an empty value is the default: no attribute for it
            if !val.is_empty()
                && opts.is_empty()
                && matches!(
                    role.as_str(),
                    "textbox" | "searchbox" | "combobox" | "spinbutton" | "slider" | "textarea"
                )
            {
                body.push_str(&format!(" value={}", quote(&squeeze(&val, 80))));
            }
            body.push_str(&state);
            if role == "combobox" {
                // a select: its options are a short inline list, not a node each
                if !opts.is_empty() {
                    let shown: Vec<String> = opts.iter().take(12).cloned().collect();
                    body.push_str(&format!(
                        " options=[{}{}]",
                        shown.join("|"),
                        if opts.len() > 12 {
                            format!("|…{}", opts.len() - 12)
                        } else {
                            String::new()
                        }
                    ));
                }
            }
            self.push_line(
                Kind::Interactive,
                &role,
                &body,
                backend,
                focused,
                &node_id,
                &nm,
                &val,
                &href,
            );
            let expanded_popup =
                role == "combobox" && prop(n, "expanded").map(val_str).as_deref() == Some("true");
            if expanded_popup
                || !(SKIP_DESCEND.contains(&role_raw.as_str())
                    || SKIP_DESCEND.contains(&role.as_str())
                    || role == "combobox")
            {
                for c in children {
                    self.walk(c, depth + 1);
                }
            }
            return;
        }
        if LANDMARK.contains(&role_raw.as_str()) || LANDMARK.contains(&role.as_str()) {
            // an unnamed generic "form"/"region" adds nothing
            if (role == "region" || role == "form") && name.is_empty() {
                for c in children {
                    self.walk(c, depth + 1);
                }
                return;
            }
            self.flush();
            let mut body = role.clone();
            if !name.is_empty() {
                body.push(' ');
                body.push_str(&quote(&name));
            }
            body.push_str(&state);
            let before = self.lines.len();
            self.boiler_id += 1;
            let bid = if BOILER.contains(&role.as_str()) {
                self.boiler_id
            } else {
                0
            };
            self.push_line(
                Kind::Structure,
                &role,
                &body,
                backend,
                focused,
                &node_id,
                &name,
                "",
                "",
            );
            if bid != 0
                && let Some(l) = self.lines.get_mut(before)
            {
                l.boiler = bid;
            }
            let my_idx = self.lines.len() - 1;
            self.region_stack.push(format!("{}", my_idx));
            if bid != 0 {
                self.boiler_stack.push(bid);
            }
            self.struct_stack.push(0);
            for c in children {
                self.walk(c, depth + 1);
            }
            self.flush();
            self.struct_stack.pop();
            if bid != 0 {
                self.boiler_stack.pop();
            }
            self.region_stack.pop();
            return;
        }
        // anything else (document, figure caption, ...) is a transparent container; a name gives context
        for c in children {
            self.walk(c, depth + 1);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn push_line(
        &mut self,
        kind: Kind,
        role: &str,
        body: &str,
        backend: Option<i64>,
        focused: bool,
        node_id: &str,
        name: &str,
        value: &str,
        href: &str,
    ) {
        let depth = self.region_stack.len();
        self.lines.push(Line {
            kind,
            depth,
            body: body.into(),
            backend,
            ref_n: None,
            parent_ref: None,
            boiler: self.boiler_stack.last().copied().unwrap_or(0),
            focused,
            in_view: None,
            role: role.into(),
            name: name.into(),
            value: value.into(),
            node_id: node_id.into(),
            region: self.region_stack.last().cloned().unwrap_or_default(),
            href: href.into(),
        });
        let _ = self.ax_depth;
        let _ = self.started;
    }
}

fn prop_value(n: &Value) -> String {
    val_str(&n["value"])
}

pub fn trim_href(h: &str) -> String {
    let h = h.trim();
    let h = h.strip_prefix("https://").unwrap_or(h);
    squeeze(h, 80)
}

/// The document title: the name of the accessibility tree's root.
pub fn root_title(nodes: &[Value]) -> String {
    nodes
        .iter()
        .find(|n| {
            matches!(val_str(&n["role"]).as_str(), "RootWebArea" | "WebArea")
                && n["parentId"].as_str().is_none_or(str::is_empty)
        })
        .map(|n| val_str(&n["name"]))
        .unwrap_or_default()
}

/// Build the line list of a tree (no refs yet). `from` limits the walk to the subtree of the AX
/// node whose backend id it names.
pub fn lines_of(nodes: &[Value], opts: &Options) -> Vec<Line> {
    let ax = index(nodes);
    let roots: Vec<&str> = match opts.region {
        Some(b) => nodes
            .iter()
            .find(|n| n.get("backendDOMNodeId").and_then(Value::as_i64) == Some(b))
            .map(|n| vec![s(n, "nodeId")])
            .unwrap_or_default(),
        None => ax.roots.clone(),
    };
    let mut b = Builder {
        ax,
        lines: vec![],
        opts,
        text: String::new(),
        text_depth: 0,
        text_region: String::new(),
        text_backend: None,
        struct_stack: vec![],
        region_stack: vec![],
        boiler_stack: vec![],
        boiler_id: 0,
        depth_limit: opts.depth.unwrap_or(usize::MAX / 2),
        ax_depth: 0,
        started: false,
        seen: HashSet::new(),
    };
    for r in roots {
        b.walk(r, 0);
    }
    b.flush();
    // keep the AX root's first node as the region itself when asked for a region
    b.lines
}

// ------------------------------------------------------------------------- rendering

/// Section hash of a boilerplate group: the bodies of its lines, refs excluded.
fn group_hash(lines: &[Line], start: usize, bid: u32) -> (u64, usize, usize) {
    let mut buf = String::new();
    let mut n = 0;
    let mut end = start;
    for (i, l) in lines.iter().enumerate().skip(start) {
        if i != start && l.boiler != bid {
            break;
        }
        buf.push_str(&l.body);
        buf.push('\n');
        n += 1;
        end = i;
    }
    (fnv(buf.as_bytes()), n, end)
}

/// Runs of this many same-shaped controls (same role, same depth, nothing between) are folded...
pub const FOLD_RUN: usize = 10;
/// ...to their first lines.
pub const FOLD_KEEP: usize = 6;

/// What a model would read twice: a link with the name and address of one already shown, a label
/// text that repeats the name of the control beside it, and long runs of same-shaped controls
/// (a language list, a navigation box), folded to their first lines with a note in place.
/// Returns how many lines were hidden; `notes` holds the note lines by the index they replace.
pub fn fold(lines: &[Line], hide: &mut [bool], notes: &mut HashMap<usize, String>) -> usize {
    let mut seen: HashSet<(&str, &str)> = HashSet::new();
    for (i, l) in lines.iter().enumerate() {
        if l.kind == Kind::Interactive
            && l.role == "link"
            && !l.name.is_empty()
            && !seen.insert((l.name.as_str(), l.href.as_str()))
        {
            hide[i] = true;
        }
    }
    let same_name = |j: usize, t: &str| {
        lines
            .get(j)
            .is_some_and(|c| c.kind == Kind::Interactive && c.name.eq_ignore_ascii_case(t))
    };
    for (i, l) in lines.iter().enumerate() {
        if l.kind == Kind::Text
            && !l.name.is_empty()
            && ((i > 0 && same_name(i - 1, &l.name)) || same_name(i + 1, &l.name))
        {
            hide[i] = true;
        }
    }
    let vis: Vec<usize> = (0..lines.len()).filter(|i| !hide[*i]).collect();
    let mut k = 0;
    while k < vis.len() {
        let a = &lines[vis[k]];
        let mut e = k;
        if a.kind == Kind::Interactive {
            while e + 1 < vis.len() {
                let b = &lines[vis[e + 1]];
                if b.kind != a.kind || b.role != a.role || b.depth != a.depth {
                    break;
                }
                e += 1;
            }
        }
        let run = e - k + 1;
        if run >= FOLD_RUN {
            for &i in &vis[k + FOLD_KEEP..=e] {
                hide[i] = true;
            }
            let pad = " ".repeat(a.depth.min(MAX_DEPTH));
            notes.insert(
                vis[k + FOLD_KEEP],
                format!(
                    "{pad}… {} more {} like these (--full)",
                    run - FOLD_KEEP,
                    a.role
                ),
            );
        }
        k = e + 1;
    }
    hide.iter().filter(|h| **h).count()
}

/// Assign refs, collapse repeated boilerplate, apply options; returns the rendered text.
/// `geometry` gives, per backend node id, whether it is in the viewport (None = not fetched).
pub fn render(
    mut lines: Vec<Line>,
    state: &mut AgentState,
    url: &str,
    title: &str,
    opts: &Options,
    geometry: &HashMap<i64, bool>,
    first_seen_gen: u64,
) -> Rendered {
    // boilerplate: collapse groups already seen on another page of this tab
    let mut collapsed = 0;
    let mut drop = vec![false; lines.len()];
    if !opts.full && opts.region.is_none() {
        let mut i = 0;
        while i < lines.len() {
            let bid = lines[i].boiler;
            if bid != 0 && lines[i].kind == Kind::Structure {
                let (h, count, end) = group_hash(&lines, i, bid);
                let seen_gen = state.boiler_gen.get(&h).copied();
                state.boiler_seen.insert(h);
                state.boiler_gen.entry(h).or_insert(first_seen_gen);
                if count > 2 && seen_gen.is_some_and(|g| g != first_seen_gen) {
                    for d in drop.iter_mut().take(end + 1).skip(i + 1) {
                        *d = true;
                    }
                    lines[i].body = format!(
                        "{} (same as the previous page, {} lines; --full to show)",
                        lines[i].body,
                        count - 1
                    );
                    collapsed += count - 1;
                    i = end + 1;
                    continue;
                }
            }
            i += 1;
        }
    }
    let mut kept: Vec<Line> = vec![];
    for (i, l) in lines.into_iter().enumerate() {
        if !drop[i] {
            kept.push(l);
        }
    }
    let mut lines = kept;
    // links: the page's own origin is noise
    let origin = url
        .find("://")
        .map(|i| {
            url[i + 3..]
                .find('/')
                .map(|j| i + 3 + j)
                .unwrap_or(url.len())
        })
        .map(|e| &url[..e])
        .unwrap_or("");
    for l in lines.iter_mut() {
        // the address of a named link is noise for a model (it clicks the ref); --hrefs shows them
        if l.role == "link" && !l.href.is_empty() && (opts.hrefs || l.name.is_empty()) {
            let h = l.href.trim();
            let shown = match h.strip_prefix(origin).filter(|_| !origin.is_empty()) {
                Some("") => "/".to_string(),
                Some(rest) => rest.to_string(),
                None => trim_href(h),
            };
            l.body = format!("{} -> {}", l.body, squeeze(&shown, 90));
        }
    }
    // refs
    for l in lines.iter_mut() {
        if matches!(
            l.kind,
            Kind::Interactive | Kind::Structure | Kind::Heading | Kind::Live
        ) && let Some(b) = l.backend
        {
            l.ref_n = Some(state.assign(b, &l.role, &l.name));
        }
        if let Some(b) = l.backend
            && let Some(v) = geometry.get(&b)
        {
            l.in_view = Some(*v);
        }
    }
    // text lines inherit the visibility of the previous line with geometry
    let mut last_view: Option<bool> = None;
    for l in lines.iter_mut() {
        if l.in_view.is_some() {
            last_view = l.in_view;
        } else if l.kind == Kind::Text || l.kind == Kind::Image {
            l.in_view = last_view;
        }
    }
    if opts.interactive_only {
        lines.retain(|l| {
            matches!(l.kind, Kind::Interactive | Kind::Live)
                || (l.kind == Kind::Structure
                    && l.ref_n.is_some()
                    && matches!(l.role.as_str(), "dialog" | "alertdialog"))
        });
    }
    if opts.viewport_only && !geometry.is_empty() {
        lines.retain(|l| l.in_view != Some(false));
    }
    let total = lines.len();
    // a data: or very long address is cut: the header orients, it does not carry the page
    let shown_url = if url.chars().count() > 200 {
        format!("{}…", url.chars().take(120).collect::<String>())
    } else {
        url.to_string()
    };
    let header = format!("@ {} {}", shown_url, quote(&squeeze(title, 100)));

    // diff
    let mut keyed: Vec<(String, String)> = Vec::with_capacity(lines.len());
    let mut occ: HashMap<u64, u32> = HashMap::new();
    for l in &lines {
        let h = fnv(l.body.as_bytes());
        let o = occ.entry(h).and_modify(|c| *c += 1).or_insert(0);
        keyed.push((l.key(*o), l.render()));
    }
    let mut selected: Vec<(usize, String)> = Vec::new();
    let mut diff_lines: Option<Vec<String>> = None;
    let mut no_baseline = false;
    let sig = format!(
        "{}|{:?}|{:?}|{}",
        opts.interactive_only, opts.depth, opts.region, opts.viewport_only
    );
    let baseline = !state.last.is_empty() && state.last_sig == sig;
    if opts.diff && !baseline {
        no_baseline = true;
    }
    if opts.diff && baseline {
        let old: HashMap<&str, &str> = state
            .last
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let new_keys: HashSet<&str> = keyed.iter().map(|(k, _)| k.as_str()).collect();
        let mut out = vec![];
        for (k, v) in &keyed {
            match old.get(k.as_str()) {
                None => out.push(format!("+ {v}")),
                Some(o) if o != v => out.push(format!("~ {v}")),
                _ => {}
            }
        }
        let mut removed_refs = vec![];
        let mut removed_other = 0;
        for (k, _) in &state.last {
            if !new_keys.contains(k.as_str()) {
                if k.starts_with('e') {
                    removed_refs.push(k.clone());
                } else {
                    removed_other += 1;
                }
            }
        }
        if !removed_refs.is_empty() {
            out.push(format!("- {}", removed_refs.join(" ")));
        }
        if removed_other > 0 {
            out.push(format!("- {removed_other} text lines"));
        }
        diff_lines = Some(out);
    }
    state.last = keyed.clone();
    state.last_sig = sig;

    let mut omitted = 0;
    let mut folded = 0;
    if let Some(d) = &diff_lines {
        // a diff is small by construction; the budget still applies in order
        for (i, t) in d.iter().enumerate() {
            selected.push((i, t.clone()));
        }
    } else {
        // the full view drops what a model would read twice; the diff baseline keeps every line
        let mut hide = vec![false; lines.len()];
        let mut notes: HashMap<usize, String> = HashMap::new();
        if !opts.full && opts.region.is_none() {
            folded = fold(&lines, &mut hide, &mut notes);
        }
        for (i, (_, text)) in keyed.iter().enumerate() {
            if !hide[i] {
                selected.push((i, text.clone()));
            } else if let Some(n) = notes.remove(&i) {
                selected.push((i, n));
            }
        }
    }
    let nodes_in = selected.len();
    if let Some(budget) = opts.max_tokens {
        let reserve = 12;
        let head = tokens(header.len() + 1);
        let mut left = budget.saturating_sub(head + reserve);
        // priority: 0 viewport or focus, 1 focus region, 2 the rest; ties by document order
        let focus_region = if diff_lines.is_none() {
            lines.iter().find(|l| l.focused).map(|l| l.region.clone())
        } else {
            None
        };
        let mut order: Vec<usize> = (0..selected.len()).collect();
        if diff_lines.is_none() {
            let class = |i: usize| -> u8 {
                let l = &lines[selected[i].0.min(lines.len().saturating_sub(1))];
                if l.focused || l.in_view == Some(true) {
                    0
                } else if focus_region
                    .as_ref()
                    .is_some_and(|r| !r.is_empty() && &l.region == r)
                {
                    1
                } else if geometry.is_empty() {
                    // no geometry: document order is the only signal
                    0
                } else {
                    2
                }
            };
            order.sort_by_key(|i| (class(*i), *i));
        }
        let mut keep = vec![false; selected.len()];
        for i in order {
            let cost = tokens(selected[i].1.len() + 1);
            if cost <= left {
                keep[i] = true;
                left -= cost;
            } else {
                // a line that does not fit stops lower-priority lines too, so the cut is a clean prefix per class
                continue;
            }
        }
        let mut kept = vec![];
        for (i, k) in keep.iter().enumerate() {
            if *k {
                kept.push(selected[i].clone());
            } else {
                omitted += 1;
            }
        }
        selected = kept;
    }
    let mut text = String::new();
    text.push_str(&header);
    text.push('\n');
    if no_baseline {
        text.push_str("(no earlier snapshot with these options: full view)\n");
    }
    if diff_lines.is_some() && selected.is_empty() {
        text.push_str("(no change)\n");
    }
    for (_, t) in &selected {
        text.push_str(t);
        text.push('\n');
    }
    if omitted > 0 {
        text.push_str(&format!("… {omitted} more nodes\n"));
    }
    let _ = (total, nodes_in);
    Rendered {
        tokens: tokens(text.len()),
        nodes: selected.len(),
        omitted,
        collapsed: collapsed + folded,
        text,
    }
}

// ------------------------------------------------------------------------- find

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Query {
    pub role: Option<String>,
    pub name_eq: Option<String>,
    pub name_has: Option<String>,
    pub value_has: Option<String>,
    pub any_has: Option<String>,
    pub css: Option<String>,
    pub testid: Option<String>,
    pub nth: Option<usize>,
}

/// Split a shell-ish string: spaces separate words, quotes group.
pub fn words(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut has = false;
    for ch in s.chars() {
        match (quote, ch) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '"') | (None, '\'') => {
                quote = Some(ch);
                has = true;
            }
            (None, c) if c.is_whitespace() => {
                if has || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            (None, c) => cur.push(c),
        }
    }
    if has || !cur.is_empty() {
        out.push(cur);
    }
    out
}

pub fn parse_query(q: &str) -> Query {
    let mut out = Query::default();
    let mut free = vec![];
    for w in words(q) {
        if let Some(v) = w.strip_prefix("role=") {
            out.role = Some(v.to_lowercase());
        } else if let Some(v) = w.strip_prefix("name~=") {
            out.name_has = Some(v.to_lowercase());
        } else if let Some(v) = w.strip_prefix("name=") {
            out.name_eq = Some(v.to_lowercase());
        } else if let Some(v) = w
            .strip_prefix("label~=")
            .or_else(|| w.strip_prefix("placeholder~="))
            .or_else(|| w.strip_prefix("label="))
        {
            out.name_has = Some(v.to_lowercase());
        } else if let Some(v) = w.strip_prefix("value~=") {
            out.value_has = Some(v.to_lowercase());
        } else if let Some(v) = w.strip_prefix("text~=").or_else(|| w.strip_prefix("text=")) {
            out.any_has = Some(v.to_lowercase());
        } else if let Some(v) = w.strip_prefix("css=") {
            out.css = Some(v.to_string());
        } else if let Some(v) = w.strip_prefix("testid=") {
            out.testid = Some(v.to_string());
        } else if let Some(v) = w.strip_prefix("nth=") {
            out.nth = v.parse().ok();
        } else {
            free.push(w);
        }
    }
    if !free.is_empty() {
        out.any_has = Some(free.join(" ").to_lowercase());
    }
    out
}

fn role_matches(want: &str, have: &str) -> bool {
    let have = have.to_lowercase();
    want == have
        || (want == "input"
            && matches!(
                have.as_str(),
                "textbox"
                    | "searchbox"
                    | "checkbox"
                    | "radio"
                    | "combobox"
                    | "spinbutton"
                    | "slider"
            ))
        || (want == "heading"
            && have.starts_with('h')
            && have.len() == 2
            && have.as_bytes()[1].is_ascii_digit())
}

/// Indexes into `lines` that match (only lines that can be referred to are returned: a matching
/// text run is reported through its nearest ref'd parent).
pub fn find(lines: &[Line], q: &Query) -> Vec<usize> {
    let mut out = vec![];
    for (i, l) in lines.iter().enumerate() {
        if let Some(r) = &q.role
            && !role_matches(r, &l.role)
        {
            continue;
        }
        let name = l.name.to_lowercase();
        if let Some(e) = &q.name_eq
            && &name != e
        {
            continue;
        }
        if let Some(h) = &q.name_has
            && !name.contains(h.as_str())
        {
            continue;
        }
        if let Some(h) = &q.value_has
            && !l.value.to_lowercase().contains(h.as_str())
        {
            continue;
        }
        if let Some(h) = &q.any_has
            && !name.contains(h.as_str())
            && !l.value.to_lowercase().contains(h.as_str())
            && !l.href.to_lowercase().contains(h.as_str())
        {
            continue;
        }
        if q.role.is_none()
            && q.name_eq.is_none()
            && q.name_has.is_none()
            && q.value_has.is_none()
            && q.any_has.is_none()
        {
            continue;
        }
        out.push(i);
    }
    if let Some(n) = q.nth {
        return out.get(n).copied().into_iter().collect();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ax(
        id: &str,
        role: &str,
        name: &str,
        parent: &str,
        children: &[&str],
        backend: i64,
        props: Value,
    ) -> Value {
        json!({"nodeId": id, "ignored": false, "role": {"type":"role","value":role},
               "name": {"type":"computedString","value":name},
               "parentId": parent, "childIds": children, "backendDOMNodeId": backend, "properties": props})
    }
    fn p(name: &str, v: Value) -> Value {
        json!({"name": name, "value": {"type":"boolean","value": v}})
    }

    fn page() -> Vec<Value> {
        vec![
            ax(
                "1",
                "RootWebArea",
                "Demo",
                "",
                &["2", "3", "4"],
                1,
                json!([]),
            ),
            ax("2", "navigation", "Main", "1", &["5", "6"], 2, json!([])),
            ax(
                "5",
                "link",
                "Home",
                "2",
                &[],
                5,
                json!([{"name":"url","value":{"type":"string","value":"https://a.test/"}}]),
            ),
            ax("6", "link", "Docs", "2", &[], 6, json!([])),
            ax(
                "3",
                "main",
                "",
                "1",
                &["7", "8", "9", "10", "11", "13"],
                3,
                json!([]),
            ),
            ax(
                "7",
                "heading",
                "Profile",
                "3",
                &[],
                7,
                json!([{"name":"level","value":{"type":"integer","value":1}}]),
            ),
            ax(
                "8",
                "textbox",
                "Email",
                "3",
                &[],
                8,
                json!([p("required", json!(true))]),
            ),
            ax(
                "9",
                "button",
                "Save",
                "3",
                &[],
                9,
                json!([p("disabled", json!(true))]),
            ),
            ax("10", "StaticText", "Hello ", "3", &[], 10, json!([])),
            ax("11", "StaticText", "world", "3", &[], 11, json!([])),
            ax("13", "status", "", "3", &["14"], 13, json!([])),
            ax("14", "StaticText", "Saved!", "13", &[], 14, json!([])),
            ax("4", "contentinfo", "", "1", &["12"], 4, json!([])),
            ax("12", "link", "Privacy", "4", &[], 12, json!([])),
        ]
    }

    #[test]
    fn renders_one_line_per_node_with_stable_refs() {
        let o = Options::default();
        let mut st = AgentState::default();
        let r = render(
            lines_of(&page(), &o),
            &mut st,
            "https://a.test/",
            "Demo",
            &o,
            &HashMap::new(),
            1,
        );
        let want = "@ https://a.test/ \"Demo\"\n\
[e1] navigation \"Main\"\n [e2] link \"Home\"\n [e3] link \"Docs\"\n\
[e4] main\n [e5] h1 \"Profile\"\n [e6] textbox \"Email\" required\n [e7] button \"Save\" disabled\n \"Hello world\"\n [e8] status \"Saved!\"\n\
[e9] contentinfo\n [e10] link \"Privacy\"\n";
        assert_eq!(r.text, want);
        // the same page again gives the same refs and the same text (deterministic)
        let r2 = render(
            lines_of(&page(), &o),
            &mut st,
            "https://a.test/",
            "Demo",
            &o,
            &HashMap::new(),
            1,
        );
        assert_eq!(r.text, r2.text);
        assert_eq!(r.tokens, tokens(r.text.len()));
    }

    #[test]
    fn interactive_only_keeps_what_can_be_acted_on() {
        let o = Options {
            interactive_only: true,
            ..Default::default()
        };
        let mut st = AgentState::default();
        let r = render(
            lines_of(&page(), &o),
            &mut st,
            "u",
            "t",
            &o,
            &HashMap::new(),
            1,
        );
        assert!(r.text.contains("textbox \"Email\""));
        assert!(!r.text.contains("Hello"));
        assert!(!r.text.contains("h1"));
        assert!(
            r.text.contains("status \"Saved!\""),
            "a toast is always shown"
        );
    }

    #[test]
    fn truncation_is_deterministic_and_says_how_many_were_dropped() {
        let o = Options {
            max_tokens: Some(40),
            ..Default::default()
        };
        let mut st = AgentState::default();
        let a = render(
            lines_of(&page(), &o),
            &mut st,
            "https://a.test/",
            "Demo",
            &o,
            &HashMap::new(),
            1,
        );
        let mut st2 = AgentState::default();
        let b = render(
            lines_of(&page(), &o),
            &mut st2,
            "https://a.test/",
            "Demo",
            &o,
            &HashMap::new(),
            1,
        );
        assert_eq!(a.text, b.text);
        assert!(a.omitted > 0 && a.text.contains(&format!("… {} more nodes", a.omitted)));
        assert!(a.tokens <= 40 + 4, "{} tokens", a.tokens);
    }

    #[test]
    fn truncation_keeps_the_viewport_first() {
        // the Save button is the only node in view; Email and the links are below
        let mut geo = HashMap::new();
        for b in [2, 5, 6, 3, 7, 8, 12, 4] {
            geo.insert(b, false);
        }
        geo.insert(9, true);
        let o = Options {
            max_tokens: Some(30),
            ..Default::default()
        };
        let mut st = AgentState::default();
        let r = render(lines_of(&page(), &o), &mut st, "u", "t", &o, &geo, 1);
        assert!(r.text.contains("button \"Save\""), "{}", r.text);
        assert!(!r.text.contains("Privacy"));
    }

    #[test]
    fn diff_shows_only_what_changed_with_the_same_refs() {
        let o = Options::default();
        let mut st = AgentState::default();
        render(
            lines_of(&page(), &o),
            &mut st,
            "u",
            "t",
            &o,
            &HashMap::new(),
            1,
        );
        let mut changed = page();
        // enable the button, drop the Docs link, add a new button
        changed[7] = ax("9", "button", "Save", "3", &[], 9, json!([]));
        changed.retain(|n| n["nodeId"] != "6");
        changed.push(ax("20", "button", "Undo", "3", &[], 20, json!([])));
        let main = changed.iter_mut().find(|n| n["nodeId"] == "3").unwrap();
        main["childIds"] = json!(["7", "8", "9", "10", "11", "13", "20"]);
        let d = Options {
            diff: true,
            ..Default::default()
        };
        let r = render(
            lines_of(&changed, &d),
            &mut st,
            "u",
            "t",
            &d,
            &HashMap::new(),
            1,
        );
        assert!(r.text.contains("~  [e7] button \"Save\""), "{}", r.text);
        assert!(r.text.contains("+  [e11] button \"Undo\""), "{}", r.text);
        assert!(r.text.contains("- e3"), "{}", r.text);
        assert!(!r.text.contains("Email"));
        // no change at all
        let r2 = render(
            lines_of(&changed, &d),
            &mut st,
            "u",
            "t",
            &d,
            &HashMap::new(),
            1,
        );
        assert!(r2.text.contains("(no change)"));
    }

    #[test]
    fn refs_reset_with_the_document() {
        let mut st = AgentState::default();
        st.sync_gen(1);
        assert_eq!(st.assign(77, "button", "x"), 1);
        assert_eq!(st.assign(77, "button", "x"), 1);
        assert_eq!(st.assign(78, "button", "y"), 2);
        st.sync_gen(2);
        assert_eq!(st.assign(78, "button", "y"), 1);
    }

    #[test]
    fn repeated_navigation_collapses_on_the_second_page() {
        let o = Options::default();
        let mut st = AgentState::default();
        let a = render(
            lines_of(&page(), &o),
            &mut st,
            "u1",
            "t",
            &o,
            &HashMap::new(),
            1,
        );
        assert!(a.text.contains("link \"Docs\""));
        // second page (new generation) has the same navigation, plus 3 lines in it to pass the size gate
        let mut nodes = page();
        nodes[1]["childIds"] = json!(["5", "6", "30"]);
        nodes.push(ax("30", "link", "Blog", "2", &[], 30, json!([])));
        let mut st = AgentState::default();
        st.sync_gen(1);
        let _ = render(
            lines_of(&nodes, &o),
            &mut st,
            "u1",
            "t",
            &o,
            &HashMap::new(),
            1,
        );
        st.sync_gen(2);
        let b = render(
            lines_of(&nodes, &o),
            &mut st,
            "u2",
            "t",
            &o,
            &HashMap::new(),
            2,
        );
        assert!(b.text.contains("same as the previous page"), "{}", b.text);
        assert!(!b.text.contains("Blog"));
        let full = Options {
            full: true,
            ..Default::default()
        };
        let c = render(
            lines_of(&nodes, &full),
            &mut st,
            "u2",
            "t",
            &full,
            &HashMap::new(),
            2,
        );
        assert!(c.text.contains("Blog"));
    }

    #[test]
    fn find_by_role_name_and_text() {
        let o = Options::default();
        let mut st = AgentState::default();
        let lines = lines_of(&page(), &o);
        let _ = render(lines.clone(), &mut st, "u", "t", &o, &HashMap::new(), 1);
        let q = parse_query("role=button name~=save");
        let hits = find(&lines, &q);
        assert_eq!(hits.len(), 1);
        assert_eq!(lines[hits[0]].name, "Save");
        assert_eq!(find(&lines, &parse_query("\"docs\"")).len(), 1);
        assert_eq!(find(&lines, &parse_query("role=heading")).len(), 1);
        assert_eq!(find(&lines, &parse_query("role=link nth=1")).len(), 1);
        assert_eq!(
            parse_query("role=link name=\"A B\"").name_eq.as_deref(),
            Some("a b")
        );
    }

    #[test]
    fn a_cyclic_or_very_deep_tree_does_not_hang_or_overflow() {
        // the browser never sends a cycle, but a decoder of outside input must survive one
        let cyc = vec![
            ax("1", "RootWebArea", "t", "", &["2"], 1, json!([])),
            ax("2", "group", "", "1", &["1", "2"], 2, json!([])),
        ];
        let o = Options::default();
        let mut st = AgentState::default();
        let r = render(
            lines_of(&cyc, &o),
            &mut st,
            "u",
            "t",
            &o,
            &HashMap::new(),
            1,
        );
        assert_eq!(r.nodes, 0);
        // a chain far deeper than any page: cut at MAX_WALK_DEPTH, no stack overflow
        let mut deep = vec![ax("n0", "RootWebArea", "t", "", &["n1"], 1, json!([]))];
        for i in 1..5000 {
            let kid = format!("n{}", i + 1);
            let kids: Vec<&str> = if i < 4999 { vec![kid.as_str()] } else { vec![] };
            deep.push(ax(
                &format!("n{i}"),
                "group",
                "",
                &format!("n{}", i - 1),
                &kids,
                i + 1,
                json!([]),
            ));
        }
        deep.push(ax("leaf", "button", "Deep", "n4999", &[], 99999, json!([])));
        // the verbs run on 8 MiB stacks (server.rs); so does this check
        let text = std::thread::Builder::new()
            .stack_size(8 << 20)
            .spawn(move || {
                let o = Options::default();
                let mut st = AgentState::default();
                render(
                    lines_of(&deep, &o),
                    &mut st,
                    "u",
                    "t",
                    &o,
                    &HashMap::new(),
                    2,
                )
                .text
            })
            .unwrap()
            .join()
            .unwrap();
        assert!(
            !text.contains("Deep"),
            "beyond the depth limit nothing is walked"
        );
    }

    #[test]
    fn a_select_lists_its_options_inline() {
        let nodes = vec![
            ax("1", "RootWebArea", "t", "", &["2"], 1, json!([])),
            ax(
                "2",
                "combobox",
                "Plan",
                "1",
                &["3", "4"],
                2,
                json!([{"name":"expanded","value":{"type":"boolean","value":false}}]),
            ),
            ax(
                "3",
                "option",
                "Free",
                "2",
                &[],
                3,
                json!([p("selected", json!(true))]),
            ),
            ax("4", "option", "Pro", "2", &[], 4, json!([])),
        ];
        let o = Options::default();
        let mut st = AgentState::default();
        let r = render(
            lines_of(&nodes, &o),
            &mut st,
            "u",
            "t",
            &o,
            &HashMap::new(),
            1,
        );
        assert!(
            r.text.contains("combobox \"Plan\"") && r.text.contains("options=[Free*|Pro]"),
            "{}",
            r.text
        );
        assert!(
            !r.text.contains("[e2] option"),
            "options are not nodes of their own: {}",
            r.text
        );
    }

    #[test]
    fn words_respect_quotes() {
        assert_eq!(
            words("role=button name~=\"Save now\" x"),
            vec!["role=button", "name~=Save now", "x"]
        );
    }

    #[test]
    fn region_renders_one_subtree() {
        let o = Options {
            region: Some(2),
            ..Default::default()
        };
        let mut st = AgentState::default();
        let r = render(
            lines_of(&page(), &o),
            &mut st,
            "u",
            "t",
            &o,
            &HashMap::new(),
            1,
        );
        assert!(
            r.text.contains("Docs") && !r.text.contains("Email"),
            "{}",
            r.text
        );
    }
}

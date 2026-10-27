//! Widget package schema: `<packages dir>/<id>/widget.json`. A layout is a tree of Kit primitives
//! with `{{path}}` bindings into the JSON the `source` returns. Validation collects every problem
//! with its JSON path, so an author fixes them all in one pass. Keys starting with `_` are comments.
use super::Size;
use super::template::{Scope, Template};
use serde_json::{Map, Value};

pub const DEFAULT_ICON: &str = "application-x-addon-symbolic";
/// Seconds. 0 = load once; else at least `MIN_REFRESH` (calm, battery).
pub const DEFAULT_REFRESH: u32 = 900;
pub const MIN_REFRESH: u32 = 60;
pub const MAX_REFRESH: u32 = 86_400;
/// Command source timeout bounds (seconds).
pub const DEFAULT_TIMEOUT: u32 = 5;
pub const MAX_TIMEOUT: u32 = 30;
/// Spacing scale (HIG §2) a layout may use.
pub const SPACING: [i32; 6] = [0, 4, 8, 12, 16, 24];
pub const MAX_DEPTH: usize = 12;
pub const MAX_NODES: usize = 200;
pub const MAX_LIST: u8 = 20;
const NAME_MAX: usize = 40;
const DESCRIPTION_MAX: usize = 120;

/// One validation problem: where (JSON path) and what to change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub path: String,
    pub message: String,
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub description: String,
    pub icon: String,
    pub sizes: Vec<Size>,
    pub refresh: u32,
    pub source: Option<Source>,
    /// One layout per size in `sizes`.
    pub layouts: Vec<(Size, Node)>,
    /// Data for gallery previews and `wol widgets validate`; the gallery never runs a source.
    pub sample: Option<Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// argv run as the user in the package dir; `./x` is relative to the package.
    Command { argv: Vec<String>, timeout: u32 },
    /// JSON file, relative to the package or `~/...`.
    File { path: String },
    /// GET; needs a grant for the host (`Desktop::allows`).
    Http { url: String },
}

impl Source {
    /// Host an http source contacts (consent key).
    pub fn host(&self) -> Option<String> {
        match self {
            Source::Http { url } => url_host(url),
            _ => None,
        }
    }
}

/// `https://user@Api.Example.com:443/x?y` -> `api.example.com`.
pub fn url_host(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let auth = rest.split(['/', '?', '#']).next()?;
    let host = auth.rsplit('@').next()?;
    let host = host.split(':').next()?.to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Start,
    Center,
    End,
    Fill,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Widget figure (temperature, time).
    Display,
    Large,
    Title,
    Body,
    Secondary,
    /// Small uppercase heading.
    Caption,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Text,
    Dim,
    Accent,
    Success,
    Warning,
    Danger,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconSize {
    Small,
    Medium,
    Large,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChartKind {
    Line,
    Bar,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// argv; the program (argv[0]) is literal, arguments may bind data.
    Run(Vec<Template>),
    /// URL opened with the default handler.
    Open(Template),
    /// Desktop entry id, e.g. `org.gnome.Calendar`.
    App(String),
}

/// URL schemes `open` accepts from data; others only when the manifest spells the scheme out.
pub const OPEN_SCHEMES: [&str; 3] = ["https", "http", "mailto"];
/// Never opened, even when written out (scripts, local files).
pub const BLOCKED_SCHEMES: [&str; 4] = ["javascript", "data", "file", "vbscript"];

/// Whether `url` (rendered from `template`) may be opened.
pub fn open_allowed(url: &str, template_prefix: &str) -> bool {
    let Some((scheme, _)) = url.split_once(':') else {
        return false;
    };
    let scheme = scheme.to_ascii_lowercase();
    !scheme.is_empty()
        && scheme
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'-' || b == b'.')
        && !BLOCKED_SCHEMES.contains(&scheme.as_str())
        && (OPEN_SCHEMES.contains(&scheme.as_str())
            || template_prefix
                .to_ascii_lowercase()
                .starts_with(&format!("{scheme}:")))
}

impl Action {
    /// Literal text before the first binding of an `open` URL (scheme check).
    pub fn open_prefix(src: &str) -> &str {
        src.split("{{").next().unwrap_or("")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Stack {
        children: Vec<Node>,
        spacing: i32,
        align: Align,
    },
    Row {
        children: Vec<Node>,
        spacing: i32,
        align: Align,
    },
    Card {
        children: Vec<Node>,
        spacing: i32,
    },
    Label {
        text: Template,
        role: Role,
        tone: Tone,
        lines: u8,
        align: Align,
    },
    Icon {
        name: Template,
        size: IconSize,
        tone: Tone,
    },
    Image {
        src: Template,
        size: IconSize,
    },
    List {
        items: Template,
        item: Box<Node>,
        max: u8,
        empty: String,
        spacing: i32,
    },
    Button {
        label: Option<Template>,
        icon: Option<Template>,
        action: Action,
        /// Literal `open` text (scheme check at click time).
        open_src: String,
    },
    Gauge {
        value: Template,
        max: f64,
        label: Option<Template>,
        icon: Option<Template>,
        tone: Tone,
        size: IconSize,
    },
    Chart {
        kind: ChartKind,
        values: Template,
        max: Option<f64>,
        tone: Tone,
        size: IconSize,
    },
    Progress {
        value: Template,
        max: f64,
        tone: Tone,
    },
    Badge {
        text: Template,
        tone: Tone,
    },
    Spacer,
}

impl Node {
    /// Templates in this subtree, with the scope they bind in (`true` = inside a list item).
    pub fn templates(&self, in_item: bool, out: &mut Vec<(Template, bool)>) {
        let mut t = |x: &Template| out.push((x.clone(), in_item));
        match self {
            Node::Stack { children, .. }
            | Node::Row { children, .. }
            | Node::Card { children, .. } => {
                for c in children {
                    c.templates(in_item, out);
                }
            }
            Node::Label { text, .. } | Node::Badge { text, .. } => t(text),
            Node::Icon { name, .. } => t(name),
            Node::Image { src, .. } => t(src),
            Node::List { items, item, .. } => {
                t(items);
                item.templates(true, out);
            }
            Node::Button {
                label,
                icon,
                action,
                ..
            } => {
                label.iter().chain(icon.iter()).for_each(&mut t);
                match action {
                    Action::Run(argv) => argv.iter().for_each(t),
                    Action::Open(u) => t(u),
                    Action::App(_) => {}
                }
            }
            Node::Gauge {
                value, label, icon, ..
            } => {
                t(value);
                label.iter().chain(icon.iter()).for_each(t);
            }
            Node::Chart { values, .. } => t(values),
            Node::Progress { value, .. } => t(value),
            Node::Spacer => {}
        }
    }
}

struct Ctx {
    problems: Vec<Problem>,
    nodes: usize,
}

impl Ctx {
    fn err(&mut self, path: &str, message: impl Into<String>) {
        self.problems.push(Problem {
            path: path.into(),
            message: message.into(),
        });
    }
}

fn kind_of(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "true/false",
        Value::Number(_) => "a number",
        Value::String(_) => "text",
        Value::Array(_) => "a list",
        Value::Object(_) => "an object",
    }
}

/// Rejects keys a node or section does not take (typos), naming the ones it does.
fn known_keys(obj: &Map<String, Value>, path: &str, allowed: &[&str], what: &str, ctx: &mut Ctx) {
    for k in obj.keys() {
        if !k.starts_with('_') && !allowed.contains(&k.as_str()) {
            ctx.err(
                &format!("{path}.{k}"),
                format!("unknown key; {what} takes {}", allowed.join(", ")),
            );
        }
    }
}

fn text(obj: &Map<String, Value>, key: &str, path: &str, ctx: &mut Ctx) -> Option<String> {
    match obj.get(key)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        v => {
            ctx.err(
                &format!("{path}.{key}"),
                format!("expected text, found {}", kind_of(v)),
            );
            None
        }
    }
}

fn template(obj: &Map<String, Value>, key: &str, path: &str, ctx: &mut Ctx) -> Option<Template> {
    let s = text(obj, key, path, ctx)?;
    match Template::parse(&s) {
        Ok(t) => Some(t),
        Err(e) => {
            ctx.err(&format!("{path}.{key}"), e);
            None
        }
    }
}

fn required_template(
    obj: &Map<String, Value>,
    key: &str,
    path: &str,
    ctx: &mut Ctx,
) -> Option<Template> {
    if !obj.contains_key(key) {
        ctx.err(path, format!("missing \"{key}\""));
        return None;
    }
    template(obj, key, path, ctx)
}

/// `"{{path}}"`, exactly one binding (lists, chart values).
fn binding(obj: &Map<String, Value>, key: &str, path: &str, ctx: &mut Ctx) -> Option<Template> {
    let t = required_template(obj, key, path, ctx)?;
    if t.is_single_binding() {
        Some(t)
    } else {
        ctx.err(
            &format!("{path}.{key}"),
            "expected one binding like \"{{friends}}\"",
        );
        None
    }
}

fn choice<T: Copy>(
    obj: &Map<String, Value>,
    key: &str,
    path: &str,
    table: &[(&str, T)],
    default: T,
    ctx: &mut Ctx,
) -> T {
    let Some(v) = obj.get(key) else {
        return default;
    };
    let names: Vec<&str> = table.iter().map(|(n, _)| *n).collect();
    match v.as_str().and_then(|s| table.iter().find(|(n, _)| *n == s)) {
        Some((_, t)) => *t,
        None => {
            ctx.err(
                &format!("{path}.{key}"),
                format!("{v} is not one of {}", names.join(", ")),
            );
            default
        }
    }
}

fn number(obj: &Map<String, Value>, key: &str, path: &str, ctx: &mut Ctx) -> Option<f64> {
    let v = obj.get(key)?;
    match v.as_f64() {
        Some(n) => Some(n),
        None => {
            ctx.err(
                &format!("{path}.{key}"),
                format!("expected a number, found {}", kind_of(v)),
            );
            None
        }
    }
}

fn spacing(obj: &Map<String, Value>, path: &str, default: i32, ctx: &mut Ctx) -> i32 {
    match number(obj, "spacing", path, ctx) {
        None => default,
        Some(n) if SPACING.contains(&(n as i32)) && n.fract() == 0.0 => n as i32,
        Some(n) => {
            ctx.err(
                &format!("{path}.spacing"),
                format!("{n} is not on the spacing scale {SPACING:?}"),
            );
            default
        }
    }
}

const ALIGNS: [(&str, Align); 4] = [
    ("start", Align::Start),
    ("center", Align::Center),
    ("end", Align::End),
    ("fill", Align::Fill),
];
const ROLES: [(&str, Role); 6] = [
    ("display", Role::Display),
    ("large", Role::Large),
    ("title", Role::Title),
    ("body", Role::Body),
    ("secondary", Role::Secondary),
    ("caption", Role::Caption),
];
const TONES: [(&str, Tone); 6] = [
    ("text", Tone::Text),
    ("dim", Tone::Dim),
    ("accent", Tone::Accent),
    ("success", Tone::Success),
    ("warning", Tone::Warning),
    ("danger", Tone::Danger),
];
const ICON_SIZES: [(&str, IconSize); 3] = [
    ("small", IconSize::Small),
    ("medium", IconSize::Medium),
    ("large", IconSize::Large),
];
const CHARTS: [(&str, ChartKind); 2] = [("line", ChartKind::Line), ("bar", ChartKind::Bar)];
pub const NODE_TYPES: [&str; 13] = [
    "stack", "row", "card", "label", "icon", "image", "list", "button", "gauge", "chart",
    "progress", "badge", "spacer",
];

fn children(obj: &Map<String, Value>, path: &str, depth: usize, ctx: &mut Ctx) -> Vec<Node> {
    match obj.get("children") {
        None => {
            ctx.err(path, "missing \"children\"");
            Vec::new()
        }
        Some(Value::Array(a)) => a
            .iter()
            .enumerate()
            .filter_map(|(i, c)| node(c, &format!("{path}.children[{i}]"), depth + 1, ctx))
            .collect(),
        Some(v) => {
            ctx.err(
                &format!("{path}.children"),
                format!("expected a list of nodes, found {}", kind_of(v)),
            );
            Vec::new()
        }
    }
}

fn positive_max(obj: &Map<String, Value>, path: &str, default: f64, ctx: &mut Ctx) -> f64 {
    match number(obj, "max", path, ctx) {
        Some(m) if m > 0.0 => m,
        Some(_) => {
            ctx.err(&format!("{path}.max"), "must be greater than 0");
            default
        }
        None => default,
    }
}

fn node(v: &Value, path: &str, depth: usize, ctx: &mut Ctx) -> Option<Node> {
    ctx.nodes += 1;
    if ctx.nodes == MAX_NODES + 1 {
        ctx.err(path, format!("more than {MAX_NODES} nodes"));
    }
    if depth > MAX_DEPTH {
        ctx.err(path, format!("nested deeper than {MAX_DEPTH} levels"));
        return None;
    }
    let Value::Object(o) = v else {
        ctx.err(
            path,
            format!("expected a node object, found {}", kind_of(v)),
        );
        return None;
    };
    let Some(ty) = o.get("type").and_then(Value::as_str) else {
        ctx.err(
            path,
            format!("missing \"type\" (one of {})", NODE_TYPES.join(", ")),
        );
        return None;
    };
    let keys = |ctx: &mut Ctx, allowed: &[&str]| {
        let mut all = vec!["type"];
        all.extend_from_slice(allowed);
        known_keys(o, path, &all, ty, ctx)
    };
    let tone = |ctx: &mut Ctx, d| choice(o, "tone", path, &TONES, d, ctx);
    Some(match ty {
        "stack" | "row" => {
            keys(ctx, &["children", "spacing", "align"]);
            let (spacing, default_align) = if ty == "stack" {
                (spacing(o, path, 4, ctx), Align::Fill)
            } else {
                (spacing(o, path, 8, ctx), Align::Center)
            };
            let align = choice(o, "align", path, &ALIGNS, default_align, ctx);
            let children = children(o, path, depth, ctx);
            if ty == "stack" {
                Node::Stack {
                    children,
                    spacing,
                    align,
                }
            } else {
                Node::Row {
                    children,
                    spacing,
                    align,
                }
            }
        }
        "card" => {
            keys(ctx, &["children", "spacing"]);
            Node::Card {
                spacing: spacing(o, path, 4, ctx),
                children: children(o, path, depth, ctx),
            }
        }
        "label" => {
            keys(ctx, &["text", "role", "tone", "lines", "align"]);
            let lines = match number(o, "lines", path, ctx) {
                None => 1,
                Some(n) if (1.0..=4.0).contains(&n) && n.fract() == 0.0 => n as u8,
                Some(_) => {
                    ctx.err(&format!("{path}.lines"), "use 1 to 4");
                    1
                }
            };
            Node::Label {
                text: required_template(o, "text", path, ctx)?,
                role: choice(o, "role", path, &ROLES, Role::Body, ctx),
                tone: tone(ctx, Tone::Text),
                lines,
                align: choice(o, "align", path, &ALIGNS, Align::Start, ctx),
            }
        }
        "icon" => {
            keys(ctx, &["name", "size", "tone"]);
            Node::Icon {
                name: required_template(o, "name", path, ctx)?,
                size: choice(o, "size", path, &ICON_SIZES, IconSize::Small, ctx),
                tone: tone(ctx, Tone::Text),
            }
        }
        "image" => {
            keys(ctx, &["src", "size"]);
            let src = required_template(o, "src", path, ctx)?;
            let lit = src.render(Scope::root(&Value::Null));
            if lit.starts_with("http:") || lit.starts_with("https:") {
                ctx.err(
                    &format!("{path}.src"),
                    "images must be local files (no network fetches)",
                );
            }
            Node::Image {
                src,
                size: choice(o, "size", path, &ICON_SIZES, IconSize::Medium, ctx),
            }
        }
        "list" => {
            keys(ctx, &["items", "item", "max", "empty", "spacing"]);
            let items = binding(o, "items", path, ctx);
            let item = match o.get("item") {
                Some(i) => node(i, &format!("{path}.item"), depth + 1, ctx),
                None => {
                    ctx.err(path, "missing \"item\" (the node drawn for each element)");
                    None
                }
            };
            let max = match number(o, "max", path, ctx) {
                None => 5,
                Some(n) if n >= 1.0 && n <= MAX_LIST as f64 && n.fract() == 0.0 => n as u8,
                Some(_) => {
                    ctx.err(&format!("{path}.max"), format!("use 1 to {MAX_LIST}"));
                    5
                }
            };
            Node::List {
                items: items?,
                item: Box::new(item?),
                max,
                empty: text(o, "empty", path, ctx).unwrap_or_default(),
                spacing: spacing(o, path, 4, ctx),
            }
        }
        "button" => {
            keys(ctx, &["label", "icon", "run", "open", "app"]);
            let label = template(o, "label", path, ctx);
            let icon = template(o, "icon", path, ctx);
            if label.is_none() && icon.is_none() {
                ctx.err(path, "a button needs \"label\" or \"icon\"");
            }
            let given: Vec<&str> = ["run", "open", "app"]
                .into_iter()
                .filter(|k| o.contains_key(*k))
                .collect();
            if given.len() != 1 {
                ctx.err(
                    path,
                    "a button needs exactly one of \"run\", \"open\", \"app\"",
                );
                return None;
            }
            let mut open_src = String::new();
            let action = match given[0] {
                "run" => {
                    let argv = match o.get("run") {
                        Some(Value::Array(a)) if !a.is_empty() => a,
                        _ => {
                            ctx.err(
                                &format!("{path}.run"),
                                "expected a command line like [\"./join.sh\", \"{{item.id}}\"]",
                            );
                            return None;
                        }
                    };
                    let mut out = Vec::new();
                    for (i, a) in argv.iter().enumerate() {
                        let p = format!("{path}.run[{i}]");
                        let Some(s) = a.as_str() else {
                            ctx.err(&p, format!("expected text, found {}", kind_of(a)));
                            continue;
                        };
                        match Template::parse(s) {
                            Ok(t) if i == 0 && (!t.is_literal() || s.is_empty()) => {
                                ctx.err(&p, "the program must be written out, not taken from data")
                            }
                            Ok(t) => out.push(t),
                            Err(e) => ctx.err(&p, e),
                        }
                    }
                    Action::Run(out)
                }
                "open" => {
                    let t = required_template(o, "open", path, ctx)?;
                    open_src = text(o, "open", path, ctx).unwrap_or_default();
                    // A scheme taken from data only opens from the safe set (checked on click).
                    if t.is_literal() && !open_allowed(&open_src, &open_src) {
                        ctx.err(&format!("{path}.open"), "expected a URL like https://…");
                    }
                    Action::Open(t)
                }
                _ => match o.get("app").and_then(Value::as_str) {
                    Some(id) if !id.is_empty() && !id.contains('/') => Action::App(id.into()),
                    _ => {
                        ctx.err(
                            &format!("{path}.app"),
                            "expected a desktop entry id like \"org.gnome.Calendar\"",
                        );
                        return None;
                    }
                },
            };
            Node::Button {
                label,
                icon,
                action,
                open_src,
            }
        }
        "gauge" => {
            keys(ctx, &["value", "max", "label", "icon", "tone", "size"]);
            Node::Gauge {
                value: required_template(o, "value", path, ctx)?,
                max: positive_max(o, path, 100.0, ctx),
                label: template(o, "label", path, ctx),
                icon: template(o, "icon", path, ctx),
                tone: tone(ctx, Tone::Accent),
                size: choice(o, "size", path, &ICON_SIZES, IconSize::Medium, ctx),
            }
        }
        "chart" => {
            keys(ctx, &["kind", "values", "max", "tone", "size"]);
            let max = o
                .contains_key("max")
                .then(|| positive_max(o, path, 100.0, ctx));
            Node::Chart {
                kind: choice(o, "kind", path, &CHARTS, ChartKind::Line, ctx),
                values: binding(o, "values", path, ctx)?,
                max,
                tone: tone(ctx, Tone::Accent),
                size: choice(o, "size", path, &ICON_SIZES, IconSize::Medium, ctx),
            }
        }
        "progress" => {
            keys(ctx, &["value", "max", "tone"]);
            Node::Progress {
                value: required_template(o, "value", path, ctx)?,
                max: positive_max(o, path, 100.0, ctx),
                tone: tone(ctx, Tone::Accent),
            }
        }
        "badge" => {
            keys(ctx, &["text", "tone"]);
            Node::Badge {
                text: required_template(o, "text", path, ctx)?,
                tone: tone(ctx, Tone::Accent),
            }
        }
        "spacer" => {
            keys(ctx, &[]);
            Node::Spacer
        }
        other => {
            ctx.err(
                &format!("{path}.type"),
                format!(
                    "unknown node type {other:?}; use one of {}",
                    NODE_TYPES.join(", ")
                ),
            );
            return None;
        }
    })
}

fn source(v: &Value, ctx: &mut Ctx) -> Option<Source> {
    let path = "source";
    let Value::Object(o) = v else {
        ctx.err(
            path,
            "expected an object like {\"command\": [\"./data.sh\"]}",
        );
        return None;
    };
    known_keys(
        o,
        path,
        &["command", "timeout", "file", "http"],
        "source",
        ctx,
    );
    let given: Vec<&str> = ["command", "file", "http"]
        .into_iter()
        .filter(|k| o.contains_key(*k))
        .collect();
    if given.len() != 1 {
        ctx.err(path, "use exactly one of \"command\", \"file\", \"http\"");
        return None;
    }
    if given[0] != "command" && o.contains_key("timeout") {
        ctx.err("source.timeout", "only command sources take a timeout");
    }
    match given[0] {
        "command" => {
            let argv: Option<Vec<String>> = o["command"].as_array().and_then(|a| {
                a.iter()
                    .map(|x| x.as_str().map(String::from))
                    .collect::<Option<Vec<_>>>()
            });
            let argv = match argv {
                Some(a) if a.first().is_some_and(|p| !p.is_empty()) => a,
                _ => {
                    ctx.err(
                        "source.command",
                        "expected a command line like [\"./friends.sh\", \"--json\"]",
                    );
                    return None;
                }
            };
            if argv[0].split('/').any(|s| s == "..") {
                ctx.err("source.command[0]", "\"..\" may not leave the package");
            }
            let timeout = match number(o, "timeout", path, ctx) {
                None => DEFAULT_TIMEOUT,
                Some(t) if t >= 1.0 && t <= MAX_TIMEOUT as f64 => t as u32,
                Some(_) => {
                    ctx.err("source.timeout", format!("use 1 to {MAX_TIMEOUT} seconds"));
                    DEFAULT_TIMEOUT
                }
            };
            Some(Source::Command { argv, timeout })
        }
        "file" => {
            let p = o["file"].as_str().unwrap_or_default().to_string();
            if p.is_empty() || p.starts_with('/') || p.split('/').any(|s| s == "..") {
                ctx.err(
                    "source.file",
                    "use a path inside the package (\"data.json\") or under your home (\"~/…\")",
                );
                return None;
            }
            Some(Source::File { path: p })
        }
        _ => {
            let url = o["http"].as_str().unwrap_or_default().to_string();
            if !(url.starts_with("https://") || url.starts_with("http://"))
                || url_host(&url).is_none()
            {
                ctx.err(
                    "source.http",
                    "expected a URL like https://api.example.com/…",
                );
                return None;
            }
            Some(Source::Http { url })
        }
    }
}

impl Manifest {
    /// Parses and validates `widget.json` of package `id`; every problem is reported.
    pub fn parse(id: &str, json: &str) -> Result<Manifest, Vec<Problem>> {
        let one = |path: &str, message: String| {
            vec![Problem {
                path: path.into(),
                message,
            }]
        };
        if !super::is_package_id(id) {
            return Err(one(
                id,
                "package folder names use a-z, 0-9 and - only".into(),
            ));
        }
        let v: Value = serde_json::from_str(json).map_err(|e| {
            one(
                super::MANIFEST_FILE,
                format!("line {}, column {}: {e}", e.line(), e.column())
                    .replace(&format!(" at line {} column {}", e.line(), e.column()), ""),
            )
        })?;
        let Value::Object(o) = v else {
            return Err(one(super::MANIFEST_FILE, "expected a JSON object".into()));
        };
        let mut ctx = Ctx {
            problems: Vec::new(),
            nodes: 0,
        };
        let root = "widget";
        known_keys(
            &o,
            root,
            &[
                "name",
                "description",
                "icon",
                "sizes",
                "refresh",
                "source",
                "layout",
                "layouts",
                "sample",
            ],
            "widget.json",
            &mut ctx,
        );
        let name = text(&o, "name", root, &mut ctx).unwrap_or_default();
        if name.trim().is_empty() {
            ctx.err("name", "missing \"name\"");
        } else if name.chars().count() > NAME_MAX {
            ctx.err("name", format!("longer than {NAME_MAX} characters"));
        }
        let description = text(&o, "description", root, &mut ctx).unwrap_or_default();
        if description.chars().count() > DESCRIPTION_MAX {
            ctx.err(
                "description",
                format!("longer than {DESCRIPTION_MAX} characters"),
            );
        }
        let icon = text(&o, "icon", root, &mut ctx).unwrap_or_else(|| DEFAULT_ICON.into());
        if icon.is_empty() || icon.contains('/') {
            ctx.err(
                "icon",
                "expected an icon name like \"input-gaming-symbolic\"",
            );
        }
        let sizes = match o.get("sizes") {
            None => vec![Size::Small],
            Some(Value::Array(a)) if !a.is_empty() => {
                let mut out = Vec::new();
                for (i, s) in a.iter().enumerate() {
                    match s.as_str().and_then(Size::parse) {
                        Some(z) if !out.contains(&z) => out.push(z),
                        Some(_) => ctx.err(&format!("sizes[{i}]"), "listed twice"),
                        None => ctx.err(
                            &format!("sizes[{i}]"),
                            format!("{s} is not one of small, medium, large"),
                        ),
                    }
                }
                out.sort();
                out
            }
            Some(_) => {
                ctx.err("sizes", "expected a list like [\"small\", \"medium\"]");
                vec![Size::Small]
            }
        };
        let refresh = match number(&o, "refresh", root, &mut ctx) {
            None => DEFAULT_REFRESH,
            Some(r) if r == 0.0 || (MIN_REFRESH as f64..=MAX_REFRESH as f64).contains(&r) => {
                r as u32
            }
            Some(r) => {
                ctx.err(
                    "refresh",
                    format!(
                        "{r} seconds; use 0 (load once) or {MIN_REFRESH} to {MAX_REFRESH} seconds"
                    ),
                );
                DEFAULT_REFRESH
            }
        };
        let source = o.get("source").and_then(|s| source(s, &mut ctx));
        if source.is_none() && !o.contains_key("source") && refresh != DEFAULT_REFRESH {
            ctx.err("refresh", "a widget without a source never refreshes");
        }
        let mut layouts = Vec::new();
        match (o.get("layout"), o.get("layouts")) {
            (Some(l), None) => {
                if let Some(n) = node(l, "layout", 0, &mut ctx) {
                    layouts.extend(sizes.iter().map(|z| (*z, n.clone())));
                }
            }
            (None, Some(Value::Object(m))) => {
                for (k, l) in m {
                    let p = format!("layouts.{k}");
                    match Size::parse(k) {
                        Some(z) if sizes.contains(&z) => {
                            if let Some(n) = node(l, &p, 0, &mut ctx) {
                                layouts.push((z, n));
                            }
                        }
                        Some(_) => ctx.err(&p, format!("\"{k}\" is not in \"sizes\"")),
                        None => ctx.err(&p, "use small, medium or large"),
                    }
                }
                for z in &sizes {
                    if !m.contains_key(z.id()) {
                        ctx.err("layouts", format!("missing a layout for \"{}\"", z.id()));
                    }
                }
                layouts.sort_by_key(|(z, _)| *z);
            }
            (Some(_), Some(_)) => ctx.err(root, "use \"layout\" or \"layouts\", not both"),
            (None, Some(_)) => ctx.err("layouts", "expected {\"small\": {…}, \"medium\": {…}}"),
            (None, None) => ctx.err(root, "missing \"layout\""),
        }
        if ctx.problems.is_empty() {
            Ok(Manifest {
                id: id.into(),
                name,
                description,
                icon,
                sizes,
                refresh,
                source,
                layouts,
                sample: o.get("sample").cloned(),
            })
        } else {
            Err(ctx.problems)
        }
    }

    pub fn layout(&self, size: Size) -> Option<&Node> {
        self.layouts
            .iter()
            .find(|(z, _)| *z == size)
            .map(|(_, n)| n)
    }

    /// Bindings that `data` does not satisfy (`wol widgets validate`, list items checked on the first element).
    pub fn unresolved(&self, data: &Value) -> Vec<String> {
        let mut out = Vec::new();
        for (size, n) in &self.layouts {
            let mut ts = Vec::new();
            n.templates(false, &mut ts);
            for (t, in_item) in ts {
                let first = first_item(n, data);
                let scope = match (in_item, &first) {
                    (true, Some(i)) => Scope::root(data).with_item(i),
                    (true, None) => continue,
                    (false, _) => Scope::root(data),
                };
                for p in t.resolves(scope) {
                    let s = format!("{}: {{{{{p}}}}} is not in the data", size.id());
                    if !out.contains(&s) {
                        out.push(s);
                    }
                }
            }
        }
        out
    }
}

/// First element of the first list in `n` (sample check of item bindings).
fn first_item(n: &Node, data: &Value) -> Option<Value> {
    match n {
        Node::List { items, .. } => items.array(Scope::root(data))?.into_iter().next(),
        Node::Stack { children, .. } | Node::Row { children, .. } | Node::Card { children, .. } => {
            children.iter().find_map(|c| first_item(c, data))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const GOOD: &str = r#"{
      "name": "Game friends",
      "description": "Who is online",
      "icon": "input-gaming-symbolic",
      "sizes": ["medium", "small"],
      "refresh": 120,
      "source": {"command": ["./friends.sh", "--json"], "timeout": 4},
      "_comment": "ignored",
      "layouts": {
        "small": {"type": "stack", "children": [
          {"type": "label", "text": "{{online.length}}", "role": "display"},
          {"type": "label", "text": "online", "role": "secondary"}
        ]},
        "medium": {"type": "stack", "spacing": 8, "children": [
          {"type": "row", "children": [
            {"type": "icon", "name": "input-gaming-symbolic", "tone": "accent"},
            {"type": "label", "text": "Friends", "role": "title"},
            {"type": "spacer"},
            {"type": "badge", "text": "{{online.length}}"}
          ]},
          {"type": "list", "items": "{{online}}", "max": 3, "empty": "Nobody online",
           "item": {"type": "row", "children": [
             {"type": "label", "text": "{{item.name}}"},
             {"type": "spacer"},
             {"type": "button", "icon": "media-playback-start-symbolic", "run": ["./join.sh", "{{item.id}}"]}
           ]}},
          {"type": "gauge", "value": "{{cpu}}", "label": "CPU", "size": "small"},
          {"type": "chart", "kind": "bar", "values": "{{history}}", "max": 100, "size": "large"},
          {"type": "progress", "value": "{{xp}}", "max": 1000, "tone": "success"},
          {"type": "button", "label": "Open", "open": "steam://friends/{{online[0].id}}"},
          {"type": "button", "label": "Calendar", "app": "org.gnome.Calendar"},
          {"type": "card", "children": [{"type": "image", "src": "avatar.png", "size": "small"}]}
        ]}
      },
      "sample": {"online": [{"name": "Dinah", "id": "7"}], "cpu": 20, "history": [1, 2], "xp": 300}
    }"#;

    #[test]
    fn parses_a_complete_package() {
        let m = Manifest::parse("game-friends", GOOD).unwrap();
        assert_eq!(m.name, "Game friends");
        assert_eq!(m.sizes, vec![Size::Small, Size::Medium]);
        assert_eq!(m.refresh, 120);
        assert_eq!(
            m.source,
            Some(Source::Command {
                argv: vec!["./friends.sh".into(), "--json".into()],
                timeout: 4
            })
        );
        assert!(matches!(m.layout(Size::Small), Some(Node::Stack { .. })));
        assert!(m.layout(Size::Large).is_none());
        assert!(m.unresolved(m.sample.as_ref().unwrap()).is_empty());
        assert_eq!(
            m.unresolved(&json!({"online": [{"name": "x"}]})),
            vec![
                "medium: {{item.id}} is not in the data",
                "medium: {{cpu}} is not in the data",
                "medium: {{history}} is not in the data",
                "medium: {{xp}} is not in the data",
                "medium: {{online.0.id}} is not in the data",
            ]
        );
    }

    #[test]
    fn single_layout_serves_every_size_and_defaults_apply() {
        let m = Manifest::parse(
            "hello",
            r#"{"name": "Hello", "layout": {"type": "label", "text": "Hi"}}"#,
        )
        .unwrap();
        assert_eq!(m.sizes, vec![Size::Small]);
        assert_eq!(m.icon, DEFAULT_ICON);
        assert_eq!(m.refresh, DEFAULT_REFRESH);
        assert_eq!(m.source, None);
        assert_eq!(
            m.layout(Size::Small),
            Some(&Node::Label {
                text: Template::parse("Hi").unwrap(),
                role: Role::Body,
                tone: Tone::Text,
                lines: 1,
                align: Align::Start
            })
        );
    }

    /// Table: (widget.json, expected problems as "path: message" prefixes).
    #[test]
    fn reports_every_problem_with_its_path() {
        let cases: &[(&str, &[&str])] = &[
            ("{", &["widget.json: line 1, column 1: EOF"]),
            ("[]", &["widget.json: expected a JSON object"]),
            (
                r#"{"layout": {"type": "spacer"}}"#,
                &["name: missing \"name\""],
            ),
            (r#"{"name": "x"}"#, &["widget: missing \"layout\""]),
            (
                r#"{"name": "x", "colour": 1, "layout": {"type": "spacer"}}"#,
                &["widget.colour: unknown key; widget.json takes name,"],
            ),
            (
                r#"{"name": "x", "sizes": ["small", "huge", "small"], "layout": {"type": "spacer"}}"#,
                &[
                    "sizes[1]: \"huge\" is not one of small, medium, large",
                    "sizes[2]: listed twice",
                ],
            ),
            (
                r#"{"name": "x", "refresh": 5, "source": {"file": "d.json"}, "layout": {"type": "spacer"}}"#,
                &["refresh: 5 seconds; use 0 (load once) or 60 to 86400 seconds"],
            ),
            (
                r#"{"name": "x", "layout": {"type": "label", "text": "{{a", "role": "huge", "colour": "red"}}"#,
                &[
                    "layout.colour: unknown key; label takes type, text, role, tone, lines, align",
                    "layout.text: \"{{\" without a closing \"}}\"",
                ],
            ),
            (
                r#"{"name": "x", "layout": {"type": "label", "text": "a", "role": "huge"}}"#,
                &[
                    "layout.role: \"huge\" is not one of display, large, title, body, secondary, caption",
                ],
            ),
            (
                r#"{"name": "x", "layout": {"type": "stack", "spacing": 10, "children": [{"type": "blink"}, 3, {"text": "a"}]}}"#,
                &[
                    "layout.spacing: 10 is not on the spacing scale [0, 4, 8, 12, 16, 24]",
                    "layout.children[0].type: unknown node type \"blink\"; use one of stack, row,",
                    "layout.children[1]: expected a node object, found a number",
                    "layout.children[2]: missing \"type\"",
                ],
            ),
            (
                r#"{"name": "x", "layout": {"type": "list", "items": "friends", "item": {"type": "spacer"}}}"#,
                &["layout.items: expected one binding like \"{{friends}}\""],
            ),
            (
                r#"{"name": "x", "layout": {"type": "button", "label": "Go", "run": ["{{cmd}}"]}}"#,
                &["layout.run[0]: the program must be written out, not taken from data"],
            ),
            (
                r#"{"name": "x", "layout": {"type": "button", "label": "Go", "run": ["a"], "app": "b"}}"#,
                &["layout: a button needs exactly one of \"run\", \"open\", \"app\""],
            ),
            (
                r#"{"name": "x", "layout": {"type": "button", "run": ["a"]}}"#,
                &["layout: a button needs \"label\" or \"icon\""],
            ),
            (
                r#"{"name": "x", "layout": {"type": "button", "label": "a", "open": "javascript:alert(1)"}}"#,
                &["layout.open: expected a URL like https://"],
            ),
            (
                r#"{"name": "x", "layout": {"type": "image", "src": "https://tracker.example/p.png"}}"#,
                &["layout.src: images must be local files"],
            ),
            (
                r#"{"name": "x", "layout": {"type": "gauge", "value": "{{v}}", "max": 0}}"#,
                &["layout.max: must be greater than 0"],
            ),
            (
                r#"{"name": "x", "source": {"command": []}, "layout": {"type": "spacer"}}"#,
                &["source.command: expected a command line"],
            ),
            (
                r#"{"name": "x", "source": {"command": ["../../bin/sh"], "timeout": 99}, "layout": {"type": "spacer"}}"#,
                &[
                    "source.command[0]: \"..\" may not leave the package",
                    "source.timeout: use 1 to 30 seconds",
                ],
            ),
            (
                r#"{"name": "x", "source": {"file": "/etc/passwd"}, "layout": {"type": "spacer"}}"#,
                &["source.file: use a path inside the package"],
            ),
            (
                r#"{"name": "x", "source": {"http": "ftp://x"}, "layout": {"type": "spacer"}}"#,
                &["source.http: expected a URL like https://"],
            ),
            (
                r#"{"name": "x", "source": {"file": "a", "http": "https://a"}, "layout": {"type": "spacer"}}"#,
                &["source: use exactly one of \"command\", \"file\", \"http\""],
            ),
            (
                r#"{"name": "x", "sizes": ["small", "medium"], "layouts": {"small": {"type": "spacer"}, "large": {"type": "spacer"}}}"#,
                &[
                    "layouts.large: \"large\" is not in \"sizes\"",
                    "layouts: missing a layout for \"medium\"",
                ],
            ),
        ];
        for (json, want) in cases {
            let got: Vec<String> = Manifest::parse("w", json)
                .expect_err(json)
                .iter()
                .map(ToString::to_string)
                .collect();
            assert_eq!(got.len(), want.len(), "{json}\n{got:#?}");
            for (g, w) in got.iter().zip(want.iter()) {
                assert!(g.starts_with(w), "{json}\n got: {g}\nwant: {w}");
            }
        }
    }

    #[test]
    fn rejects_unsafe_package_ids() {
        let e = Manifest::parse("../x", r#"{"name": "x", "layout": {"type": "spacer"}}"#);
        assert!(e.is_err());
    }

    #[test]
    fn deep_or_huge_layouts_are_refused() {
        let mut deep = r#"{"type": "spacer"}"#.to_string();
        for _ in 0..=MAX_DEPTH {
            deep = format!(r#"{{"type": "stack", "children": [{deep}]}}"#);
        }
        let e = Manifest::parse("w", &format!(r#"{{"name": "x", "layout": {deep}}}"#)).unwrap_err();
        assert!(e[0].message.contains("nested deeper"), "{e:?}");
        let many = vec![r#"{"type": "spacer"}"#; MAX_NODES + 5].join(",");
        let e = Manifest::parse(
            "w",
            &format!(r#"{{"name": "x", "layout": {{"type": "stack", "children": [{many}]}}}}"#),
        )
        .unwrap_err();
        assert_eq!(e.len(), 1);
        assert!(e[0].message.contains("more than"), "{e:?}");
    }

    #[test]
    fn hosts_and_open_schemes() {
        for (url, host) in [
            ("https://api.example.com/v1?x", Some("api.example.com")),
            ("http://User@Api.Example.com:8080/", Some("api.example.com")),
            ("https://", None),
            ("nope", None),
        ] {
            assert_eq!(url_host(url).as_deref(), host, "{url}");
        }
        for (url, prefix, ok) in [
            ("https://x.org", "", true),
            ("mailto:a@b", "", true),
            ("steam://run/1", "steam://run/", true),
            ("steam://run/1", "", false),
            ("file:///etc/passwd", "", false),
            ("file:///etc/passwd", "file://", false),
            ("javascript:alert(1)", "", false),
            ("no-scheme", "", false),
        ] {
            assert_eq!(open_allowed(url, prefix), ok, "{url}");
        }
    }
}

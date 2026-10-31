//! Desktop widgets contract (archicture/adr/0010-desktop-widgets.md): the per-user layout file
//! `$XDG_CONFIG_HOME/wolos/widgets.json` and the widget package schema (`manifest`).
//! Compatibility: unknown keys and kinds survive a load/save round trip; positions are grid cells.
pub mod displays;
pub mod grid;
pub mod manifest;
pub mod template;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Under `$XDG_CONFIG_HOME`.
pub const DESKTOP_FILE: &str = "wolos/widgets.json";
/// User packages under `$XDG_DATA_HOME` (default `~/.local/share`): `<dir>/<id>/widget.json`.
pub const USER_PACKAGES_DIR: &str = "wolos/widgets";
/// Packages shipped with the system.
pub const SYSTEM_PACKAGES_DIR: &str = "/usr/share/wolos/widgets";
pub const MANIFEST_FILE: &str = "widget.json";
pub const VERSION: u32 = 1;
/// Prefix of custom widget kinds in the layout file: `custom:<package id>`.
pub const CUSTOM_PREFIX: &str = "custom:";
/// Prefix of Lua widget kinds: `lua:<file id>` for `~/.config/wolos/widgets/<file id>.lua`.
pub const LUA_PREFIX: &str = "lua:";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Size {
    Small,
    Medium,
    Large,
}

impl Size {
    pub const ALL: [Size; 3] = [Size::Small, Size::Medium, Size::Large];
    /// Grid cells (columns, rows).
    pub fn cells(self) -> (u16, u16) {
        match self {
            Size::Small => (1, 1),
            Size::Medium => (2, 1),
            Size::Large => (2, 2),
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            Size::Small => "small",
            Size::Medium => "medium",
            Size::Large => "large",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Size::Small => "Small",
            Size::Medium => "Medium",
            Size::Large => "Large",
        }
    }
    pub fn parse(s: &str) -> Option<Size> {
        Size::ALL.into_iter().find(|z| z.id() == s)
    }
}

/// Widgets that ship inside the shell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Builtin {
    Clock,
    Calendar,
    Weather,
    Music,
    Battery,
    System,
    WorldClock,
    Timer,
    Note,
}

impl Builtin {
    pub const ALL: [Builtin; 9] = [
        Builtin::Clock,
        Builtin::Calendar,
        Builtin::Weather,
        Builtin::Music,
        Builtin::Battery,
        Builtin::System,
        Builtin::WorldClock,
        Builtin::Timer,
        Builtin::Note,
    ];
    pub fn id(self) -> &'static str {
        match self {
            Builtin::Clock => "clock",
            Builtin::Calendar => "calendar",
            Builtin::Weather => "weather",
            Builtin::Music => "music",
            Builtin::Battery => "battery",
            Builtin::System => "system",
            Builtin::WorldClock => "world-clock",
            Builtin::Timer => "timer",
            Builtin::Note => "note",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Builtin::Clock => "Clock",
            Builtin::Calendar => "Calendar",
            Builtin::Weather => "Weather",
            Builtin::Music => "Music",
            Builtin::Battery => "Batteries",
            Builtin::System => "System",
            Builtin::WorldClock => "World Clock",
            Builtin::Timer => "Timer",
            Builtin::Note => "Note",
        }
    }
    pub fn description(self) -> &'static str {
        match self {
            Builtin::Clock => "The time, as a dial or in digits.",
            Builtin::Calendar => "This month and your next events.",
            Builtin::Weather => "Conditions and forecast for a city you choose.",
            Builtin::Music => "What is playing, with controls.",
            Builtin::Battery => "Charge of this computer and connected devices.",
            Builtin::System => "Processor and memory use.",
            Builtin::WorldClock => "The time in the cities you choose.",
            Builtin::Timer => "Focus sessions with breaks, a countdown or a stopwatch.",
            Builtin::Note => "A note on the desktop. Click it to write.",
        }
    }
    pub fn parse(s: &str) -> Option<Builtin> {
        Builtin::ALL.into_iter().find(|b| b.id() == s)
    }
}

/// What a placed widget shows. Kinds this version does not know stay `Unknown` and are saved back.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Builtin(Builtin),
    /// Package id under the widget package dirs.
    Custom(String),
    /// A Lua widget file (`wol_lua`), by its id.
    Lua(String),
    Unknown(String),
}

impl Kind {
    pub fn parse(s: &str) -> Kind {
        if let Some(b) = Builtin::parse(s) {
            return Kind::Builtin(b);
        }
        if let Some(id) = s.strip_prefix(LUA_PREFIX) {
            return if is_package_id(id) {
                Kind::Lua(id.into())
            } else {
                Kind::Unknown(s.into())
            };
        }
        match s.strip_prefix(CUSTOM_PREFIX) {
            Some(id) if is_package_id(id) => Kind::Custom(id.into()),
            _ => Kind::Unknown(s.into()),
        }
    }
    pub fn id(&self) -> String {
        match self {
            Kind::Builtin(b) => b.id().into(),
            Kind::Custom(id) => format!("{CUSTOM_PREFIX}{id}"),
            Kind::Lua(id) => format!("{LUA_PREFIX}{id}"),
            Kind::Unknown(s) => s.clone(),
        }
    }
}

impl Serialize for Kind {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.id())
    }
}

impl<'de> Deserialize<'de> for Kind {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Kind::parse(&String::deserialize(d)?))
    }
}

/// Package ids are directory names: lowercase ASCII letters, digits and `-`, 1..=64 long.
pub fn is_package_id(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !s.starts_with('-')
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Face {
    Analog,
    Digital,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Units {
    Celsius,
    Fahrenheit,
}

/// A city the user picked (geocoding search); never derived from the network address.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Place {
    pub name: String,
    /// Region and country, e.g. "Lisbon, Portugal".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub region: String,
    pub lat: f64,
    pub lon: f64,
}

/// Per-widget settings. Keys from newer versions land in `extra` and are written back.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Options {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub face: Option<Face>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<Place>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub units: Option<Units>,
    /// World Clock: IANA zones, in the order shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cities: Option<Vec<String>>,
    /// Timer: "focus", "stopwatch", or countdown minutes ("10").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timer: Option<String>,
    /// Note: what it says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Options {
    pub fn is_empty(&self) -> bool {
        self == &Options::default()
    }
}

/// One widget on the desktop: kind, size and top-left grid cell.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Placed {
    pub id: String,
    pub kind: Kind,
    pub size: Size,
    pub col: u16,
    pub row: u16,
    #[serde(default, skip_serializing_if = "Options::is_empty")]
    pub options: Options,
    /// The display it lives on (`displays`); None: the main display.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
}

/// Network access the user allowed for one widget package (http sources).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    pub widget: String,
    pub host: String,
}

/// `widgets.json`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Desktop {
    pub widgets: Vec<Placed>,
    pub grants: Vec<Grant>,
    /// Entries this version could not read; written back unchanged.
    pub unreadable: Vec<Value>,
    /// Top-level keys from newer versions.
    pub extra: Map<String, Value>,
}

impl Desktop {
    /// Fails only when the file is not a JSON object; bad entries are kept aside, not dropped.
    pub fn parse(json: &str) -> Result<Desktop, String> {
        let v: Value = serde_json::from_str(json).map_err(|e| format!("widgets.json: {e}"))?;
        let Value::Object(mut m) = v else {
            return Err("widgets.json: not a JSON object".into());
        };
        let mut d = Desktop::default();
        if let Some(Value::Array(ws)) = m.remove("widgets") {
            for w in ws {
                match serde_json::from_value::<Placed>(w.clone()) {
                    Ok(p) if !d.widgets.iter().any(|q| q.id == p.id) => d.widgets.push(p),
                    _ => d.unreadable.push(w),
                }
            }
        }
        if let Some(g) = m.remove("grants") {
            d.grants = serde_json::from_value(g).unwrap_or_default();
        }
        m.remove("version");
        d.extra = m;
        Ok(d)
    }

    pub fn to_json(&self) -> String {
        let mut m = self.extra.clone();
        m.insert("version".into(), VERSION.into());
        let mut ws: Vec<Value> = self
            .widgets
            .iter()
            .filter_map(|w| serde_json::to_value(w).ok())
            .collect();
        ws.extend(self.unreadable.iter().cloned());
        m.insert("widgets".into(), Value::Array(ws));
        if !self.grants.is_empty() {
            m.insert(
                "grants".into(),
                serde_json::to_value(&self.grants).unwrap_or_default(),
            );
        }
        let mut s = serde_json::to_string_pretty(&Value::Object(m)).unwrap_or_default();
        s.push('\n');
        s
    }

    /// Fresh instance id for `kind`: `weather`, then `weather-2`, ...
    pub fn new_id(&self, kind: &Kind) -> String {
        let base = match kind {
            Kind::Builtin(b) => b.id().to_string(),
            Kind::Custom(id) | Kind::Lua(id) | Kind::Unknown(id) => id.clone(),
        };
        (1..)
            .map(|n| {
                if n == 1 {
                    base.clone()
                } else {
                    format!("{base}-{n}")
                }
            })
            .find(|id| !self.widgets.iter().any(|w| &w.id == id))
            .unwrap_or(base)
    }

    pub fn get(&self, id: &str) -> Option<&Placed> {
        self.widgets.iter().find(|w| w.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Placed> {
        self.widgets.iter_mut().find(|w| w.id == id)
    }

    pub fn remove(&mut self, id: &str) -> Option<Placed> {
        let i = self.widgets.iter().position(|w| w.id == id)?;
        Some(self.widgets.remove(i))
    }

    /// Fail closed: only an explicit grant for exactly this package and host allows a request.
    pub fn allows(&self, widget: &str, host: &str) -> bool {
        self.grants
            .iter()
            .any(|g| g.widget == widget && g.host.eq_ignore_ascii_case(host))
    }

    pub fn grant(&mut self, widget: &str, host: &str) {
        if !self.allows(widget, host) {
            self.grants.push(Grant {
                widget: widget.into(),
                host: host.to_ascii_lowercase(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_kinds_round_trip() {
        for s in Size::ALL {
            assert_eq!(Size::parse(s.id()), Some(s));
        }
        assert_eq!(Size::Large.cells(), (2, 2));
        for (src, want) in [
            ("clock", Kind::Builtin(Builtin::Clock)),
            ("system", Kind::Builtin(Builtin::System)),
            ("world-clock", Kind::Builtin(Builtin::WorldClock)),
            ("timer", Kind::Builtin(Builtin::Timer)),
            ("note", Kind::Builtin(Builtin::Note)),
            ("custom:game-friends", Kind::Custom("game-friends".into())),
            ("custom:Bad_Id", Kind::Unknown("custom:Bad_Id".into())),
            ("custom:", Kind::Unknown("custom:".into())),
            ("lua:clock", Kind::Lua("clock".into())),
            ("lua:../x", Kind::Unknown("lua:../x".into())),
            ("notes", Kind::Unknown("notes".into())),
        ] {
            let k = Kind::parse(src);
            assert_eq!(k, want, "{src}");
            assert_eq!(k.id(), src);
        }
    }

    #[test]
    fn world_clock_cities_round_trip() {
        let o: Options = serde_json::from_str(r#"{"cities":["Asia/Tokyo"],"x":1}"#).unwrap();
        assert_eq!(o.cities.as_deref(), Some(&["Asia/Tokyo".to_string()][..]));
        let back = serde_json::to_value(&o).unwrap();
        assert_eq!(back, serde_json::json!({"cities":["Asia/Tokyo"],"x":1}));
        assert!(Options::default().is_empty());
        let t: Options = serde_json::from_str(r#"{"timer":"stopwatch"}"#).unwrap();
        assert_eq!(t.timer.as_deref(), Some("stopwatch"));
        let n: Options = serde_json::from_str(r#"{"text":"Milk\nEggs"}"#).unwrap();
        assert_eq!(n.text.as_deref(), Some("Milk\nEggs"));
    }

    #[test]
    fn package_ids_are_safe_dir_names() {
        for ok in ["a", "game-friends", "w2"] {
            assert!(is_package_id(ok), "{ok}");
        }
        for bad in ["", "-x", "A", "a/b", "..", "a b", "a_b", &"x".repeat(65)] {
            assert!(!is_package_id(bad), "{bad}");
        }
    }

    const FILE: &str = r#"{
      "version": 1,
      "theme_hint": "kept",
      "widgets": [
        {"id": "clock", "kind": "clock", "size": "small", "col": 0, "row": 0, "options": {"face": "digital"}},
        {"id": "weather", "kind": "weather", "size": "medium", "col": 1, "row": 0,
         "options": {"place": {"name": "Lisbon", "region": "Portugal", "lat": 38.72, "lon": -9.13}, "future": 3}},
        {"id": "friends", "kind": "custom:game-friends", "size": "small", "col": 0, "row": 1},
        {"id": "notes", "kind": "notes", "size": "small", "col": 3, "row": 0},
        {"id": "broken", "kind": "clock", "size": "huge", "col": 0, "row": 0},
        {"id": "clock", "kind": "clock", "size": "large", "col": 0, "row": 2}
      ],
      "grants": [{"widget": "game-friends", "host": "api.example.com"}]
    }"#;

    #[test]
    fn parses_the_layout_file_and_keeps_what_it_cannot_read() {
        let d = Desktop::parse(FILE).unwrap();
        let ids: Vec<&str> = d.widgets.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, ["clock", "weather", "friends", "notes"]);
        assert_eq!(d.widgets[0].options.face, Some(Face::Digital));
        assert_eq!(d.widgets[1].options.place.as_ref().unwrap().name, "Lisbon");
        assert_eq!(d.widgets[1].options.extra["future"], 3);
        assert_eq!(d.widgets[3].kind, Kind::Unknown("notes".into()));
        assert_eq!(d.unreadable.len(), 2, "bad size and duplicate id");
        assert_eq!(d.extra["theme_hint"], "kept");

        // Round trip: nothing is lost.
        let again = Desktop::parse(&d.to_json()).unwrap();
        assert_eq!(again, d);
        let v: Value = serde_json::from_str(&d.to_json()).unwrap();
        assert_eq!(v["version"], VERSION);
        assert_eq!(v["widgets"].as_array().unwrap().len(), 6);
    }

    #[test]
    fn rejects_files_that_are_not_objects() {
        for bad in ["", "[]", "{", "42"] {
            assert!(Desktop::parse(bad).is_err(), "{bad:?}");
        }
        assert_eq!(Desktop::parse("{}").unwrap(), Desktop::default());
    }

    #[test]
    fn empty_options_are_not_written() {
        let mut d = Desktop::default();
        d.widgets.push(Placed {
            id: "battery".into(),
            kind: Kind::Builtin(Builtin::Battery),
            size: Size::Small,
            col: 2,
            row: 1,
            options: Options::default(),
            display: None,
        });
        let j = d.to_json();
        assert!(!j.contains("options") && !j.contains("grants"), "{j}");
    }

    #[test]
    fn instance_ids_are_unique() {
        let mut d = Desktop::parse(FILE).unwrap();
        let k = Kind::Builtin(Builtin::Clock);
        assert_eq!(d.new_id(&k), "clock-2");
        assert_eq!(d.new_id(&Kind::Builtin(Builtin::Music)), "music");
        assert_eq!(
            d.new_id(&Kind::Custom("game-friends".into())),
            "game-friends"
        );
        assert!(d.remove("clock").is_some());
        assert_eq!(d.new_id(&k), "clock");
        assert!(d.remove("nope").is_none());
    }

    #[test]
    fn network_grants_fail_closed() {
        let mut d = Desktop::parse(FILE).unwrap();
        assert!(d.allows("game-friends", "api.example.com"));
        assert!(d.allows("game-friends", "API.example.com"));
        assert!(!d.allows("game-friends", "evil.example.com"));
        assert!(!d.allows("other", "api.example.com"));
        d.grant("other", "Data.Example.org");
        d.grant("other", "data.example.org");
        assert_eq!(d.grants.len(), 2);
        assert!(d.allows("other", "data.example.org"));
    }
}

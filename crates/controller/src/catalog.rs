//! Helper catalogue: the `wol` command tree ([`TREE`]) plus what each verb may touch. One source:
//! `wol api --json` prints it, `wol mcp` lists and calls exactly these verbs through the same
//! parser and handlers as the CLI. A verb is open to helpers only when listed in [`HELPER_VERBS`];
//! anything else (and anything added to the tree later) stays for people and the desktop.
use crate::cli::{Cmd, Invocation, TREE};
use serde_json::{Value, json};
use shared::shell::ApprovalRequest;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Access {
    /// Reads state; changes nothing.
    Read,
    /// User-level change (undoable or harmless).
    Change,
    /// Needs the administrator password (wol-admind). Helpers get the on-screen prompt.
    Admin,
}

/// One helper verb: tree path, access, capability (`wol_core::automation::CAPABILITIES`), and an
/// optional secret the CLI reads from stdin (never argv, never logged).
#[derive(Clone, Copy, Debug)]
pub struct HelperVerb {
    pub path: &'static [&'static str],
    pub access: Access,
    pub capability: &'static str,
    pub secret: Option<(&'static str, &'static str)>,
}

const fn hv(path: &'static [&'static str], access: Access, capability: &'static str) -> HelperVerb {
    HelperVerb {
        path,
        access,
        capability,
        secret: None,
    }
}

use Access::{Admin, Change, Read};

pub const HELPER_VERBS: &[HelperVerb] = &[
    hv(&["about"], Read, "read"),
    hv(&["sys"], Read, "read"),
    hv(&["update", "check"], Read, "read"),
    hv(&["settings"], Read, "read"),
    hv(&["theme", "get"], Read, "read"),
    hv(&["theme", "list"], Read, "read"),
    hv(&["theme", "set"], Change, "appearance"),
    hv(&["appearance"], Read, "read"),
    hv(&["appearance", "set"], Change, "appearance"),
    hv(&["accent", "set"], Change, "appearance"),
    hv(&["wallpaper"], Read, "read"),
    hv(&["wallpaper", "set"], Change, "appearance"),
    hv(&["audio"], Read, "read"),
    hv(&["audio", "volume"], Change, "sound"),
    hv(&["audio", "mute"], Change, "sound"),
    hv(&["audio", "unmute"], Change, "sound"),
    hv(&["display"], Read, "read"),
    hv(&["display", "brightness"], Change, "display"),
    hv(&["battery"], Read, "read"),
    hv(&["network"], Read, "read"),
    hv(&["hw"], Read, "read"),
    hv(&["hw", "list"], Read, "read"),
    hv(&["hw", "status"], Read, "read"),
    hv(&["wifi"], Read, "read"),
    hv(&["wifi", "on"], Admin, "network"),
    hv(&["wifi", "off"], Admin, "network"),
    HelperVerb {
        secret: Some(("password", "Wi-Fi password (secured networks only)")),
        ..hv(&["wifi", "join"], Admin, "network")
    },
    hv(&["bluetooth"], Read, "read"),
    hv(&["bluetooth", "on"], Admin, "network"),
    hv(&["bluetooth", "off"], Admin, "network"),
    hv(&["focus"], Read, "read"),
    hv(&["focus", "on"], Change, "focus"),
    hv(&["focus", "off"], Change, "focus"),
    hv(&["open"], Change, "apps"),
    hv(&["notify"], Change, "notify"),
    hv(&["screenshot"], Change, "screen-capture"),
    hv(&["windows"], Read, "window-list"),
    hv(&["windows", "focus"], Change, "window-list"),
    hv(&["files", "tidy"], Change, "files"),
    hv(&["history"], Read, "restore"),
    hv(&["undo"], Change, "restore"),
    hv(&["restore", "list"], Read, "restore"),
    hv(&["restore", "activate"], Admin, "restore"),
];

/// A visible command and, when helpers may call it, its rule.
#[derive(Clone, Copy, Debug)]
pub struct Verb {
    pub cmd: &'static Cmd,
    pub helper: Option<&'static HelperVerb>,
}

impl Verb {
    /// MCP tool name: path words joined by `_` (`theme_set`).
    pub fn tool(&self) -> String {
        self.cmd.path.join("_")
    }
    pub fn access(&self) -> Option<Access> {
        self.helper.map(|h| h.access)
    }
    /// `wol theme set <id>` usage line.
    pub fn usage(&self) -> String {
        let mut u = format!("wol {}", self.cmd.path.join(" "));
        for a in self.cmd.args {
            let n = if a.choices.is_empty() {
                a.name.to_string()
            } else {
                a.choices.join("|")
            };
            u.push_str(&match (a.optional, a.rest) {
                (_, true) => format!(" [{n}...]"),
                (true, _) => format!(" [{n}]"),
                _ => format!(" <{n}>"),
            });
        }
        for f in self.cmd.flags {
            u.push_str(&format!(" [--{f}]"));
        }
        u
    }
}

/// Every visible command, in tree order.
pub fn verbs() -> Vec<Verb> {
    TREE.iter()
        .filter(|c| !c.hidden)
        .map(|cmd| Verb {
            cmd,
            helper: HELPER_VERBS.iter().find(|h| h.path == cmd.path),
        })
        .collect()
}

/// Verbs `wol mcp` offers.
pub fn tools() -> Vec<Verb> {
    verbs().into_iter().filter(|v| v.helper.is_some()).collect()
}

pub fn find_tool(name: &str) -> Option<Verb> {
    tools().into_iter().find(|v| v.tool() == name)
}

/// The rule for an invocation, when its command is a helper verb.
pub fn rule_of(inv: &Invocation) -> Option<&'static HelperVerb> {
    HELPER_VERBS.iter().find(|h| h.path == inv.path.as_slice())
}

fn arg_schema(name: &str, choices: &[&str]) -> Value {
    if !choices.is_empty() {
        json!({"type": "string", "enum": choices})
    } else if name == "percent" {
        json!({"type": "integer", "minimum": 0, "maximum": 100})
    } else {
        json!({"type": "string"})
    }
}

/// JSON Schema of the tool arguments: one property per positional (rest = one string), plus the
/// stdin secret.
pub fn input_schema(v: &Verb) -> Value {
    let mut props = serde_json::Map::new();
    let mut required = Vec::new();
    for a in v.cmd.args {
        props.insert(a.name.into(), arg_schema(a.name, a.choices));
        if !a.optional && !a.rest {
            required.push(a.name);
        }
    }
    if let Some((name, desc)) = v.helper.and_then(|h| h.secret) {
        props.insert(name.into(), json!({"type": "string", "description": desc}));
    }
    json!({"type": "object", "properties": props, "required": required, "additionalProperties": false})
}

const ADMIN_NOTE: &str = " Needs your approval: WolOS shows its password prompt on screen; nothing changes until you allow it.";

pub fn description(v: &Verb) -> String {
    let mut d = format!("{} (`{}`).", v.cmd.help, v.usage());
    if v.access() == Some(Admin) {
        d.push_str(ADMIN_NOTE);
    }
    if v.cmd.path == ["files", "tidy"] {
        d.push_str(" Call without plan first (dry run), show the moves, then call with that plan id. Undo with the undo tool.");
    }
    d
}

/// MCP tool definition.
pub fn tool_json(v: &Verb) -> Value {
    let a = v.access().unwrap_or(Read);
    let last = v.cmd.path.last().copied().unwrap_or("");
    let destructive = matches!(
        v.cmd.path,
        ["files", "tidy"] | ["restore", "activate"] | ["undo"]
    );
    let idempotent = a == Read
        || matches!(
            last,
            "set" | "on" | "off" | "mute" | "unmute" | "volume" | "brightness" | "activate"
        );
    json!({
        "name": v.tool(),
        "title": v.cmd.help,
        "description": description(v),
        "inputSchema": input_schema(v),
        "annotations": {
            "readOnlyHint": a == Read,
            "destructiveHint": destructive,
            "idempotentHint": idempotent,
            "openWorldHint": v.cmd.path == ["open"],
        },
    })
}

/// `wol api --json`.
pub fn api_json() -> Value {
    let verbs: Vec<Value> = verbs()
        .iter()
        .map(|v| {
            let args: Vec<Value> = v
                .cmd
                .args
                .iter()
                .map(|a| json!({"name": a.name, "required": !a.optional && !a.rest, "rest": a.rest, "choices": a.choices}))
                .collect();
            json!({
                "command": v.usage(),
                "path": v.cmd.path,
                "help": v.cmd.help,
                "args": args,
                "flags": v.cmd.flags,
                "helpers": v.helper.is_some(),
                "access": v.access(),
                "admin": v.access() == Some(Admin),
                "capability": v.helper.map(|h| h.capability),
                "stdin": v.helper.and_then(|h| h.secret).map(|s| s.0),
                "tool": v.helper.map(|_| v.tool()),
            })
        })
        .collect();
    let mut exits = serde_json::Map::new();
    for c in shared::errors::ErrorCode::ALL {
        let k = c.exit().to_string();
        let name = match exits.get(&k).and_then(Value::as_str) {
            Some(prev) => format!("{prev}|{}", c.name()),
            None => c.name().to_string(),
        };
        exits.insert(k, name.into());
    }
    json!({
        "version": 1,
        "global_flags": crate::cli::GLOBAL_FLAGS,
        "exit_codes": exits,
        "verbs": verbs,
    })
}

/// MCP arguments -> `wol` argv (`--json`, then `--` so no value is read as a flag) and the
/// stdin secret. Unknown or missing arguments are refused.
pub fn argv(
    v: &Verb,
    args: &serde_json::Map<String, Value>,
) -> Result<(Vec<String>, Option<String>), String> {
    let text = |k: &str, x: &Value| -> Result<String, String> {
        match x {
            Value::String(s) => Ok(s.clone()),
            Value::Number(n) => Ok(n.to_string()),
            Value::Bool(b) => Ok(b.to_string()),
            _ => Err(format!("argument '{k}' must be a string")),
        }
    };
    let secret_name = v.helper.and_then(|h| h.secret).map(|s| s.0);
    if let Some(k) = args
        .keys()
        .find(|k| Some(k.as_str()) != secret_name && !v.cmd.args.iter().any(|a| a.name == *k))
    {
        return Err(format!("unknown argument '{k}'"));
    }
    let mut out: Vec<String> = v.cmd.path.iter().map(|s| s.to_string()).collect();
    out.push("--json".into());
    out.push("--".into());
    let mut skipped: Option<&str> = None;
    for a in v.cmd.args {
        match args.get(a.name) {
            Some(x) => {
                if let Some(s) = skipped {
                    return Err(format!("argument '{}' needs '{s}' too", a.name));
                }
                out.push(text(a.name, x)?);
            }
            None if a.optional || a.rest => skipped = Some(a.name),
            None => return Err(format!("missing argument '{}'", a.name)),
        }
    }
    let secret = match secret_name.and_then(|k| args.get(k).map(|x| (k, x))) {
        Some((k, x)) => Some(text(k, x)?),
        None => None,
    };
    Ok((out, secret))
}

/// Whether a helper calling `inv` changes something (and so asks first). A tidy without a plan
/// is a dry run.
pub fn changes(inv: &Invocation) -> bool {
    match rule_of(inv).map(|h| h.access) {
        Some(Read) | None => false,
        Some(_) => !(inv.action == "files.tidy" && inv.get("plan").is_none()),
    }
}

/// Sentence end for "<helper> wants to …": exactly what `inv` changes, in plain words.
pub fn describe(inv: &Invocation) -> String {
    let get = |k: &str| inv.get(k).unwrap_or_default().to_string();
    match inv.action {
        "appearance.set" => match get("mode").as_str() {
            "light" => "switch to Light appearance".into(),
            "dark" => "switch to Dark appearance".into(),
            _ => "switch appearance to Auto (light by day, dark at night)".into(),
        },
        "theme.set" => format!("switch the theme to “{}”", get("id")),
        "accent.set" => format!("change the accent colour to {}", get("accent")),
        "wallpaper.set" => {
            let p = get("path");
            let name = std::path::Path::new(&p)
                .file_name()
                .map_or(p.clone(), |n| n.to_string_lossy().into_owned());
            format!("use “{name}” as the wallpaper")
        }
        "audio.volume" => format!("set the volume to {}%", get("percent")),
        "audio.mute" => "mute the sound".into(),
        "audio.unmute" => "unmute the sound".into(),
        "display.brightness" => format!("set the display brightness to {}%", get("percent")),
        "focus.on" => "turn Do Not Disturb on".into(),
        "focus.off" => "turn Do Not Disturb off".into(),
        "open" => format!("open “{}”", get("target")),
        "notify" => format!("show the notification “{}”", get("title")),
        "screenshot" => "take a screenshot of the whole screen".into(),
        "windows.focus" => format!("bring the window “{}” forward", get("window")),
        "files.tidy" => "sort the files in Downloads into folders".into(),
        "undo" => match inv.get("id") {
            Some(id) => format!("undo settings change #{id}"),
            None => "undo the last settings change".into(),
        },
        _ => match approval(inv, None) {
            Ok(r) => r.describe(),
            Err(_) => format!("run `wol {}`", inv.path.join(" ")),
        },
    }
}

/// The on-screen approval for an admin verb (helpers never run these themselves).
pub fn approval(inv: &Invocation, secret: Option<String>) -> Result<ApprovalRequest, String> {
    let get = |k: &str| inv.get(k).unwrap_or_default().to_string();
    Ok(match inv.action {
        "wifi.on" | "wifi.off" => ApprovalRequest::Wifi {
            on: inv.action == "wifi.on",
        },
        "bluetooth.on" | "bluetooth.off" => ApprovalRequest::Bluetooth {
            on: inv.action == "bluetooth.on",
        },
        "wifi.join" => {
            let ssid = get("ssid");
            if let Some(e) = wol_core::wifi::validate(&ssid, secret.as_deref()) {
                return Err(e.to_string());
            }
            ApprovalRequest::WifiJoin {
                ssid,
                password: secret,
            }
        }
        "restore.activate" => {
            let name = get("be");
            if !wol_core::update::is_valid_be(&name) {
                return Err(format!("bad restore point '{name}'"));
            }
            ApprovalRequest::BootEnvironment { name }
        }
        a => return Err(format!("{a} has no approval")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_helper_verb_is_in_the_tree_with_help_and_a_known_capability() {
        for h in HELPER_VERBS {
            let cmd = TREE
                .iter()
                .find(|c| c.path == h.path && !c.hidden)
                .unwrap_or_else(|| panic!("{:?} not a visible command", h.path));
            assert!(!cmd.help.is_empty(), "{:?}", h.path);
            assert!(
                wol_core::automation::policy::label(h.capability).is_some(),
                "{:?}: capability {}",
                h.path,
                h.capability
            );
        }
        let mut names: Vec<String> = tools().iter().map(Verb::tool).collect();
        names.sort();
        let n = names.len();
        names.dedup();
        assert_eq!(names.len(), n, "tool names unique");
        assert_eq!(n, HELPER_VERBS.len());
        for t in &names {
            assert!(
                t.len() <= 64
                    && t.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
                "{t}"
            );
        }
    }

    #[test]
    fn argv_from_arguments() {
        let v = find_tool("theme_set").unwrap();
        let m = |j: Value| j.as_object().unwrap().clone();
        assert_eq!(
            argv(&v, &m(json!({"id": "nord"}))).unwrap().0,
            ["theme", "set", "--json", "--", "nord"]
        );
        assert!(argv(&v, &m(json!({}))).unwrap_err().contains("missing"));
        assert!(
            argv(&v, &m(json!({"id": "x", "rm": 1})))
                .unwrap_err()
                .contains("unknown")
        );
        let vol = find_tool("audio_volume").unwrap();
        assert_eq!(argv(&vol, &m(json!({"percent": 40}))).unwrap().0[4], "40");
        let j = find_tool("wifi_join").unwrap();
        let (a, s) = argv(&j, &m(json!({"ssid": "--Home", "password": "hunter22"}))).unwrap();
        assert_eq!(
            a,
            ["wifi", "join", "--json", "--", "--Home"],
            "flag-like value stays a value"
        );
        assert_eq!(s.as_deref(), Some("hunter22"));
        assert!(
            !a.iter().any(|x| x.contains("hunter")),
            "secret never in argv"
        );
    }

    #[test]
    fn every_admin_tool_maps_to_an_on_screen_approval() {
        for v in tools().iter().filter(|v| v.access() == Some(Admin)) {
            let mut words: Vec<&str> = v.cmd.path.to_vec();
            for a in v.cmd.args.iter().filter(|a| !a.optional) {
                words.push(if a.name == "be" { "wol-1" } else { "Home" });
            }
            let inv = crate::cli::parse(&words).unwrap();
            assert!(approval(&inv, None).is_ok(), "{}", v.tool());
        }
        let inv = crate::cli::parse(&["restore", "activate", "a;b"]).unwrap();
        assert!(approval(&inv, None).is_err());
    }

    /// The approval sheet says exactly what will change, in plain words, for every change verb.
    #[test]
    fn every_change_is_described_in_plain_words() {
        let p = |w: &[&str]| crate::cli::parse(w).unwrap();
        for (words, want) in [
            (
                &["appearance", "set", "dark"][..],
                "switch to Dark appearance",
            ),
            (
                &["appearance", "set", "auto"],
                "switch appearance to Auto (light by day, dark at night)",
            ),
            (&["theme", "set", "nord"], "switch the theme to “nord”"),
            (
                &["accent", "set", "pink"],
                "change the accent colour to pink",
            ),
            (
                &["wallpaper", "set", "/home/a/Pictures/sea.jpg"],
                "use “sea.jpg” as the wallpaper",
            ),
            (&["audio", "volume", "40"], "set the volume to 40%"),
            (&["audio", "mute"], "mute the sound"),
            (&["audio", "unmute"], "unmute the sound"),
            (
                &["display", "brightness", "70"],
                "set the display brightness to 70%",
            ),
            (&["focus", "on"], "turn Do Not Disturb on"),
            (&["focus", "off"], "turn Do Not Disturb off"),
            (&["open", "https://wolos.org"], "open “https://wolos.org”"),
            (&["notify", "Hi", "there"], "show the notification “Hi”"),
            (&["screenshot"], "take a screenshot of the whole screen"),
            (
                &["windows", "focus", "foot"],
                "bring the window “foot” forward",
            ),
            (
                &["files", "tidy", "ab12"],
                "sort the files in Downloads into folders",
            ),
            (&["undo"], "undo the last settings change"),
            (&["undo", "4"], "undo settings change #4"),
        ] {
            assert_eq!(describe(&p(words)), want, "{words:?}");
        }
        for v in tools().iter().filter(|v| v.access() == Some(Change)) {
            let mut words: Vec<&str> = v.cmd.path.to_vec();
            for a in v.cmd.args.iter().filter(|a| !a.optional) {
                words.push(a.choices.first().copied().unwrap_or("x"));
            }
            assert!(!describe(&p(&words)).starts_with("run "), "{}", v.tool());
        }
        assert!(changes(&p(&["files", "tidy", "ab12"])));
        assert!(
            !changes(&p(&["files", "tidy"])),
            "a dry run changes nothing"
        );
        assert!(!changes(&p(&["theme", "get"])));
        assert!(changes(&p(&["wifi", "off"])));
    }

    #[test]
    fn api_json_lists_every_visible_verb() {
        let a = api_json();
        let verbs = a["verbs"].as_array().unwrap();
        assert_eq!(verbs.len(), TREE.iter().filter(|c| !c.hidden).count());
        let set = verbs
            .iter()
            .find(|v| v["command"] == "wol theme set <id>")
            .unwrap();
        assert_eq!(
            (set["access"].as_str(), set["admin"].as_bool()),
            (Some("change"), Some(false))
        );
        let off = verbs.iter().find(|v| v["tool"] == "wifi_off").unwrap();
        assert_eq!(off["admin"], true);
        let shell = verbs
            .iter()
            .find(|v| {
                v["command"]
                    .as_str()
                    .unwrap()
                    .starts_with("wol shell toggle")
            })
            .unwrap();
        assert_eq!(
            (shell["helpers"].as_bool(), shell["tool"].is_null()),
            (Some(false), true)
        );
        assert_eq!(a["exit_codes"]["77"], "denied");
        assert_eq!(a["exit_codes"]["69"], "unavailable|unsupported");
    }
}

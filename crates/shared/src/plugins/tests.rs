// SPDX-License-Identifier: BSD-2-Clause
//! Unit tests of the plugin manifest: parsing, permissions, entries and the layout slots.
use super::*;

const STATS: &str = r#"
schema = 1
id = "org.wolos.stats"
name = "Stats"
version = "1.0"
author = "WolOS"
license = "GPL-2.0-or-later"
description = "CPU and memory in the menu bar, with a desktop card." # trailing comment
min_wolos = "0.12"
permissions = ["run", "notifications"]

[[menubar]]
id = "cpu"
script = "cpu.lua"
side = "right"

[[widget]]
id = "card"
script = "card.lua"
sizes = [
  "small",
  "medium",  # medium too
]

[[toggle]]
id = "awake"
on = ["wol", "power", "awake", "on"]
off = ["wol", "power", "awake", "off"]

[[tile]]
id = "awake-tile"
icon = "weather-clear-symbolic"
label = "Stay Awake"
toggle = "awake"

[[hotkey]]
id = "top"
default = "Super+Shift+T"
exec = ["foot", "-e", "btop"]

[[webapp]]
id = "mail"
name = "Mail"
url = "https://mail.example.com/"
icon = "mail.png"

[[hypr]]
id = "rules"
file = "rules.conf"
"#;

#[test]
fn example_parses_and_round_trips_through_to_toml() {
    let m = parse(STATS).unwrap();
    assert_eq!(
        (m.schema, m.id.as_str(), m.name.as_str()),
        (1, "org.wolos.stats", "Stats")
    );
    assert_eq!(m.permissions, [Permission::Notifications, Permission::Run]);
    assert_eq!(m.menubar[0].side, Side::Right);
    assert_eq!(m.widget[0].sizes, [Size::Small, Size::Medium]);
    assert_eq!(m.hotkey[0].default, "super+shift+t");
    assert_eq!(m.hotkey[0].combo(), (vec!["super", "shift"], "t"));
    assert_eq!(m.webapp[0].profile, Profile::Shared);
    assert_eq!(
        m.kinds(),
        [
            Kind::Menubar,
            Kind::Widget,
            Kind::Tile,
            Kind::Toggle,
            Kind::Hotkey,
            Kind::Webapp,
            Kind::Hypr
        ]
    );
    assert_eq!(m.files(), ["cpu.lua", "card.lua", "mail.png", "rules.conf"]);
    assert_eq!(m.scripts().len(), 2);
    assert!(m.is_reserved());
    assert!(
        is_reserved_id("org.wolos"),
        "the bare id shares the built-in flag files"
    );
    assert!(!is_reserved_id("org.wolosx.a"));
    assert_eq!(
        m.commands().len(),
        1,
        "only the hotkey's foot; the toggle is a wol verb"
    );
    assert_eq!(parse(&m.to_toml()).unwrap(), m, "{}", m.to_toml());
    let json = serde_json::to_string(&m).unwrap();
    assert_eq!(serde_json::from_str::<Manifest>(&json).unwrap(), m);
}

#[test]
fn every_problem_is_one_sentence_naming_the_place() {
    let base = |extra: &str| {
        format!(
            "schema = 1\nid = \"alice.demo\"\nname = \"Demo\"\nversion = \"1\"\nlicense = \"MIT\"\n{extra}"
        )
    };
    let cases: Vec<(String, &str)> = vec![
        (base(""), "does nothing yet"),
        (base("id = 1\n"), "set twice"),
        (base("colour = \"red\"\n[[menubar]]\nid = \"a\"\nscript = \"a.lua\"\n"), "unknown key \"colour\""),
        (base("[[sidebar]]\nid = \"a\"\n"), "Unknown section [[sidebar]]"),
        (base("[menubar]\nid = \"a\"\n"), "two brackets"),
        (base("[[menubar]]\nid = \"a\"\n"), "[[menubar]] 1 needs script"),
        (base("[[menubar]]\nid = \"a\"\nscript = \"../x.lua\"\n"), "inside the plugin folder"),
        (base("[[menubar]]\nid = \"a\"\nscript = \"a.lua\"\nside = \"middle\"\n"), "one of left, centre, right"),
        (base("[[menubar]]\nid = \"A\"\nscript = \"a.lua\"\n"), "lowercase"),
        (base("[[menubar]]\nid = \"a\"\nscript = \"a.lua\"\n[[widget]]\nid = \"a\"\nscript = \"b.lua\"\n"), "used twice"),
        (base("[[menubar]]\nid = \"a\"\nscript = \"a.lua\"\nextra = true\n"), "it takes id, script, side"),
        (base("[[tile]]\nid = \"t\"\nicon = \"x-symbolic\"\nlabel = \"T\"\ntoggle = \"nope\"\n"), "does not define"),
        (base("[[toggle]]\nid = \"t\"\non = []\noff = [\"x\"]\n"), "command as a list"),
        (base("[[hotkey]]\nid = \"h\"\ndefault = \"t\"\nexec = [\"x\"]\n"), "key combo"),
        (base("[[hotkey]]\nid = \"h\"\ndefault = \"super+super+t\"\nexec = [\"x\"]\n"), "key combo"),
        (base("[[search]]\nid = \"s\"\nscript = \"s.lua\"\nkind = \"command\"\n"), "needs a prefix"),
        (base("[[search]]\nid = \"s\"\nscript = \"s.lua\"\nprefix = \"=\"\n"), "only for kind"),
        (base("[[webapp]]\nid = \"w\"\nname = \"W\"\nurl = \"ftp://x\"\nicon = \"x\"\n"), "https://"),
        (base("[[hypr]]\nid = \"h\"\nfile = \"h.lua\"\n"), ".conf file"),
        (base("permissions = [\"camera\"]\n[[dock]]\nid = \"d\"\nname = \"D\"\nicon = \"x\"\nexec = [\"x\"]\n"), "permission \"camera\""),
        (base("[[dock]]\nid = \"d\"\nname = \"D\"\nicon = \"x\"\nexec = \"x\"\n"), "is a list like"),
        (base("[[toggle]]\nid = \"t\"\non = [\"sh\", \"-c\", \"x\"]\noff = [\"wol\", \"focus\", \"off\"]\n"), "add \"run\" to permissions"),
        (base("[[dock]]\nid = \"d\"\nname = \"D\"\nicon = \"x\"\nexec = [\"foot\"]\n"), "runs \"foot\""),
        (base("[[hotkey]]\nid = \"h\"\ndefault = \"super+t\"\nexec = [\"foot\"]\n"), "not a wol command"),
        (base("[[dock]]\nid = \"d\"\nname = { a = 1 }\n"), "Inline tables"),
        (base("[[dock]]\nid = \"d\"\nname = \"\"\"x\"\"\"\n"), "Multi-line"),
        (base("[[dock]]\nid = \"d\"\nname = \"x\n"), "closing quote"),
        (base("[[dock]]\nid = \"d\"\nname = 1.5\n"), "whole number"),
        (base("[[dock]]\nid = \"d\" name = \"x\"\n"), "one key = value per line"),
        (base("[[dock]]\nid \"d\"\n"), "Expected = after"),
        (base("[[dock]]\nid = \"d\"\nexec = [\"a\" \"b\"]\n"), "separated by commas"),
        (base("[a.b]\n"), "Sections inside sections"),
        ("schema = 2\nid = \"a.b\"\n".into(), "needs manifest schema 2"),
        ("id = \"a.b\"\n".into(), "starts with schema = 1"),
        ("schema = 1\nid = \"Bad\"\nname = \"x\"\nversion = \"1\"\nlicense = \"MIT\"\n".into(), "reverse-DNS"),
        ("schema = 1\nid = \"a.b\"\nname = \"x\"\nversion = \"1\"\n".into(), "needs license"),
        ("schema = 1\nid = \"a.b\"\nname = \"x\"\nversion = \"1\"\nlicense = \"MIT\"\nmin_wolos = \"soon\"\n".into(), "WolOS version"),
        ("schema = 1\nid = \"a.b\"\nname = \"x\"\nversion = \"1\"\nlicense = \"MIT\"\nmenubar = 1\n".into(), "is a section"),
    ];
    for (src, want) in cases {
        let e = parse(&src).unwrap_err();
        assert!(e.contains(want), "{src:?}\n  got: {e}");
        assert_eq!(e.matches(". ").count(), 0, "one sentence: {e}");
        assert!(e.ends_with('.'), "{e}");
    }
}

#[test]
fn deep_lists_are_refused_not_recursed() {
    let deep = format!("id = {}", "[".repeat(100_000));
    let e = parse(&deep).unwrap_err();
    assert!(e.contains("nested"), "{e}");
    let ok = format!(
        "x = {}1{}",
        "[".repeat(MAX_NESTING),
        "]".repeat(MAX_NESTING)
    );
    assert!(!parse(&ok).unwrap_err().contains("nested"));
}

#[test]
fn toml_subset_reads_escapes_literals_and_comments() {
    let m = parse(
        "schema = 1 # one\nid = 'alice.demo'\nname = \"Quote\\\" \\u00e9\"\nversion = \"1_0\"\nlicense = \"MIT\"\npermissions = [\"run\"]\n\n# section\n[[ dock ]]\nid = \"d\"\nname = \"D\"\nicon = \"folder-symbolic\"\nexec = [\"sh\", \"-c\", 'echo \"hi\"']\n",
    )
    .unwrap();
    assert_eq!(m.name, "Quote\" é");
    assert_eq!(m.dock[0].exec[2], "echo \"hi\"");
}

#[test]
fn id_and_path_tables() {
    for (id, ok) in [
        ("org.wolos.stats", true),
        ("alice.stats", true),
        ("a-b.c2", true),
        ("stats", false),
        ("Org.Wolos", false),
        ("a..b", false),
        ("-a.b", false),
        ("a.b/c", false),
    ] {
        assert_eq!(is_plugin_id(id), ok, "{id}");
    }
    for (p, ok) in [
        ("cpu.lua", true),
        ("lua/cpu.lua", true),
        ("../cpu.lua", false),
        ("/etc/passwd", false),
        (".hidden", false),
        ("a b.lua", false),
        ("", false),
    ] {
        assert_eq!(is_plugin_path(p), ok, "{p}");
    }
    for (c, ok) in [
        ("super+shift+s", true),
        ("super+f5", true),
        ("f12", true),
        ("super+space", true),
        ("ctrl+alt+delete", true),
        ("s", false),
        ("space", false),
        ("super+shift+super+s", false),
        ("cmd+s", false),
        ("super+", false),
    ] {
        assert_eq!(is_combo(c), ok, "{c}");
    }
}

#[test]
fn hypr_fragments_are_whitelisted() {
    let ok = "# rules\nwindowrule = float, class:^(foot)$\nlayerrule = blur, wol-panel # x\nbezier = soft, 0.2, 0.8, 0.2, 1\nanimation = windows, 1, 4, soft\n";
    let pairs = check_hypr(ok).unwrap();
    assert_eq!(pairs.len(), 4);
    assert_eq!(pairs[1], ("layerrule".into(), "blur, wol-panel".into()));
    let fine = "windowrule = opacity 0.9 0.8 override, class:^(foot)$\nwindowrule = size 800 600, title:^(Notes)$\n";
    assert_eq!(check_hypr(fine).map(|p| p.len()), Ok(2));
    for (bad, want) in [
        ("bind = SUPER, T, exec, foot\n", "\"bind\""),
        ("exec-once = evil\n", "\"exec-once\""),
        ("source = ~/x.conf\n", "\"source\""),
        ("env = X, 1\n", "\"env\""),
        ("plugin = x\n", "\"plugin\""),
        ("general {\n", "not a keyword = value"),
        ("windowrule = $evil\n", "$variables"),
        ("windowrule = float, class:^($mod)\n", "$variables"),
        // Audit L7: rules that take windows away from the person.
        (
            "windowrule = opacity 0.0, class:^(foot)$\n",
            "almost invisible",
        ),
        (
            "windowrule = opacity 0.9 0.1, class:^(foot)$\n",
            "almost invisible",
        ),
        ("windowrule = nofocus, class:^(foot)$\n", "\"nofocus\""),
        (
            "windowrule = fullscreen, class:^(foot)$\n",
            "\"fullscreen\"",
        ),
        ("windowrule = pin, class:^(foot)$\n", "\"pin\""),
        (
            "windowrule = stayfocused, class:^(foot)$\n",
            "\"stayfocused\"",
        ),
        ("layerrule = xray, wol-panel\n", "\"xray\""),
        ("layerrule = noanim, wol-panel\n", "\"noanim\""),
        ("windowrule = float, class:.*\n", "every window"),
        ("windowrule = float, title:^(.+)$\n", "every window"),
        (
            "windowrule = float, class:^(foot)$, title:.*\n",
            "every window",
        ),
        ("windowrule = float\n", "every window"),
        ("layerrule = blur, .*\n", "every window"),
    ] {
        let e = check_hypr(bad).unwrap_err();
        assert!(e.contains(want), "{bad}: {e}");
    }
}

/// The published schema and the parser describe the same keys and choices.
#[test]
fn schema_json_matches_the_parser() {
    let s: serde_json::Value = serde_json::from_str(SCHEMA_JSON).unwrap();
    let props = s["properties"].as_object().unwrap();
    let mut keys: Vec<&str> = props.keys().map(String::as_str).collect();
    keys.sort();
    let mut want: Vec<&str> = HEADER_KEYS.to_vec();
    want.extend(Kind::ALL.map(Kind::id));
    want.sort();
    assert_eq!(keys, want);
    assert_eq!(s["additionalProperties"], false);
    let required: Vec<&str> = s["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(required, ["schema", "id", "name", "version", "license"]);
    let perms: Vec<&str> = props["permissions"]["items"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(perms, Permission::ALL.map(Permission::id));
    for k in Kind::ALL {
        let item = &props[k.id()]["items"];
        let mut got: Vec<&str> = item["properties"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        got.sort();
        let mut want = k.keys().to_vec();
        want.sort();
        assert_eq!(got, want, "[[{}]]", k.id());
        assert_eq!(item["additionalProperties"], false, "[[{}]]", k.id());
    }
    let choices = |table: &str, key: &str| -> Vec<String> {
        props[table]["items"]["properties"][key]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(
        choices("menubar", "side"),
        Side::ALL.map(|x| x.id().to_owned())
    );
    assert_eq!(
        choices("panel", "anchor"),
        Anchor::ALL.map(|x| x.id().to_owned())
    );
    assert_eq!(
        choices("search", "kind"),
        SearchKind::ALL.map(|x| x.id().to_owned())
    );
    assert_eq!(
        choices("webapp", "profile"),
        Profile::ALL.map(|x| x.id().to_owned())
    );
    let sizes: Vec<&str> = props["widget"]["items"]["properties"]["sizes"]["items"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(sizes, Size::ALL.map(Size::id));
}

// ---- `[[ipc]]` (SP-6): parse, round trip, argument types and the schema's choices. ----

const HEAD: &str =
    "schema = 1\nid = \"alice.music\"\nname = \"Music\"\nversion = \"1.0\"\nlicense = \"MIT\"\n";

#[test]
fn an_ipc_entry_names_a_lua_function_and_its_argument() {
    let m = parse(&format!(
        "{HEAD}\n[[ipc]]\nid = \"next\"\nscript = \"ipc.lua\"\n\n[[ipc]]\nid = \"volume\"\nscript = \"ipc.lua\"\nargs = \"number\"\n"
    ))
    .unwrap();
    assert_eq!(m.kinds(), [Kind::Ipc]);
    assert_eq!(
        m.ipc,
        [
            IpcEntry {
                id: "next".into(),
                script: "ipc.lua".into(),
                args: ArgType::Any
            },
            IpcEntry {
                id: "volume".into(),
                script: "ipc.lua".into(),
                args: ArgType::Number
            }
        ]
    );
    assert_eq!(m.scripts().len(), 2);
    assert_eq!(m.files(), ["ipc.lua"], "one file, named once");
    assert_eq!(parse(&m.to_toml()).unwrap(), m, "{}", m.to_toml());
    let json = serde_json::to_string(&m).unwrap();
    assert_eq!(serde_json::from_str::<Manifest>(&json).unwrap(), m);
}

#[test]
fn a_bad_ipc_entry_is_one_sentence() {
    let bad = |body: &str| parse(&format!("{HEAD}\n[[ipc]]\n{body}")).unwrap_err();
    assert_eq!(bad("id = \"next\"\n"), "[[ipc]] 1 needs script.");
    assert_eq!(
        bad("id = \"next\"\nscript = \"ipc.txt\"\n"),
        "[[ipc]] 1: script is a .lua file inside the plugin folder, like \"item.lua\"."
    );
    assert_eq!(
        bad("id = \"next\"\nscript = \"ipc.lua\"\nargs = \"date\"\n"),
        "[[ipc]] 1: args is one of any, none, text, number, bool, list, object."
    );
    assert_eq!(
        bad("id = \"Next\"\nscript = \"ipc.lua\"\n"),
        "[[ipc]] 1: id is 1 to 32 lowercase letters, digits, - or _."
    );
}

#[test]
fn argument_types_check_the_json_sent() {
    use serde_json::json;
    let all = [
        json!(null),
        json!("a"),
        json!(1.5),
        json!(true),
        json!([1]),
        json!({"a": 1}),
    ];
    let fits = |t: ArgType| -> Vec<bool> { all.iter().map(|v| t.accepts(v)).collect() };
    assert_eq!(fits(ArgType::Any), [true; 6]);
    assert_eq!(
        fits(ArgType::None),
        [true, false, false, false, false, false]
    );
    assert_eq!(
        fits(ArgType::Text),
        [false, true, false, false, false, false]
    );
    assert_eq!(
        fits(ArgType::Number),
        [false, false, true, false, false, false]
    );
    assert_eq!(
        fits(ArgType::Bool),
        [false, false, false, true, false, false]
    );
    assert_eq!(
        fits(ArgType::List),
        [false, false, false, false, true, false]
    );
    assert_eq!(
        fits(ArgType::Object),
        [false, false, false, false, false, true]
    );
    assert_eq!(
        ArgType::None.sentence("alice.music", "next"),
        "alice.music next takes no arguments."
    );
    assert_eq!(
        ArgType::Number.sentence("alice.music", "volume"),
        "alice.music volume takes one JSON number."
    );
}

#[test]
fn the_schema_lists_the_argument_types() {
    let s: serde_json::Value = serde_json::from_str(SCHEMA_JSON).unwrap();
    let got: Vec<&str> = s["properties"]["ipc"]["items"]["properties"]["args"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(got, ArgType::ALL.map(ArgType::id));
}

const LISTED: &str = r#"
schema = 1
id = "alice.stats"
name = "Stats"
version = "1.0"
license = "MIT"
description = "CPU in the bar."
min_wolos = "0.13"
icon = "icon.png"
screenshots = ["shots/one.png", "shots/two.webp"]
categories = ["Develop", "Work", "Develop"]
homepage = "https://example.org/stats"
about = "First line.\nSecond line with \"quotes\"."

[[menubar]]
id = "cpu"
script = "cpu.lua"
"#;

/// The hub's listing keys (hubcheck `facts`): read, kept, written back, and named in `files()`.
#[test]
fn listing_keys_are_read_and_round_trip() {
    let m = parse(LISTED).unwrap();
    assert_eq!(m.icon, "icon.png");
    assert_eq!(m.screenshots, ["shots/one.png", "shots/two.webp"]);
    assert_eq!(m.categories, ["Develop", "Work"], "twice is once");
    assert_eq!(m.homepage, "https://example.org/stats");
    assert_eq!(m.about, "First line.\nSecond line with \"quotes\".");
    assert_eq!(
        m.files(),
        ["cpu.lua", "icon.png", "shots/one.png", "shots/two.webp"]
    );
    assert_eq!(parse(&m.to_toml()).unwrap(), m, "{}", m.to_toml());
    // the raw document (validators, Plugin Maker) carries the line break too
    let j = to_json(LISTED).unwrap();
    assert_eq!(j["about"], "First line.\nSecond line with \"quotes\".");
    // a plugin without them has none of them
    let m = parse(STATS).unwrap();
    assert!(m.icon.is_empty() && m.screenshots.is_empty() && m.categories.is_empty());
    assert!(m.homepage.is_empty() && m.about.is_empty());
    assert!(!m.to_toml().contains("about"));
}

/// These keys are new in 0.13: a 0.12 reader refuses them, so the manifest must say 0.13.
#[test]
fn listing_keys_need_min_wolos_013() {
    for key in LISTING_KEYS {
        let line = match key {
            "icon" => "icon = \"icon.png\"",
            "screenshots" => "screenshots = [\"a.png\"]",
            "categories" => "categories = [\"Work\"]",
            "homepage" => "homepage = \"https://example.org\"",
            _ => "about = \"text\"",
        };
        let base = |min: &str| {
            format!(
                "schema = 1\nid = \"alice.x\"\nname = \"X\"\nversion = \"1\"\nlicense = \"MIT\"\n{min}{line}\n[[menubar]]\nid = \"m\"\nscript = \"m.lua\"\n"
            )
        };
        for (min, ok) in [
            ("", false),
            ("min_wolos = \"0.12\"\n", false),
            ("min_wolos = \"0.12.9\"\n", false),
            ("min_wolos = \"0.13\"\n", true),
            ("min_wolos = \"0.13.2\"\n", true),
            ("min_wolos = \"0.14\"\n", true),
            ("min_wolos = \"1.0\"\n", true),
        ] {
            let r = parse(&base(min));
            assert_eq!(r.is_ok(), ok, "{key} with [{min}]: {r:?}");
            if !ok {
                let e = r.unwrap_err();
                assert!(e.contains(key) && e.contains("min_wolos = \"0.13\""), "{e}");
            }
        }
    }
}

#[test]
fn listing_values_are_checked() {
    let with = |line: &str| {
        parse(&format!(
            "schema = 1\nid = \"alice.x\"\nname = \"X\"\nversion = \"1\"\nlicense = \"MIT\"\nmin_wolos = \"0.13\"\n{line}\n[[menubar]]\nid = \"m\"\nscript = \"m.lua\"\n"
        ))
    };
    for (bad, word) in [
        ("icon = \"icon.svg\"", "icon"),
        ("icon = \"icon.gif\"", "icon"),
        ("icon = \"../icon.png\"", "icon"),
        ("icon = \"/etc/x.png\"", "icon"),
        ("icon = \".hidden.png\"", "icon"),
        ("icon = 3", "icon"),
        ("screenshots = [\"a.png\", \"a.png\"]", "twice"),
        ("screenshots = [\"a.jpg\"]", "screenshots"),
        ("screenshots = [\"a/../b.png\"]", "screenshots"),
        ("screenshots = \"a.png\"", "screenshots"),
        (
            "screenshots = [\"1.png\",\"2.png\",\"3.png\",\"4.png\",\"5.png\",\"6.png\",\"7.png\",\"8.png\",\"9.png\"]",
            "at most 8",
        ),
        ("categories = [\"Games\"]", "categories"),
        ("categories = \"Work\"", "categories"),
        ("homepage = \"http://example.org\"", "homepage"),
        ("homepage = \"https://user@example.org\"", "homepage"),
        ("homepage = \"https://\"", "homepage"),
        ("homepage = \"https://a b.org\"", "homepage"),
        ("homepage = \"javascript:alert(1)\"", "homepage"),
        ("about = 3", "about"),
    ] {
        let e = with(bad).unwrap_err();
        assert!(e.contains(word), "{bad}: {e}");
    }
    let long = format!("about = \"{}\"", "x".repeat(MAX_ABOUT + 1));
    assert!(with(&long).unwrap_err().contains("about"));
    let max = format!("about = \"{}\"", "x".repeat(MAX_ABOUT));
    assert!(with(&max).is_ok());
    let eight = "screenshots = [\"1.png\",\"2.png\",\"3.png\",\"4.png\",\"5.png\",\"6.png\",\"7.png\",\"8.webp\"]";
    assert_eq!(with(eight).unwrap().screenshots.len(), 8);
    // only `about` may hold a line break: it never reaches a name, a path or a command
    for line in [
        "description = \"a\\nb\"",
        "author = \"a\\nb\"",
        "homepage = \"https://a.org/\\n\"",
        "icon = \"a\\n.png\"",
    ] {
        assert!(with(line).is_err(), "{line}");
    }
    assert!(
        with("about = \"a\\u0007b\"").is_err(),
        "other control characters stay refused"
    );
    assert!(with("about = \"a\\rb\"").is_err());
    // the same line break in a section's text is still refused
    let e = parse(&format!(
        "{}\n[[dock]]\nid = \"d\"\nname = \"a\\nb\"\nicon = \"x-symbolic\"\nexec = [\"foot\"]\n",
        LISTED.split("[[menubar]]").next().unwrap()
    ))
    .unwrap_err();
    assert!(e.contains("control") || e.contains("name"), "{e}");
}

#[test]
fn the_wolos_version_comparison() {
    for (v, ok) in [
        ("0.13", true),
        ("0.13.0", true),
        ("0.12.99", false),
        ("0.9", false),
        ("1.0", true),
        ("0.100", true),
        ("", false),
        ("0", false),
        ("x.y", false),
    ] {
        assert_eq!(wolos_at_least(v, LISTING_MIN_WOLOS), ok, "{v}");
    }
}

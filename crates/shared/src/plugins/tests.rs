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

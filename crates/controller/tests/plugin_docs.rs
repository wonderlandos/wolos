// SPDX-License-Identifier: GPL-2.0-or-later
//! docs/modules/plugins.md is executable: every ```toml block parses as a manifest and matches the
//! published JSON Schema; every ```lua block loads as a plugin script with all permissions and
//! renders (or answers a query) against a host that answers everything.
use serde_json::Value;
use shared::plugins::{Kind, Permission, SCHEMA_JSON};
use std::path::Path;
use std::rc::Rc;
use wol_lua::plugin::{Entry, Script};
use wol_lua::{Services, Stamp};

fn docs() -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/modules/plugins.md");
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// Fenced blocks of one language, with the line they start on.
fn blocks(md: &str, lang: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut cur: Option<(usize, String)> = None;
    let open = format!("```{lang}");
    for (i, line) in md.lines().enumerate() {
        if let Some((at, body)) = cur.as_mut() {
            if line.trim_end() == "```" {
                out.push((*at, std::mem::take(body)));
                cur = None;
            } else {
                body.push_str(line);
                body.push('\n');
            }
        } else if line.trim_end() == open {
            cur = Some((i + 1, String::new()));
        }
    }
    out
}

/// The part of JSON Schema this schema uses: $ref into $defs, type, const, enum, required,
/// properties + additionalProperties, items, minLength, maxLength, minItems, uniqueItems.
/// `pattern` is not checked (no regex crate); the parser's own checks cover those rules.
fn validate(root: &Value, schema: &Value, v: &Value, path: &str, errs: &mut Vec<String>) {
    let mut schema = schema;
    if let Some(r) = schema["$ref"].as_str() {
        let name = r
            .strip_prefix("#/$defs/")
            .unwrap_or_else(|| panic!("ref {r}"));
        schema = &root["$defs"][name];
    }
    let bad = |errs: &mut Vec<String>, what: &str| errs.push(format!("{path}: {what}"));
    if let Some(c) = schema.get("const")
        && c != v
    {
        bad(errs, &format!("must be {c}"));
    }
    if let Some(t) = schema["type"].as_str() {
        let ok = match t {
            "object" => v.is_object(),
            "array" => v.is_array(),
            "string" => v.is_string(),
            "integer" => v.is_i64(),
            "boolean" => v.is_boolean(),
            other => panic!("type {other}"),
        };
        if !ok {
            bad(errs, &format!("is not {t}"));
        }
    }
    if let Some(e) = schema["enum"].as_array()
        && !e.contains(v)
    {
        bad(errs, &format!("{v} is not one of {e:?}"));
    }
    if let Some(s) = v.as_str() {
        if let Some(n) = schema["minLength"].as_u64()
            && (s.chars().count() as u64) < n
        {
            bad(errs, "too short");
        }
        if let Some(n) = schema["maxLength"].as_u64()
            && (s.len() as u64) > n
        {
            bad(errs, "too long");
        }
    }
    if let Some(a) = v.as_array() {
        if let Some(n) = schema["minItems"].as_u64()
            && (a.len() as u64) < n
        {
            bad(errs, "too few items");
        }
        if schema["uniqueItems"] == true {
            let mut seen: Vec<&Value> = Vec::new();
            for x in a {
                if seen.contains(&x) {
                    bad(errs, &format!("{x} repeated"));
                }
                seen.push(x);
            }
        }
        if let Some(items) = schema.get("items") {
            for (i, x) in a.iter().enumerate() {
                validate(root, items, x, &format!("{path}[{i}]"), errs);
            }
        }
    }
    if let Some(o) = v.as_object() {
        for r in schema["required"].as_array().into_iter().flatten() {
            if !o.contains_key(r.as_str().unwrap()) {
                bad(errs, &format!("missing {r}"));
            }
        }
        let props = schema["properties"].as_object();
        for (k, x) in o {
            match props.and_then(|p| p.get(k)) {
                Some(s) => validate(root, s, x, &format!("{path}.{k}"), errs),
                None if schema["additionalProperties"] == false => {
                    bad(errs, &format!("unknown key {k}"))
                }
                None => {}
            }
        }
    }
}

#[test]
fn every_toml_example_parses_and_matches_the_schema() {
    let md = docs();
    let examples = blocks(&md, "toml");
    assert!(examples.len() >= 3, "{}", examples.len());
    let schema: Value = serde_json::from_str(SCHEMA_JSON).unwrap();
    for (line, text) in &examples {
        let m = shared::plugins::parse(text)
            .unwrap_or_else(|e| panic!("plugins.md line {line}: {e}\n{text}"));
        assert!(!m.entries().is_empty());
        let raw = shared::plugins::to_json(text).unwrap();
        let mut errs = Vec::new();
        validate(&schema, &schema, &raw, "plugin.toml", &mut errs);
        assert!(errs.is_empty(), "plugins.md line {line}: {errs:?}");
        // Round trip through the canonical writer.
        assert_eq!(
            shared::plugins::parse(&m.to_toml()).unwrap(),
            m,
            "line {line}"
        );
    }
    // The schema also refuses what the parser refuses (one example each).
    let bad = shared::plugins::to_json("schema = 1\nid = \"a.b\"\nname = \"X\"\nversion = \"1\"\nlicense = \"MIT\"\ncolour = \"red\"\n[[menubar]]\nid = \"a\"\nscript = \"a.lua\"\nside = \"middle\"\n").unwrap();
    let mut errs = Vec::new();
    validate(&schema, &schema, &bad, "plugin.toml", &mut errs);
    assert!(
        errs.iter().any(|e| e.contains("unknown key colour")),
        "{errs:?}"
    );
    assert!(errs.iter().any(|e| e.contains("middle")), "{errs:?}");
}

/// A host that says yes to everything, so every documented call runs.
struct Yes;

impl Services for Yes {
    fn now(&self) -> Stamp {
        Stamp {
            year: 2026,
            month: 11,
            day: 1,
            weekday: 7,
            hour: 9,
            minute: 0,
            second: 0,
        }
    }
    fn system(&self) -> wol_lua::SystemInfo {
        wol_lua::SystemInfo {
            os_name: "WolOS".into(),
            os_version: "0.12".into(),
            kernel: "WolKernel".into(),
            appearance: wol_lua::Appearance::Dark,
            accent_name: "blue".into(),
            accent_rgb: 0x6aaef2,
        }
    }
    fn notify(&self, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn run(&self, _: &str, argv: &[String], _: u32) -> Result<String, String> {
        Ok(if argv.get(1).is_some_and(|a| a == "sys") {
            r#"{"cpu_percent": 12.5, "memory": {"total": 16000000000, "used": 4000000000}, "uptime_secs": 3700}"#.into()
        } else {
            String::new()
        })
    }
    fn http_get(&self, _: &str, _: &str) -> Result<String, String> {
        Ok(r#"{"text": "Hello"}"#.into())
    }
    fn open(&self, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn toggle_set(&self, _: &str, _: &str, _: bool) -> Result<(), String> {
        Ok(())
    }
    fn clipboard_text(&self, _: &str) -> Result<String, String> {
        Ok("clip".into())
    }
    fn show(&self, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn state_save(&self, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn every_lua_example_loads_and_renders() {
    let md = docs();
    let examples = blocks(&md, "lua");
    assert!(examples.len() >= 4, "{}", examples.len());
    for (line, src) in &examples {
        let kind = ["menubar", "widget", "panel", "search"]
            .iter()
            .find(|k| src.contains(&format!("return wol.{k} {{")))
            .and_then(|k| Kind::parse(k))
            .unwrap_or_else(|| panic!("plugins.md line {line}: no return wol.<kind>"));
        let at = Entry {
            kind,
            plugin: "alice.docs",
            id: "x",
            dir: Path::new("/plug/alice.docs"),
        };
        let mut s = Script::load(at, src, &Permission::ALL, Rc::new(Yes))
            .unwrap_or_else(|e| panic!("plugins.md line {line}: {e}"));
        let r = if kind == Kind::Search {
            s.query("2+2")
                .map(|h| assert!(!h.is_empty(), "line {line}"))
        } else {
            s.render().and_then(|n| {
                n.validate()?;
                s.popover().map(|_| ())
            })
        };
        r.unwrap_or_else(|e| panic!("plugins.md line {line}: {e}"));
    }
    // The reference plugin's scripts are the documentation's long form: they load and render too.
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/shell/org.wolos.stats");
    for (file, kind) in [("bar.lua", Kind::Menubar), ("card.lua", Kind::Widget)] {
        let src = std::fs::read_to_string(dir.join(file)).unwrap();
        let at = Entry {
            kind,
            plugin: "org.wolos.stats",
            id: "x",
            dir: &dir,
        };
        let mut s = Script::load(at, &src, &[Permission::Run], Rc::new(Yes))
            .unwrap_or_else(|e| panic!("{file}: {e}"));
        let n = s.render().unwrap_or_else(|e| panic!("{file}: {e}"));
        let text = serde_json::to_string(&n).unwrap();
        assert!(
            text.contains("CPU 13%") || text.contains("13%"),
            "{file}: {text}"
        );
        if kind == Kind::Menubar {
            let p = serde_json::to_string(&s.popover().unwrap()).unwrap();
            assert!(
                p.contains("Up 1 h 01 min") && p.contains("3.7 GiB of 14.9 GiB"),
                "{p}"
            );
        }
    }
}

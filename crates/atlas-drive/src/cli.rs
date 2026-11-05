// SPDX-License-Identifier: GPL-2.0-or-later
//! Command line to params, and result to text. Pure, so it is tested without a daemon.
use crate::spec::Verb;
use serde_json::{Value, json};

const GENERIC: &[(&str, &str)] = &[
    ("budget", "integer"),
    ("tab", "integer"),
    ("timeout", "integer"),
    ("then", "string"),
    ("then_budget", "integer"),
    ("nav_timeout", "integer"),
];

fn norm(k: &str) -> String {
    k.replace('-', "_")
}

fn convert(ty: &str, v: &str) -> Result<Value, String> {
    match ty {
        "integer" => v
            .parse::<i64>()
            .map(Value::from)
            .map_err(|_| format!("{v} is not an integer")),
        "number" => v
            .parse::<f64>()
            .map(Value::from)
            .map_err(|_| format!("{v} is not a number")),
        "boolean" => Ok(Value::Bool(v != "false")),
        "object" => serde_json::from_str(v).map_err(|e| format!("not JSON: {e}")),
        _ => Ok(Value::String(v.to_string())),
    }
}

/// `click e12 --dom`, `fill e3 "a@b.c" --submit`, `emulate --width 390 --scheme dark`, `upload e5 --files a b`.
pub fn parse(verb: &Verb, args: &[String]) -> Result<Value, String> {
    // `js search <regex>`, `js source <script>`, `js break <event>`: the second word names what the op takes
    if verb.name == "tab"
        && args.len() >= 2
        && matches!(args[0].as_str(), "select" | "close")
        && !args[1].starts_with("--")
    {
        let mut a = vec![args[0].clone(), "--index".to_string(), args[1].clone()];
        a.extend(args[2..].iter().cloned());
        return parse(verb, &a);
    }
    if verb.name == "js" && args.len() >= 2 && !args[1].starts_with("--") {
        let flag = match args[0].as_str() {
            "search" => "--query",
            "source" => "--script",
            "break" => "--event",
            _ => "",
        };
        if !flag.is_empty() {
            let mut a = vec![args[0].clone(), flag.to_string(), args[1].clone()];
            a.extend(args[2..].iter().cloned());
            return parse(verb, &a);
        }
    }
    let mut out = serde_json::Map::new();
    let ty_of = |k: &str| -> Option<&'static str> {
        verb.args
            .iter()
            .find(|a| a.name == k)
            .map(|a| a.ty)
            .or_else(|| GENERIC.iter().find(|(n, _)| *n == k).map(|(_, t)| *t))
    };
    let mut positional: Vec<&String> = vec![];
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if let Some(flag) = a.strip_prefix("--") {
            let (k, inline) = match flag.split_once('=') {
                Some((k, v)) => (norm(k), Some(v.to_string())),
                None => (norm(flag), None),
            };
            // `--for` is the wait condition, written `wait load` too
            let ty = ty_of(&k).ok_or_else(|| {
                format!(
                    "{} has no option --{}; see --help-agent",
                    verb.name,
                    k.replace('_', "-")
                )
            })?;
            match ty {
                "boolean" => {
                    let v = match inline {
                        Some(v) => v != "false",
                        None if args.get(i + 1).is_some_and(|n| n == "true" || n == "false") => {
                            i += 1;
                            args[i] == "true"
                        }
                        None => true,
                    };
                    out.insert(k, Value::Bool(v));
                }
                "array" => {
                    let mut items: Vec<Value> = vec![];
                    if let Some(v) = inline {
                        items.extend(v.split(',').map(|x| json!(x)));
                    }
                    while args.get(i + 1).is_some_and(|n| !n.starts_with("--")) {
                        i += 1;
                        items.push(convert_array_item(&args[i]));
                    }
                    if items.is_empty() {
                        return Err(format!("--{} needs a value", k.replace('_', "-")));
                    }
                    out.insert(k, Value::Array(items));
                }
                _ => {
                    let v = match inline {
                        Some(v) => v,
                        None => {
                            i += 1;
                            args.get(i)
                                .cloned()
                                .ok_or_else(|| format!("--{} needs a value", k.replace('_', "-")))?
                        }
                    };
                    out.insert(k.clone(), convert(ty, &v)?);
                }
            }
        } else {
            positional.push(a);
        }
        i += 1;
    }
    let mut pos = positional.into_iter();
    // positionals fill the required arguments, then the optional ones that are natural to write bare
    // (`wait selector '#x'`, `network body 12`); the others need their --flag
    const BARE: &[&str] = &[
        "value", "url", "key", "id", "text", "query", "script", "name", "op", "index", "pattern",
        "action", "out", "dir", "target", "event",
    ];
    for spec in verb.args.iter() {
        if out.contains_key(spec.name)
            || spec.ty == "boolean"
            || !(spec.required || BARE.contains(&spec.name))
        {
            continue;
        }
        if spec.ty == "array" {
            let mut rest: Vec<Value> = pos.by_ref().map(|s| convert_array_item(s)).collect();
            // one positional that is itself a JSON array is the array (`batch '[{...}]'`)
            if rest.len() == 1
                && let Some(Ok(Value::Array(a))) = rest[0]
                    .as_str()
                    .filter(|t| t.trim_start().starts_with('['))
                    .map(serde_json::from_str::<Value>)
            {
                rest = a;
            }
            if !rest.is_empty() {
                out.insert(spec.name.into(), Value::Array(rest));
            }
            break;
        }
        match pos.next() {
            Some(v) => {
                out.insert(spec.name.into(), convert(spec.ty, v)?);
            }
            None => break,
        }
    }
    if let Some(extra) = pos.next() {
        return Err(format!("unexpected argument {extra}; see --help-agent"));
    }
    for spec in verb.args.iter().filter(|a| a.required) {
        if !out.contains_key(spec.name) {
            return Err(format!("{} needs {}", verb.name, spec.name));
        }
    }
    Ok(Value::Object(out))
}

fn convert_array_item(s: &str) -> Value {
    s.parse::<f64>()
        .map(Value::from)
        .unwrap_or_else(|_| Value::String(s.to_string()))
}

/// The text a person or model reads: the `text` (or `view`) field raw, the other fields as one JSON line, then tokens.
pub fn render(result: &Value) -> String {
    let mut out = String::new();
    let mut rest = serde_json::Map::new();
    let mut texts: Vec<String> = vec![];
    let tokens = result.get("tokens").and_then(Value::as_u64);
    if let Some(o) = result.as_object() {
        for (k, v) in o {
            match (k.as_str(), v) {
                ("tokens", _) => {}
                ("text", Value::String(t)) | ("view", Value::String(t)) => texts.push(t.clone()),
                _ => {
                    rest.insert(k.clone(), v.clone());
                }
            }
        }
    }
    if !rest.is_empty() {
        out.push_str(&Value::Object(rest).to_string());
        out.push('\n');
    }
    for t in texts {
        out.push_str(&t);
        if !t.ends_with('\n') {
            out.push('\n');
        }
    }
    if let Some(t) = tokens {
        out.push_str(&format!("tokens≈{t}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::find;

    fn a(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn positionals_follow_the_spec_order() {
        let v = parse(find("fill").unwrap(), &a(&["e3", "a@b.c", "--submit"])).unwrap();
        assert_eq!(v, json!({"target": "e3", "text": "a@b.c", "submit": true}));
        let v = parse(
            find("click").unwrap(),
            &a(&["role=button name~=Save", "--dom", "--count", "2"]),
        )
        .unwrap();
        assert_eq!(v["count"], 2);
        assert_eq!(v["dom"], true);
        let v = parse(
            find("wait").unwrap(),
            &a(&["selector", "#x", "--timeout", "500"]),
        )
        .unwrap();
        assert_eq!(v, json!({"for": "selector", "value": "#x", "timeout": 500}));
    }

    #[test]
    fn arrays_numbers_and_inline_values() {
        let v = parse(find("upload").unwrap(), &a(&["e5", "--files", "/a", "/b"])).unwrap();
        assert_eq!(v["files"], json!(["/a", "/b"]));
        let v = parse(
            find("emulate").unwrap(),
            &a(&[
                "--width=390",
                "--scheme",
                "dark",
                "--geolocation",
                "52.5",
                "13.4",
            ]),
        )
        .unwrap();
        assert_eq!(
            v,
            json!({"width": 390, "scheme": "dark", "geolocation": [52.5, 13.4]})
        );
        let v = parse(
            find("cdp_events").unwrap(),
            &a(&["Network.*", "Page.load", "--ms", "300"]),
        )
        .unwrap();
        assert_eq!(v["events"], json!(["Network.*", "Page.load"]));
        assert_eq!(
            parse(
                find("snapshot").unwrap(),
                &a(&["--max-tokens", "200", "--diff"])
            )
            .unwrap(),
            json!({"max_tokens": 200, "diff": true})
        );
    }

    #[test]
    fn mistakes_are_one_line() {
        assert!(
            parse(find("click").unwrap(), &a(&[]))
                .unwrap_err()
                .contains("needs target")
        );
        assert!(
            parse(find("click").unwrap(), &a(&["e1", "e2"]))
                .unwrap_err()
                .contains("unexpected argument e2")
        );
        assert!(
            parse(find("click").unwrap(), &a(&["e1", "--nope"]))
                .unwrap_err()
                .contains("no option --nope")
        );
        assert!(
            parse(find("goto").unwrap(), &a(&["x", "--timeout", "soon"]))
                .unwrap_err()
                .contains("not an integer")
        );
    }

    #[test]
    fn tab_ops_take_an_index() {
        assert_eq!(
            parse(find("tab").unwrap(), &a(&["select", "2"])).unwrap(),
            json!({"op": "select", "index": 2})
        );
        assert_eq!(
            parse(find("tab").unwrap(), &a(&["new", "https://a.test"])).unwrap(),
            json!({"op": "new", "url": "https://a.test"})
        );
    }

    #[test]
    fn batch_takes_a_json_array() {
        let v = parse(
            find("batch").unwrap(),
            &a(&["[{\"verb\":\"tabs\"},{\"verb\":\"status\"}]", "--parallel"]),
        )
        .unwrap();
        assert_eq!(v["steps"].as_array().unwrap().len(), 2);
        assert_eq!(v["parallel"], true);
    }

    #[test]
    fn js_ops_take_their_word() {
        let v = parse(
            find("js").unwrap(),
            &a(&["search", "confirm\\(", "--limit", "5"]),
        )
        .unwrap();
        assert_eq!(
            v,
            json!({"op": "search", "query": "confirm\\(", "limit": 5})
        );
        let v = parse(find("js").unwrap(), &a(&["source", "app.js", "--pretty"])).unwrap();
        assert_eq!(v["script"], "app.js");
    }

    #[test]
    fn results_print_text_then_tokens() {
        let r = json!({"text": "@ u \"t\"\n [e1] button \"Go\"\n", "nodes": 1, "tokens": 12});
        let s = render(&r);
        assert_eq!(
            s,
            "{\"nodes\":1}\n@ u \"t\"\n [e1] button \"Go\"\ntokens≈12\n"
        );
        assert_eq!(
            render(&json!({"ok": true, "tokens": 3})),
            "{\"ok\":true}\ntokens≈3\n"
        );
    }
}

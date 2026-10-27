//! `{{path}}` bindings in widget layouts. A path walks the data JSON: `user.name`, `friends[0].name`,
//! `friends.0.name`; `friends.length` counts an array. Inside a list item, `item` is the current element.
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
enum Part {
    Lit(String),
    Path(Vec<String>),
}

/// Parsed text with bindings.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Template {
    parts: Vec<Part>,
}

/// Lookup scope: the data root plus the current list item, if any.
#[derive(Clone, Copy, Debug)]
pub struct Scope<'a> {
    pub root: &'a Value,
    pub item: Option<&'a Value>,
}

impl<'a> Scope<'a> {
    pub fn root(root: &'a Value) -> Self {
        Scope { root, item: None }
    }
    pub fn with_item(self, item: &'a Value) -> Self {
        Scope {
            root: self.root,
            item: Some(item),
        }
    }
}

impl Template {
    pub fn parse(text: &str) -> Result<Template, String> {
        let mut parts = Vec::new();
        let mut rest = text;
        while let Some(open) = rest.find("{{") {
            if open > 0 {
                parts.push(Part::Lit(rest[..open].to_string()));
            }
            let after = &rest[open + 2..];
            let close = after
                .find("}}")
                .ok_or_else(|| format!("\"{{{{\" without a closing \"}}}}\" in {text:?}"))?;
            parts.push(Part::Path(parse_path(after[..close].trim())?));
            rest = &after[close + 2..];
        }
        if rest.contains("}}") {
            return Err(format!("\"}}}}\" without an opening \"{{{{\" in {text:?}"));
        }
        if !rest.is_empty() {
            parts.push(Part::Lit(rest.to_string()));
        }
        Ok(Template { parts })
    }

    /// True when the text is exactly one binding (`"{{cpu}}"`): values keep their JSON type.
    pub fn is_single_binding(&self) -> bool {
        matches!(self.parts.as_slice(), [Part::Path(_)])
    }

    pub fn is_literal(&self) -> bool {
        self.parts.iter().all(|p| matches!(p, Part::Lit(_)))
    }

    /// Paths this template reads (for validation against sample data).
    pub fn paths(&self) -> Vec<String> {
        self.parts
            .iter()
            .filter_map(|p| match p {
                Part::Path(s) => Some(s.join(".")),
                Part::Lit(_) => None,
            })
            .collect()
    }

    /// Text with every binding replaced; missing values render as nothing.
    pub fn render(&self, scope: Scope) -> String {
        let mut s = String::new();
        for p in &self.parts {
            match p {
                Part::Lit(l) => s.push_str(l),
                Part::Path(path) => {
                    if let Some(v) = lookup(scope, path) {
                        s.push_str(&display(&v));
                    }
                }
            }
        }
        s
    }

    /// The bound JSON value (single binding), else the rendered text as a string.
    pub fn value(&self, scope: Scope) -> Option<Value> {
        match self.parts.as_slice() {
            [Part::Path(p)] => lookup(scope, p),
            _ => Some(Value::String(self.render(scope))),
        }
    }

    /// Numeric value: a JSON number, or text that parses as one.
    pub fn number(&self, scope: Scope) -> Option<f64> {
        match self.value(scope)? {
            Value::Number(n) => n.as_f64(),
            Value::String(s) => s.trim().trim_end_matches('%').trim().parse().ok(),
            _ => None,
        }
        .filter(|v: &f64| v.is_finite())
    }

    /// Bound array (lists, charts); None when the path is missing or not an array.
    pub fn array(&self, scope: Scope) -> Option<Vec<Value>> {
        match self.value(scope)? {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    /// True when every binding resolves in `scope` (validation against sample data).
    pub fn resolves(&self, scope: Scope) -> Vec<String> {
        self.parts
            .iter()
            .filter_map(|p| match p {
                Part::Path(path) if lookup(scope, path).is_none() => Some(path.join(".")),
                _ => None,
            })
            .collect()
    }
}

fn parse_path(src: &str) -> Result<Vec<String>, String> {
    if src.is_empty() {
        return Err("empty binding \"{{}}\"".into());
    }
    let mut segs = Vec::new();
    for dotted in src.split('.') {
        // `a[0][1]` -> a, 0, 1
        let (head, mut idx) = match dotted.find('[') {
            Some(i) => (&dotted[..i], &dotted[i..]),
            None => (dotted, ""),
        };
        if head.is_empty() && (segs.is_empty() || idx.is_empty()) {
            return Err(format!("bad binding path {src:?}"));
        }
        if !head.is_empty() {
            if head.contains([']', '{', '}', ' ']) {
                return Err(format!("bad binding path {src:?}"));
            }
            segs.push(head.to_string());
        }
        while !idx.is_empty() {
            let close = idx
                .find(']')
                .filter(|_| idx.starts_with('['))
                .ok_or_else(|| format!("bad binding path {src:?}"))?;
            let n = &idx[1..close];
            if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
                return Err(format!("bad index [{n}] in binding {src:?}"));
            }
            segs.push(n.to_string());
            idx = &idx[close + 1..];
        }
    }
    Ok(segs)
}

fn lookup(scope: Scope, path: &[String]) -> Option<Value> {
    let (mut v, rest) = match (path.first().map(String::as_str), scope.item) {
        (Some("item"), Some(item)) => (item, &path[1..]),
        _ => (scope.root, path),
    };
    for (i, seg) in rest.iter().enumerate() {
        v = match v {
            Value::Object(m) => m.get(seg)?,
            Value::Array(a) => match seg.parse::<usize>() {
                Ok(n) => a.get(n)?,
                Err(_) if (seg == "length" || seg == "count") && i + 1 == rest.len() => {
                    return Some(Value::from(a.len()));
                }
                Err(_) => return None,
            },
            _ => return None,
        };
    }
    (!v.is_null()).then(|| v.clone())
}

/// Text form of a bound value: integers without a fraction, other numbers to at most 2 places.
pub fn display(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => i.to_string(),
            (None, Some(f)) => {
                let s = format!("{f:.2}");
                s.trim_end_matches('0').trim_end_matches('.').to_string()
            }
            _ => n.to_string(),
        },
        Value::Bool(b) => if *b { "Yes" } else { "No" }.into(),
        Value::Null | Value::Array(_) | Value::Object(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn data() -> Value {
        json!({
            "user": {"name": "Alice", "level": 12},
            "cpu": 23.456,
            "ratio": "40%",
            "online": true,
            "friends": [{"name": "Dinah", "game": "Chess"}, {"name": "Hatter"}],
            "tags": ["a", "b", "c"],
            "nothing": null
        })
    }

    #[test]
    fn renders_bindings_in_text() {
        let d = data();
        let s = Scope::root(&d);
        for (src, want) in [
            ("Hello", "Hello"),
            ("Hi {{user.name}}!", "Hi Alice!"),
            ("{{ user.name }}", "Alice"),
            ("L{{user.level}}", "L12"),
            ("{{cpu}}%", "23.46%"),
            (
                "{{friends[0].name}} plays {{friends.0.game}}",
                "Dinah plays Chess",
            ),
            ("{{friends.length}} online", "2 online"),
            ("{{tags.count}}", "3"),
            ("{{online}}", "Yes"),
            ("[{{missing.path}}]", "[]"),
            ("[{{nothing}}]", "[]"),
            ("[{{friends[5].name}}]", "[]"),
            ("[{{user}}]", "[]"),
        ] {
            assert_eq!(Template::parse(src).unwrap().render(s), want, "{src}");
        }
    }

    #[test]
    fn rejects_malformed_templates() {
        for src in [
            "{{", "a {{b", "a }} b", "{{}}", "{{ }}", "{{a..b}}", "{{a[x]}}", "{{a[]}}", "{{a b}}",
            "{{.a}}",
        ] {
            assert!(Template::parse(src).is_err(), "{src}");
        }
    }

    #[test]
    fn single_binding_keeps_json_type() {
        let d = data();
        let s = Scope::root(&d);
        let t = Template::parse("{{cpu}}").unwrap();
        assert!(t.is_single_binding());
        assert_eq!(t.number(s), Some(23.456));
        assert_eq!(Template::parse("{{ratio}}").unwrap().number(s), Some(40.0));
        assert_eq!(Template::parse("{{user.name}}").unwrap().number(s), None);
        assert_eq!(Template::parse("42").unwrap().number(s), Some(42.0));
        assert_eq!(
            Template::parse("{{friends}}")
                .unwrap()
                .array(s)
                .map(|a| a.len()),
            Some(2)
        );
        assert_eq!(Template::parse("{{user}}").unwrap().array(s), None);
        assert!(Template::parse("x {{cpu}}").unwrap().array(s).is_none());
    }

    #[test]
    fn list_items_bind_as_item() {
        let d = data();
        let f = d["friends"][1].clone();
        let s = Scope::root(&d).with_item(&f);
        assert_eq!(
            Template::parse("{{item.name}} / {{user.name}}")
                .unwrap()
                .render(s),
            "Hatter / Alice"
        );
        let tag = json!("b");
        assert_eq!(
            Template::parse("#{{item}}")
                .unwrap()
                .render(Scope::root(&d).with_item(&tag)),
            "#b"
        );
    }

    #[test]
    fn reports_unresolved_paths() {
        let d = data();
        let t = Template::parse("{{user.name}} {{user.email}} {{x[0]}}").unwrap();
        assert_eq!(t.resolves(Scope::root(&d)), vec!["user.email", "x.0"]);
        assert_eq!(t.paths(), vec!["user.name", "user.email", "x.0"]);
    }
}

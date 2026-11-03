// SPDX-License-Identifier: BSD-2-Clause
//! Recorded answers for development and tests (ADR 0048): the same fixture files the browser's
//! mock bridge (`webapps/kit/mock-bridge.js`) reads, so a page behaves the same in a desktop
//! browser, in the headless test and against this [`Wold`]. A fixture file is one call,
//! `wolkitFixtures({...});`, whose argument is strict JSON:
//! `{"app": {..wolos.app..}, "permissions": [..what app.toml declares..], "calls": {"<method>":
//! <result> | {"$error": {code, message, hint}}}}`; every key may be left out.
use super::{Fault, Wold};
use serde_json::{Map, Value};
use shared::errors::ErrorCode;

pub const CALL: &str = "wolkitFixtures(";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Fixtures {
    pub app: Map<String, Value>,
    pub calls: Map<String, Value>,
    /// The app's declared permissions, as its `app.toml` lists them (an app's own file only).
    pub permissions: Option<Vec<String>>,
}

impl Fixtures {
    /// Reads one fixture file's text.
    pub fn parse(text: &str) -> Result<Fixtures, String> {
        let start = text.find(CALL).ok_or("no wolkitFixtures( call")? + CALL.len();
        let end = text
            .rfind(");")
            .filter(|e| *e >= start)
            .ok_or("no closing );")?;
        let v: Value =
            serde_json::from_str(&text[start..end]).map_err(|e| format!("fixtures: {e}"))?;
        let Value::Object(mut top) = v else {
            return Err("fixtures: not an object".into());
        };
        let permissions = match top.remove("permissions") {
            None => None,
            Some(v) => Some(
                serde_json::from_value(v)
                    .map_err(|_| "fixtures: permissions is not a list of names")?,
            ),
        };
        let mut take = |k: &str| match top.remove(k) {
            None => Ok(Map::new()),
            Some(Value::Object(m)) => Ok(m),
            Some(_) => Err(format!("fixtures: {k} is not an object")),
        };
        let f = Fixtures {
            app: take("app")?,
            calls: take("calls")?,
            permissions,
        };
        match top.keys().next() {
            Some(k) => Err(format!("fixtures: unknown key {k:?}")),
            None => Ok(f),
        }
    }

    /// Later files win, key by key (an app's own fixtures over the kit's).
    pub fn merge(&mut self, other: Fixtures) {
        self.app.extend(other.app);
        self.calls.extend(other.calls);
        if other.permissions.is_some() {
            self.permissions = other.permissions;
        }
    }
}

fn code(name: &str) -> ErrorCode {
    ErrorCode::ALL
        .into_iter()
        .find(|c| c.name() == name && *c != ErrorCode::Ok)
        .unwrap_or(ErrorCode::Failed)
}

impl Wold for Fixtures {
    fn call(&self, _app: &str, method: &str, _params: &Map<String, Value>) -> Result<Value, Fault> {
        match self.calls.get(method) {
            None => Err(Fault::new(
                ErrorCode::Unavailable,
                "No recorded answer for this.",
                format!("add {method:?} to a fixtures file"),
            )),
            Some(Value::Object(o)) if o.contains_key("$error") => {
                let e = &o["$error"];
                let s = |k: &str| e[k].as_str().unwrap_or_default().to_owned();
                Err(Fault::new(code(&s("code")), s("message"), s("hint")))
            }
            Some(v) => Ok(v.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_merges_and_answers() {
        let mut f = Fixtures::parse(
            concat!(
                "// x\nwolkitFixtures({\"app\":{\"locale\":\"en\"},\"calls\":{\"about\":{\"name\":\"WolOS\"},",
                "\"battery\":{\"$error\":{\"code\":\"unavailable\",\"message\":\"m\",\"hint\":\"h\"}}}});\n",
            ),
        )
        .unwrap();
        f.merge(
            Fixtures::parse("wolkitFixtures({\"calls\":{\"about\":{\"name\":\"Other\"}}});")
                .unwrap(),
        );
        let none = Map::new();
        assert_eq!(f.call("a", "about", &none).unwrap()["name"], "Other");
        assert_eq!(
            f.call("a", "battery", &none).unwrap_err().code,
            "unavailable"
        );
        assert_eq!(f.call("a", "sys", &none).unwrap_err().code, "unavailable");
        assert!(Fixtures::parse("wolkitFixtures({\"calls\":[]});").is_err());
        assert!(Fixtures::parse("wolkitFixtures({\"extra\":{}});").is_err());
        assert!(Fixtures::parse("wolkitFixtures({\"permissions\":[1]});").is_err());
        f.merge(Fixtures::parse("wolkitFixtures({\"permissions\":[\"system.read\"]});").unwrap());
        assert_eq!(
            f.permissions.as_deref(),
            Some(&["system.read".to_string()][..])
        );
        assert!(Fixtures::parse("{}").is_err());
    }
}

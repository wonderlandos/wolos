// SPDX-License-Identifier: GPL-2.0-or-later
//! YARA-X rule sets, compiled ahead of time. Loading costs tens of MB and hundreds of ms, so only
//! the short-lived worker does it; the resident guard never links them in.
use crate::scan::Deep;
use std::cell::RefCell;
use std::path::Path;

/// Compiled rules in a rules folder.
pub const FILE: &str = "rules.yxc";

/// Loaded rules; each thread scans through its own [`Session`].
pub struct Rules(yara_x::Rules);

/// One thread's scanner.
pub struct Session<'a>(RefCell<yara_x::Scanner<'a>>);

impl Rules {
    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        yara_x::Rules::deserialize(b).ok().map(Self)
    }

    pub fn session(&self) -> Session<'_> {
        Session(RefCell::new(yara_x::Scanner::new(&self.0)))
    }

    /// None: no rules there, or they do not load.
    pub fn open(path: &Path) -> Option<Self> {
        Self::from_bytes(&std::fs::read(path).ok()?)
    }
}

impl Deep for Session<'_> {
    fn matches(&self, data: &[u8]) -> Vec<String> {
        self.0.borrow_mut().scan(data).map_or_else(
            |_| Vec::new(),
            |r| {
                r.matching_rules()
                    .map(|m| m.identifier().to_owned())
                    .collect()
            },
        )
    }
}

/// Compile rule sources to the [`FILE`] format; each source gets its own namespace.
/// Returns the bytes and the number of sources refused.
pub fn compile(sources: &[(String, String)]) -> Result<(Vec<u8>, usize), String> {
    let mut c = yara_x::Compiler::new();
    c.relaxed_re_syntax(true);
    let mut refused = 0;
    for (name, text) in sources {
        c.new_namespace(name);
        refused += usize::from(c.add_source(text.as_str()).is_err());
    }
    let bytes = c.build().serialize().map_err(|e| e.to_string())?;
    Ok((bytes, refused))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiled_rules_round_trip_and_match() {
        let src = r#"rule TestRule { strings: $a = "EICAR-STANDARD" condition: $a }"#;
        let (b, refused) = compile(&[("t".into(), src.into())]).unwrap();
        assert_eq!(refused, 0);
        let rules = Rules::from_bytes(&b).unwrap();
        let r = rules.session();
        assert_eq!(r.matches(b"x EICAR-STANDARD x"), ["TestRule"]);
        assert!(r.matches(b"nothing").is_empty());
        assert_eq!(
            r.matches(b"EICAR-STANDARD"),
            ["TestRule"],
            "a scanner is reused"
        );
        assert!(Rules::from_bytes(b"junk").is_none());
    }

    #[test]
    fn a_bad_source_is_counted_not_fatal() {
        let (b, refused) = compile(&[
            ("bad".into(), "rule {".into()),
            ("ok".into(), "rule A { condition: true }".into()),
        ])
        .unwrap();
        assert_eq!(refused, 1);
        assert_eq!(
            Rules::from_bytes(&b).unwrap().session().matches(b"x"),
            ["A"]
        );
    }
}

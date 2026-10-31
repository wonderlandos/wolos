// SPDX-License-Identifier: GPL-2.0-or-later
//! YARA-X rule sets, compiled ahead of time. Loading costs tens of MB and hundreds of ms, so only
//! the short-lived worker does it; the resident guard never links them in.
use crate::scan::Deep;
use std::cell::RefCell;
use std::path::Path;

/// Compiled rules in a rules folder.
pub const FILE: &str = "rules.yxc";

pub struct Rules {
    rules: Box<yara_x::Rules>,
    scanner: RefCell<Option<yara_x::Scanner<'static>>>,
}

impl Rules {
    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        let rules = Box::new(yara_x::Rules::deserialize(b).ok()?);
        Some(Self {
            rules,
            scanner: RefCell::new(None),
        })
    }

    /// None: no rules there, or they do not load.
    pub fn open(path: &Path) -> Option<Self> {
        Self::from_bytes(&std::fs::read(path).ok()?)
    }
}

impl Deep for Rules {
    fn matches(&self, data: &[u8]) -> Vec<String> {
        let mut slot = self.scanner.borrow_mut();
        let s = slot.get_or_insert_with(|| {
            // SAFETY: the box never moves out of self, and the field order drops the scanner first
            // is not relied on: `scanner` is declared after `rules` and cleared in Drop.
            let r: &'static yara_x::Rules = unsafe { &*std::ptr::from_ref(self.rules.as_ref()) };
            yara_x::Scanner::new(r)
        });
        s.scan(data).map_or_else(
            |_| Vec::new(),
            |r| {
                r.matching_rules()
                    .map(|m| m.identifier().to_owned())
                    .collect()
            },
        )
    }
}

impl Drop for Rules {
    fn drop(&mut self) {
        self.scanner.get_mut().take();
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
        let r = Rules::from_bytes(&b).unwrap();
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
        assert_eq!(Rules::from_bytes(&b).unwrap().matches(b"x"), ["A"]);
    }
}

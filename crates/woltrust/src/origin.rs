// SPDX-License-Identifier: BSD-2-Clause
//! Where a signed release comes from (ADR 0046, "Origin"): the source repository, tag, commit and
//! build date, carried inside `SHA256.wsig` as the exact bytes the signer signed (base64url in
//! `"origin"`). With an origin the signed message is `label || 0x00 || SHA-512(content) ||
//! SHA-512(origin)`; without one it is unchanged. The verifier checks offline that the repository
//! belongs to the certificate's GitHub login (or its organisation), so the Store can say "Made by
//! Ana (ana on GitHub), from github.com/ana/notes, version 1.2".
use crate::cert::Subject;
use serde::{Deserialize, Serialize};

/// Largest origin record.
pub const MAX_ORIGIN: usize = 1024;
const HOST: &str = "https://github.com/";
/// The GitHub owner of WolOS's own repositories (origins of `org.wolos.` releases).
pub const WOLOS_OWNER: &str = "wonderlandos";

/// The origin of one release.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Origin {
    /// `https://github.com/<owner>/<name>`.
    pub repo: String,
    /// The release tag.
    pub tag: String,
    /// The full commit id (40 lowercase hex).
    pub commit: String,
    /// When it was built, Unix seconds.
    pub built: u64,
    /// The serial of the revocation database the signer held (lead decision "RKDB"): a machine
    /// that verifies this release never accepts an older database again. Absent in older
    /// releases.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rkdb_serial: Option<u64>,
}

fn gh_name(s: &str, max: usize) -> bool {
    !s.is_empty()
        && s.len() <= max
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        && !s.starts_with('.')
}

impl Origin {
    /// Parses and checks the exact bytes.
    pub fn parse(bytes: &[u8]) -> Result<Origin, &'static str> {
        if bytes.len() > MAX_ORIGIN {
            return Err("origin too large");
        }
        let o: Origin = serde_json::from_slice(bytes).map_err(|_| "origin damaged")?;
        o.owner()
            .ok_or("origin repository is not https://github.com/OWNER/NAME")?;
        let tag_ok =
            !o.tag.is_empty() && o.tag.len() <= 128 && o.tag.bytes().all(|b| b.is_ascii_graphic());
        let commit_ok = o.commit.len() == 40
            && o.commit
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !tag_ok || !commit_ok {
            return Err("origin tag or commit malformed");
        }
        Ok(o)
    }

    /// The exact bytes to sign (key order as declared, no spaces).
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    /// The repository owner, when `repo` has the GitHub form.
    pub fn owner(&self) -> Option<&str> {
        let rest = self.repo.strip_prefix(HOST)?;
        let (owner, name) = rest.split_once('/')?;
        (gh_name(owner, 39) && gh_name(name, 100)).then_some(owner)
    }

    /// The repository as shown (`github.com/ana/notes`).
    pub fn shown(&self) -> &str {
        self.repo.strip_prefix("https://").unwrap_or(&self.repo)
    }

    /// True when the repository belongs to the subject's login or organisation (GitHub logins
    /// compare without case).
    pub fn belongs_to(&self, s: &Subject) -> bool {
        let Some(owner) = self.owner() else {
            return false;
        };
        let is = |l: &Option<String>| l.as_deref().is_some_and(|l| l.eq_ignore_ascii_case(owner));
        is(&s.login)
            || s.org
                .as_ref()
                .is_some_and(|o| o.login.eq_ignore_ascii_case(owner))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn o(repo: &str) -> Origin {
        Origin {
            repo: repo.into(),
            tag: "v1.2".into(),
            commit: "0123456789abcdef0123456789abcdef01234567".into(),
            built: 5,
            rkdb_serial: None,
        }
    }

    #[test]
    fn origins_parse_and_name_their_owner() {
        let a = o("https://github.com/ana/notes");
        assert_eq!(Origin::parse(&a.to_bytes()), Ok(a.clone()));
        assert_eq!(a.owner(), Some("ana"));
        assert_eq!(a.shown(), "github.com/ana/notes");
        for bad in [
            "http://github.com/ana/notes",
            "https://gitlab.com/ana/notes",
            "https://github.com/ana",
            "https://github.com/ana/notes/extra",
            "https://github.com/../x",
        ] {
            assert!(Origin::parse(&o(bad).to_bytes()).is_err(), "{bad}");
        }
        let mut c = o("https://github.com/ana/notes");
        c.commit = "abc".into();
        assert!(Origin::parse(&c.to_bytes()).is_err());
    }

    #[test]
    fn an_origin_belongs_to_the_login_or_the_organisation() {
        let s = Subject {
            github_id: Some(1),
            login: Some("Ana".into()),
            name: Some("Ana".into()),
            label: None,
            org: Some(crate::cert::OrgRef {
                github_id: 9,
                login: "anaco".into(),
            }),
            text: None,
        };
        assert!(o("https://github.com/ana/notes").belongs_to(&s));
        assert!(o("https://github.com/AnaCo/notes").belongs_to(&s));
        assert!(!o("https://github.com/bob/notes").belongs_to(&s));
    }
}

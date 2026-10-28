//! The WolOS base (ADR 0017): `wolos-base` ships a manifest of every base file
//! (`<sha256>  <path>` per line, like `sha256sum`); `wol base verify` reports the files that
//! changed or went missing, like `freebsd-update IDS`. Pure: the caller hashes files.

/// Manifest the `wolos-base` package installs.
pub const MANIFEST: &str = "/usr/share/wolos/base.manifest";

/// One manifest line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    pub sha256: String,
}

/// What differs from the manifest.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Finding {
    pub path: String,
    /// `changed` or `missing`.
    pub state: &'static str,
}

/// Manifest lines `<64 hex>  <absolute path>` (one or two spaces; `*` binary marker allowed);
/// blank lines and `#` comments skipped. None when any other line is malformed.
pub fn parse_manifest(text: &str) -> Option<Vec<Entry>> {
    text.lines()
        .map(str::trim_end)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let (hash, path) = l.split_once(' ')?;
            let path = path.trim_start_matches(' ').trim_start_matches('*');
            (hash.len() == 64
                && hash.bytes().all(|b| b.is_ascii_hexdigit())
                && path.starts_with('/')
                && !path.split('/').any(|c| c == ".."))
            .then(|| Entry {
                path: path.into(),
                sha256: hash.to_ascii_lowercase(),
            })
        })
        .collect()
}

/// Compare each entry with `hash(path)` (None: missing or unreadable).
pub fn verify(entries: &[Entry], hash: &dyn Fn(&str) -> Option<String>) -> Vec<Finding> {
    entries
        .iter()
        .filter_map(|e| match hash(&e.path) {
            None => Some(Finding {
                path: e.path.clone(),
                state: "missing",
            }),
            Some(h) if !h.eq_ignore_ascii_case(&e.sha256) => Some(Finding {
                path: e.path.clone(),
                state: "changed",
            }),
            Some(_) => None,
        })
        .collect()
}

/// WolOS base version from `/etc/os-release` (VERSION_ID).
pub fn version(os_release: &str) -> Option<String> {
    crate::games::doctor::parse_os_release(os_release)
        .map(|(_, v)| v)
        .filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    const B: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn manifest_lines() {
        let text = format!("# wolos-base 2026.11\n{A}  /usr/bin/ls\n\n{B} */usr/lib/os-release\n");
        let m = parse_manifest(&text).unwrap();
        assert_eq!(
            m,
            [
                Entry {
                    path: "/usr/bin/ls".into(),
                    sha256: A.into()
                },
                Entry {
                    path: "/usr/lib/os-release".into(),
                    sha256: B.into()
                },
            ]
        );
        for bad in [
            "abc  /usr/bin/ls",
            &format!("{A}  usr/bin/ls"),
            &format!("{A}  /usr/../etc/shadow"),
            A,
        ] {
            assert_eq!(parse_manifest(bad), None, "{bad}");
        }
        assert_eq!(parse_manifest(""), Some(vec![]));
    }

    #[test]
    fn reports_changed_and_missing_files() {
        let m = parse_manifest(&format!(
            "{A}  /usr/bin/ls\n{B}  /usr/bin/cp\n{A}  /usr/bin/gone\n"
        ))
        .unwrap();
        let hash = |p: &str| match p {
            "/usr/bin/ls" => Some(A.to_ascii_uppercase()),
            "/usr/bin/cp" => Some(A.to_string()),
            _ => None,
        };
        assert_eq!(
            verify(&m, &hash),
            [
                Finding {
                    path: "/usr/bin/cp".into(),
                    state: "changed"
                },
                Finding {
                    path: "/usr/bin/gone".into(),
                    state: "missing"
                },
            ]
        );
        assert_eq!(
            version("NAME=WolOS\nID=wolos\nVERSION_ID=\"2026.11\"\n").as_deref(),
            Some("2026.11")
        );
        assert_eq!(version("ID=wolos\n"), None);
    }
}

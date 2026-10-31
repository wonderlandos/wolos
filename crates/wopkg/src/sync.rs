//! Index update: fetch every source's input (conditional GET, in parallel), then rebuild only the
//! segments whose input changed (size + mtime + parser version), also in parallel.
use crate::context::Context;
use crate::fetch::{self, Outcome};
use crate::ffi::{Error, Result};
use crate::index::{Kind, parse, segment};
use serde::{Deserialize, Serialize};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// Bump when a parser changes what it produces: every segment is rebuilt once.
pub const PARSER_VERSION: u32 = 3;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Source {
    /// Segment name: `core`, `extra`, `flathub`, `aur`, ...
    pub name: String,
    pub kind: Kind,
    /// Remote input (fetched into the cache) ...
    #[serde(default)]
    pub url: Option<String>,
    /// ... or a local file (a pacman sync db, Flatpak's own appstream copy).
    #[serde(default)]
    pub path: Option<PathBuf>,
    /// Sub-format for `pacman` (always a db) and `appimage` (`feed`); unused otherwise.
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Request {
    pub sources: Vec<Source>,
    /// Remove segments of sources not in this list.
    #[serde(default)]
    pub prune: bool,
}

#[derive(Clone, Debug, Serialize, Default)]
pub struct Report {
    pub name: String,
    pub records: usize,
    pub rebuilt: bool,
    pub fetched: Option<Outcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub fn valid_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 64
        && !n.starts_with('.')
        && n.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

fn fingerprint(p: &Path) -> Option<String> {
    let m = std::fs::metadata(p).ok()?;
    Some(format!(
        "v{PARSER_VERSION} {} {}.{}",
        m.len(),
        m.mtime(),
        m.mtime_nsec()
    ))
}

pub fn build(kind: Kind, name: &str, bytes: &[u8]) -> Result<Vec<crate::index::Record>> {
    match kind {
        Kind::Pacman => parse::pacman_db(name, bytes),
        Kind::Flatpak => parse::appstream(name, bytes),
        Kind::Aur => parse::aur(bytes),
        Kind::Wol => parse::wol_index(bytes),
        Kind::Recipe => parse::recipes(bytes),
        Kind::Appimage => parse::appimage_feed(bytes),
    }
}

/// Rebuild one segment from `input` unless its fingerprint is unchanged.
pub fn rebuild(ctx: &Context, src: &Source, input: &Path) -> Result<(usize, bool)> {
    let seg_path = ctx.index_dir().join(format!("{}.seg", src.name));
    let fp = fingerprint(input)
        .ok_or_else(|| Error::unavailable(format!("The {} list is missing.", src.name)))?;
    let fp = format!("{} {}", src.kind.name(), fp);
    if let Some(s) = segment::Segment::open(&seg_path)
        && s.fingerprint() == fp
    {
        return Ok((s.len(), false));
    }
    let bytes = std::fs::read(input)
        .map_err(|_| Error::unavailable(format!("The {} list cannot be read.", src.name)))?;
    let recs = build(src.kind, &src.name, &bytes)?;
    segment::write(&seg_path, &fp, &recs)?;
    Ok((recs.len(), true))
}

pub fn run(
    ctx: &Context,
    req: &Request,
    progress: &mut dyn FnMut(&serde_json::Value),
) -> Result<Vec<Report>> {
    for s in &req.sources {
        if !valid_name(&s.name) {
            return Err(Error::invalid(format!(
                "“{}” is not a valid source name.",
                s.name
            )));
        }
        if s.url.is_none() == s.path.is_none() {
            return Err(Error::invalid(format!(
                "Source {} needs exactly one of url and path.",
                s.name
            )));
        }
    }
    let inputs = ctx.cache.join("inputs");
    let reports: Vec<Report> = std::thread::scope(|sc| {
        let handles: Vec<_> = req
            .sources
            .iter()
            .map(|s| {
                let inputs = inputs.clone();
                sc.spawn(move || {
                    let mut r = Report {
                        name: s.name.clone(),
                        ..Default::default()
                    };
                    let input = match (&s.url, &s.path) {
                        (Some(u), _) => {
                            let dest = inputs.join(&s.name);
                            match fetch::conditional(&ctx.net, u, &dest) {
                                Ok(o) => r.fetched = Some(o),
                                Err(e) if dest.exists() => {
                                    r.error = Some(format!("{e} Using the last copy."))
                                }
                                Err(e) => {
                                    r.error = Some(e.message);
                                    return r;
                                }
                            }
                            dest
                        }
                        (None, Some(p)) => p.clone(),
                        _ => unreachable!(),
                    };
                    match rebuild(ctx, s, &input) {
                        Ok((n, b)) => {
                            r.records = n;
                            r.rebuilt = b;
                        }
                        Err(e) => r.error = Some(e.message),
                    }
                    r
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_default())
            .collect()
    });
    for r in &reports {
        progress(&serde_json::json!({"event":"source","report":r}));
    }
    if req.prune {
        let keep: Vec<String> = req
            .sources
            .iter()
            .map(|s| format!("{}.seg", s.name))
            .collect();
        for e in std::fs::read_dir(ctx.index_dir())
            .into_iter()
            .flatten()
            .flatten()
        {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.ends_with(".seg") && !keep.contains(&n) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    Ok(reports)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{Served, serve, temp};

    fn ctx(d: &Path) -> Context {
        Context::new(crate::context::Config {
            home: Some(d.to_path_buf()),
            cache_dir: Some(d.join("cache")),
            allow_loopback_http: true,
            ..Default::default()
        })
        .unwrap()
    }

    #[test]
    fn incremental_rebuild_and_remote_inputs() {
        let d = temp("sync");
        let c = ctx(&d);
        let db = d.join("extra.db");
        std::fs::write(
            &db,
            crate::index::parse::tests_support::tar(&[(
                "vim-1-1/desc",
                "%NAME%\nvim\n\n%VERSION%\n1-1\n",
            )]),
        )
        .unwrap();
        let srv = serve(vec![(
            "/aur.json",
            Served::bytes(br#"[{"Name":"paru","Version":"2"}]"#.to_vec()).etag("\"1\""),
        )]);
        let req = Request {
            sources: vec![
                Source {
                    name: "extra".into(),
                    kind: Kind::Pacman,
                    url: None,
                    path: Some(db.clone()),
                    format: None,
                },
                Source {
                    name: "aur".into(),
                    kind: Kind::Aur,
                    url: Some(srv.url("/aur.json")),
                    path: None,
                    format: None,
                },
            ],
            prune: true,
        };
        let r = run(&c, &req, &mut |_| {}).unwrap();
        assert_eq!(
            (r[0].records, r[0].rebuilt, r[1].records, r[1].rebuilt),
            (1, true, 1, true)
        );
        let r = run(&c, &req, &mut |_| {}).unwrap();
        assert!(
            !r[0].rebuilt && !r[1].rebuilt,
            "unchanged inputs are not rebuilt"
        );
        assert_eq!(r[1].fetched, Some(Outcome::NotModified));
        std::fs::write(d.join("cache/index/old.seg"), b"x").unwrap();
        run(&c, &req, &mut |_| {}).unwrap();
        assert!(!d.join("cache/index/old.seg").exists(), "pruned");
        assert_eq!(c.segments().len(), 2);
    }

    #[test]
    fn names_are_checked() {
        for (n, ok) in [
            ("core", true),
            ("wolos-cachy", true),
            ("../x", false),
            (".x", false),
            ("", false),
            ("a b", false),
        ] {
            assert_eq!(valid_name(n), ok, "{n}");
        }
    }
}

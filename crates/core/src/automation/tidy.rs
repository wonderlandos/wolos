//! Downloads tidy: sort loose files into kind folders (Images, Documents, ...). Dry run first: the
//! plan carries an id, and only that exact plan is applied. Only regular files directly inside a
//! folder strictly inside the home; never hidden or partial downloads; never replaces a file;
//! unknown kinds stay put. Moves are journaled, so `wol undo` puts them back.
use serde::Serialize;
use std::path::{Path, PathBuf};

/// (folder, extensions, lowercase).
pub const KINDS: [(&str, &[&str]); 6] = [
    (
        "Images",
        &[
            "png", "jpg", "jpeg", "gif", "webp", "heic", "svg", "bmp", "tif", "tiff",
        ],
    ),
    (
        "Documents",
        &[
            "pdf", "txt", "md", "rtf", "odt", "ods", "odp", "doc", "docx", "xls", "xlsx", "ppt",
            "pptx", "csv", "epub",
        ],
    ),
    (
        "Archives",
        &["zip", "tar", "gz", "tgz", "xz", "bz2", "zst", "7z", "rar"],
    ),
    (
        "Audio",
        &["mp3", "flac", "ogg", "opus", "wav", "m4a", "aac"],
    ),
    ("Video", &["mp4", "mkv", "webm", "mov", "avi"]),
    ("Disk Images", &["iso", "img"]),
];
/// Downloads still in progress.
const PARTIAL: [&str; 4] = ["part", "crdownload", "download", "partial"];

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Move {
    pub from: PathBuf,
    pub to: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Plan {
    pub dir: PathBuf,
    /// Pass back to apply exactly this plan.
    pub id: String,
    pub moves: Vec<Move>,
}

pub fn kind_of(name: &str) -> Option<&'static str> {
    let (stem, ext) = name.rsplit_once('.')?;
    let ext = ext.to_ascii_lowercase();
    if stem.is_empty() || name.starts_with('.') || PARTIAL.contains(&ext.as_str()) {
        return None;
    }
    KINDS
        .iter()
        .find(|(_, exts)| exts.contains(&ext.as_str()))
        .map(|k| k.0)
}

/// `name (2).ext` .. `name (99).ext`.
fn numbered(name: &str, n: u32) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) => format!("{stem} ({n}).{ext}"),
        None => format!("{name} ({n})"),
    }
}

/// FNV-1a 64 over the moves: same files, same destinations, same id.
fn plan_id(moves: &[Move]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for m in moves {
        for b in m
            .from
            .as_os_str()
            .as_encoded_bytes()
            .iter()
            .chain(b"\0")
            .chain(m.to.as_os_str().as_encoded_bytes())
            .chain(b"\n")
        {
            h = (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("{h:016x}")
}

/// Pure: `names` are the regular files in `dir`; `exists` answers for destinations.
pub fn plan(dir: &Path, names: &[String], exists: &dyn Fn(&Path) -> bool) -> Plan {
    let mut names: Vec<&String> = names.iter().collect();
    names.sort();
    let mut moves: Vec<Move> = Vec::new();
    for name in names {
        let Some(kind) = kind_of(name) else { continue };
        let taken = |p: &Path| exists(p) || moves.iter().any(|m| m.to == p);
        let to = (1..100)
            .map(|n| {
                dir.join(kind).join(if n == 1 {
                    name.clone()
                } else {
                    numbered(name, n)
                })
            })
            .find(|p| !taken(p));
        if let Some(to) = to {
            moves.push(Move {
                from: dir.join(name),
                to,
            });
        }
    }
    Plan {
        dir: dir.to_owned(),
        id: plan_id(&moves),
        moves,
    }
}

/// Regular files directly in `dir` (no symlinks, no hidden files).
pub fn scan(dir: &Path) -> std::io::Result<Vec<String>> {
    let mut v = Vec::new();
    for e in std::fs::read_dir(dir)?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.starts_with('.') && e.file_type().is_ok_and(|t| t.is_file()) {
            v.push(name);
        }
    }
    Ok(v)
}

/// `dir` resolved, and strictly inside the (resolved) home.
pub fn in_home(home: &Path, dir: &Path) -> Result<PathBuf, String> {
    let h = home
        .canonicalize()
        .map_err(|e| format!("{}: {e}", home.display()))?;
    let d = dir
        .canonicalize()
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    (d.starts_with(&h) && d != h)
        .then_some(d)
        .ok_or_else(|| format!("{} is not a folder inside your home", dir.display()))
}

/// `XDG_DOWNLOAD_DIR` from `user-dirs.dirs`, else `~/Downloads`.
pub fn downloads_dir(home: &Path, user_dirs: Option<&str>) -> PathBuf {
    user_dirs
        .and_then(|t| {
            t.lines().find_map(|l| {
                let v = l
                    .trim()
                    .strip_prefix("XDG_DOWNLOAD_DIR=")?
                    .trim_matches('"');
                Some(match v.strip_prefix("$HOME") {
                    Some(r) => home.join(r.trim_start_matches('/')),
                    None => PathBuf::from(v),
                })
            })
        })
        .unwrap_or_else(|| home.join("Downloads"))
}

/// Applies the moves in order, never replacing a file; stops at the first failure. Returns the
/// moves done (journal them even on error) and the error.
pub fn apply(plan: &Plan) -> (Vec<(PathBuf, PathBuf)>, Option<std::io::Error>) {
    let mut done = Vec::new();
    for m in &plan.moves {
        if let Err(e) = crate::journal::move_no_clobber(&m.from, &m.to) {
            return (done, Some(e));
        }
        done.push((m.from.clone(), m.to.clone()));
    }
    (done, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_table() {
        for (name, want) in [
            ("a.PNG", Some("Images")),
            ("report.pdf", Some("Documents")),
            ("x.tar.gz", Some("Archives")),
            ("FreeBSD.iso", Some("Disk Images")),
            ("movie.mkv.part", None),
            ("setup.crdownload", None),
            (".hidden.png", None),
            ("Makefile", None),
            ("weird.xyz", None),
        ] {
            assert_eq!(kind_of(name), want, "{name}");
        }
    }

    #[test]
    fn plan_never_reuses_a_name_and_is_stable() {
        let dir = Path::new("/h/Downloads");
        let names: Vec<String> = ["b.png", "a.png", "notes.txt", "keep.xyz"]
            .map(String::from)
            .to_vec();
        let exists = |p: &Path| p == Path::new("/h/Downloads/Images/a.png");
        let p = plan(dir, &names, &exists);
        let got: Vec<_> = p
            .moves
            .iter()
            .map(|m| m.to.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            got,
            [
                "/h/Downloads/Images/a (2).png",
                "/h/Downloads/Images/b.png",
                "/h/Downloads/Documents/notes.txt"
            ]
        );
        assert_eq!(plan(dir, &names, &exists).id, p.id, "deterministic");
        assert_ne!(plan(dir, &names, &|_| false).id, p.id);
        assert_eq!(p.id.len(), 16);
        // "a (2).png" is planned first; "a.png" must skip that name too.
        let names: Vec<String> = ["a.png", "a (2).png"].map(String::from).to_vec();
        let to: Vec<_> = plan(dir, &names, &exists)
            .moves
            .into_iter()
            .map(|m| m.to.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            to,
            [
                "/h/Downloads/Images/a (2).png",
                "/h/Downloads/Images/a (3).png"
            ]
        );
    }

    #[test]
    fn scope_scan_apply() {
        let home = std::env::temp_dir().join(format!("wol-tidy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let dl = home.join("Downloads");
        std::fs::create_dir_all(dl.join("sub")).unwrap();
        for f in ["a.png", ".x.png", "b.pdf"] {
            std::fs::write(dl.join(f), f).unwrap();
        }
        std::os::unix::fs::symlink("/etc/passwd", dl.join("link.txt")).unwrap();
        let mut names = scan(&dl).unwrap();
        names.sort();
        assert_eq!(
            names,
            ["a.png", "b.pdf"],
            "no hidden files, dirs or symlinks"
        );
        assert!(in_home(&home, &home).is_err(), "not the home itself");
        assert!(in_home(&home, Path::new("/tmp")).is_err());
        assert!(in_home(&home, &dl.join("sub/..")).is_ok());
        let p = plan(&dl, &names, &|p| p.exists());
        let (done, err) = apply(&p);
        assert!(err.is_none());
        assert_eq!(done.len(), 2);
        assert_eq!(
            std::fs::read_to_string(dl.join("Images/a.png")).unwrap(),
            "a.png"
        );
        assert_eq!(
            downloads_dir(
                Path::new("/h"),
                Some("XDG_DOWNLOAD_DIR=\"$HOME/Transferências\"\n")
            ),
            Path::new("/h/Transferências")
        );
        assert_eq!(
            downloads_dir(Path::new("/h"), None),
            Path::new("/h/Downloads")
        );
        std::fs::remove_dir_all(home).unwrap();
    }
}

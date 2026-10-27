//! Spotlight's file names: a per-user list of `$HOME` (one relative path per line, folders end
//! in `/`), rebuilt by the shell at login and by Spotlight when stale, searched by name in place.
use crate::search::Item;
use std::path::{Path, PathBuf};

/// Folders never listed: build output and package caches, thousands of names nobody looks for.
const SKIP: [&str; 3] = ["node_modules", "target", "__pycache__"];
/// Levels below `$HOME` that are listed.
pub const DEPTH: usize = 8;
/// Names listed at most (shallow ones first).
pub const MAX: usize = 200_000;
/// A list older than this is rebuilt when Spotlight opens.
pub const STALE: std::time::Duration = std::time::Duration::from_secs(30);

/// `$XDG_CACHE_HOME/wolos/files` (else `~/.cache/wolos/files`).
pub fn cache_file(home: &Path) -> PathBuf {
    cache_under(std::env::var_os("XDG_CACHE_HOME"), home)
}

fn cache_under(xdg: Option<std::ffi::OsString>, home: &Path) -> PathBuf {
    xdg.filter(|x| !x.is_empty())
        .map_or_else(|| home.join(".cache"), PathBuf::from)
        .join("wolos/files")
}

/// Every name under `home`, breadth first: no hidden names, no [`SKIP`] folders, no symlinked
/// folders followed.
pub fn walk(home: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut dirs = std::collections::VecDeque::from([(PathBuf::new(), 0usize)]);
    while let Some((rel, depth)) = dirs.pop_front() {
        let Ok(rd) = std::fs::read_dir(home.join(&rel)) else {
            continue;
        };
        for e in rd.flatten() {
            let name = e.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.starts_with('.') || name.contains('\n') {
                continue;
            }
            let Ok(t) = e.file_type() else { continue };
            let path = rel.join(name);
            let Some(p) = path.to_str() else { continue };
            if t.is_dir() {
                if SKIP.contains(&name) {
                    continue;
                }
                out.push(format!("{p}/"));
                if depth + 1 < DEPTH {
                    dirs.push_back((path, depth + 1));
                }
            } else {
                out.push(p.to_string());
            }
            if out.len() >= MAX {
                return out;
            }
        }
    }
    out
}

/// Writes the list whole (a reader never sees half of it).
pub fn save(text: &str, file: &Path) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = file.with_extension("new");
    std::fs::write(&tmp, text)?;
    std::fs::rename(tmp, file)
}

/// The saved list, and whether it is missing or older than [`STALE`].
pub fn load(home: &Path) -> (Index, bool) {
    let file = cache_file(home);
    let stale = std::fs::metadata(&file)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_none_or(|age| age > STALE);
    let list = std::fs::read_to_string(&file).unwrap_or_default();
    (Index::parse(&list, home), stale)
}

/// Lists `home` again and saves it (shell at login and hourly; Spotlight when stale).
pub fn rebuild(home: &Path) -> Index {
    let text = lines(&walk(home));
    let _ = save(&text, &cache_file(home));
    Index::parse(&text, home)
}

/// The list as saved: one name per line.
pub fn lines(list: &[String]) -> String {
    let mut text = list.join("\n");
    text.push('\n');
    text
}

/// Kind shown on the row and its icon.
pub fn kind(name: &str, folder: bool) -> (&'static str, &'static str) {
    if folder {
        return ("Folder", "folder");
    }
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "pdf" => ("PDF Document", "application-pdf"),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "heic" | "bmp" | "tiff" => {
            ("Image", "image-x-generic")
        }
        "txt" | "md" | "rst" | "log" | "csv" => ("Text", "text-x-generic"),
        "mp3" | "flac" | "ogg" | "opus" | "wav" | "m4a" => ("Audio", "audio-x-generic"),
        "mp4" | "mkv" | "webm" | "mov" | "avi" => ("Movie", "video-x-generic"),
        "zip" | "gz" | "xz" | "zst" | "bz2" | "tar" | "7z" => ("Archive", "package-x-generic"),
        _ => ("Document", "text-x-generic"),
    }
}

/// The list in memory: paths plus lowercase names; rows are made only for the hits.
#[derive(Default)]
pub struct Index {
    home: PathBuf,
    rel: Vec<Box<str>>,
    lower: Vec<Box<str>>,
}

impl Index {
    pub fn parse(list: &str, home: &Path) -> Self {
        let rel: Vec<Box<str>> = list
            .lines()
            .filter(|l| !l.is_empty())
            .map(Into::into)
            .collect();
        let lower = rel.iter().map(|r| name(r).to_lowercase().into()).collect();
        Index {
            home: home.to_path_buf(),
            rel,
            lower,
        }
    }

    pub fn len(&self) -> usize {
        self.rel.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rel.is_empty()
    }

    /// Best `max` names containing the query: name start, word start, then anywhere; shorter
    /// names first. Opening one runs `gio open <path>`.
    pub fn search(&self, query: &str, max: usize) -> Vec<Item> {
        let q = query.trim().to_lowercase();
        if q.is_empty() || max == 0 {
            return vec![];
        }
        // `contains` first (fast); only those get ranked, into a short sorted list of the best:
        // rank desc, shorter, earlier. No list of every hit (a one-letter query hits them all).
        let mut best: Vec<(std::cmp::Reverse<u8>, usize, usize)> = Vec::with_capacity(max + 1);
        for (i, n) in self.lower.iter().enumerate() {
            if !n.contains(q.as_str()) {
                continue;
            }
            let key = (std::cmp::Reverse(crate::search::rank(n, &q)), n.len(), i);
            if best.len() == max && key >= best[max - 1] {
                continue;
            }
            let at = best.partition_point(|b| *b < key);
            best.insert(at, key);
            best.truncate(max);
        }
        best.into_iter().map(|(_, _, i)| self.item(i)).collect()
    }

    fn item(&self, i: usize) -> Item {
        let r = &*self.rel[i];
        let folder = r.ends_with('/');
        let (kind, icon) = kind(name(r), folder);
        let path = self.home.join(r.trim_end_matches('/'));
        Item::new(
            name(r),
            kind,
            icon,
            vec![
                "gio".into(),
                "open".into(),
                path.to_string_lossy().into_owned(),
            ],
        )
    }
}

/// Last component of a list line (`Documents/` -> `Documents`).
fn name(rel: &str) -> &str {
    let r = rel.trim_end_matches('/');
    r.rsplit_once('/').map_or(r, |(_, n)| n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(root: &Path, paths: &[&str]) {
        for p in paths {
            let f = root.join(p);
            if p.ends_with('/') {
                std::fs::create_dir_all(&f).unwrap();
            } else {
                std::fs::create_dir_all(f.parent().unwrap()).unwrap();
                std::fs::write(&f, "").unwrap();
            }
        }
    }

    #[test]
    fn lists_home_without_hidden_names_or_build_output() {
        let home = std::env::temp_dir().join(format!("wol-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        tree(
            &home,
            &[
                "Documents/Taxes 2026.pdf",
                "Documents/Notes/plan.md",
                ".config/secret.txt",
                "Documents/.hidden",
                "code/app/node_modules/left-pad/index.js",
                "code/app/target/debug/app",
                "code/app/main.rs",
            ],
        );
        std::os::unix::fs::symlink(&home, home.join("loop")).unwrap();
        let mut got = walk(&home);
        got.sort();
        assert_eq!(
            got,
            [
                "Documents/",
                "Documents/Notes/",
                "Documents/Notes/plan.md",
                "Documents/Taxes 2026.pdf",
                "code/",
                "code/app/",
                "code/app/main.rs",
                "loop",
            ]
        );
        let cache = home.join("cache/wolos/files");
        save(&lines(&got), &cache).unwrap();
        let idx = Index::parse(&std::fs::read_to_string(&cache).unwrap(), &home);
        assert_eq!(idx.len(), got.len());
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn finds_names_and_opens_the_full_path() {
        let idx = Index::parse(
            "Documents/\nDocuments/Taxes 2026.pdf\nPictures/tax receipt.png\nsyntax.txt\nMusic/\n",
            Path::new("/home/alice"),
        );
        let hits = idx.search("Tax", 8);
        let titles: Vec<&str> = hits.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, ["Taxes 2026.pdf", "tax receipt.png", "syntax.txt"]);
        assert_eq!(
            hits[0].exec,
            ["gio", "open", "/home/alice/Documents/Taxes 2026.pdf"]
        );
        assert_eq!((hits[0].kind, hits[0].icon.as_str()), kind("x.pdf", false));
        let docs = &idx.search("docu", 8)[0];
        assert_eq!((docs.title.as_str(), docs.kind), ("Documents", "Folder"));
        assert_eq!(docs.exec[2], "/home/alice/Documents");
        assert!(
            idx.search("tr", 8).is_empty(),
            "no scattered-letter matches"
        );
        assert!(idx.search(" ", 8).is_empty());
        assert_eq!(idx.search("a", 2).len(), 2);
    }

    #[test]
    fn kinds_by_extension() {
        for (name, folder, want) in [
            ("Work", true, "Folder"),
            ("a.PDF", false, "PDF Document"),
            ("b.jpeg", false, "Image"),
            ("c.md", false, "Text"),
            ("d.flac", false, "Audio"),
            ("e.mkv", false, "Movie"),
            ("f.tar.gz", false, "Archive"),
            ("Makefile", false, "Document"),
        ] {
            assert_eq!(kind(name, folder).0, want, "{name}");
        }
    }

    #[test]
    fn cache_follows_xdg() {
        let home = Path::new("/home/alice");
        assert_eq!(cache_under(None, home), home.join(".cache/wolos/files"));
        assert_eq!(
            cache_under(Some("".into()), home),
            home.join(".cache/wolos/files")
        );
        assert_eq!(
            cache_under(Some("/var/cache/alice".into()), home),
            Path::new("/var/cache/alice/wolos/files")
        );
    }

    /// A keystroke stays under 50 ms on a full list (J10). A debug build, sharing the CPU with
    /// the other tests, is ~4x slower (10-36 ms measured alone), hence its wider bound.
    #[test]
    fn a_keystroke_over_the_largest_list_is_fast() {
        let mut list = String::new();
        for i in 0..MAX {
            list.push_str(&format!("Projects/p{}/src/module_{i}.rs\n", i % 97));
        }
        let idx = Index::parse(&list, Path::new("/home/alice"));
        for q in ["m", "mod", "module_19", "zzz"] {
            // Best of three: other test threads share the CPU.
            let took = (0..3)
                .map(|_| {
                    let t0 = std::time::Instant::now();
                    let _ = idx.search(q, 8);
                    t0.elapsed()
                })
                .min()
                .unwrap();
            let budget = if cfg!(debug_assertions) { 200 } else { 50 };
            assert!(took.as_millis() < budget, "{q}: {took:?}");
        }
    }
}

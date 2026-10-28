//! The Dock's Downloads stack: where Downloads is (xdg-user-dirs) and its newest files.

/// One file in the stack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub name: String,
    pub path: String,
    /// Seconds since 1970, newest first.
    pub modified: u64,
    pub dir: bool,
}

/// `XDG_DOWNLOAD_DIR` from `~/.config/user-dirs.dirs`, else `~/Downloads`.
pub fn dir(user_dirs: Option<&str>, home: &str) -> String {
    let fallback = format!("{home}/Downloads");
    let Some(text) = user_dirs else {
        return fallback;
    };
    text.lines()
        .filter_map(|l| l.trim().strip_prefix("XDG_DOWNLOAD_DIR="))
        .next_back()
        .map(|v| v.trim().trim_matches('"').replace("$HOME", home))
        .filter(|v| v.starts_with('/') && v.trim_end_matches('/') != home)
        .unwrap_or(fallback)
}

/// Unfinished downloads and hidden files stay out of the stack.
pub fn shown(name: &str) -> bool {
    !name.starts_with('.')
        && ![".part", ".crdownload", ".download", ".tmp"]
            .iter()
            .any(|s| name.ends_with(s))
}

/// The `n` newest shown items, newest first (ties by name).
pub fn newest(mut items: Vec<Item>, n: usize) -> Vec<Item> {
    items.retain(|i| shown(&i.name));
    items.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.name.cmp(&b.name)));
    items.truncate(n);
    items
}

/// Reads `dir` (blocking, one directory listing).
pub fn read(dir: &str, n: usize) -> Vec<Item> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let items = rd
        .flatten()
        .filter_map(|e| {
            let md = e.metadata().ok()?;
            let modified = md
                .modified()
                .ok()?
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_secs();
            Some(Item {
                name: e.file_name().to_string_lossy().into_owned(),
                path: e.path().to_string_lossy().into_owned(),
                modified,
                dir: md.is_dir(),
            })
        })
        .collect();
    newest(items, n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downloads_dir_from_user_dirs() {
        let t = "# comment\nXDG_DESKTOP_DIR=\"$HOME/Desktop\"\nXDG_DOWNLOAD_DIR=\"$HOME/Transferências\"\n";
        assert_eq!(dir(Some(t), "/home/ann"), "/home/ann/Transferências");
        assert_eq!(dir(None, "/home/ann"), "/home/ann/Downloads");
        // Pointing at Home itself (xdg "disabled") or relative: the default.
        assert_eq!(
            dir(Some("XDG_DOWNLOAD_DIR=\"$HOME/\"\n"), "/home/ann"),
            "/home/ann/Downloads"
        );
        assert_eq!(dir(Some("XDG_DOWNLOAD_DIR=\"x\"\n"), "/h"), "/h/Downloads");
        assert_eq!(
            dir(Some("XDG_DOWNLOAD_DIR=\"/data/dl\"\n"), "/h"),
            "/data/dl"
        );
    }

    #[test]
    fn newest_first_without_partial_or_hidden() {
        let i = |n: &str, m| Item {
            name: n.into(),
            path: format!("/d/{n}"),
            modified: m,
            dir: false,
        };
        let got = newest(
            vec![
                i("old.pdf", 1),
                i("new.zip", 9),
                i("film.mkv.part", 10),
                i(".hidden", 11),
                i("b.png", 5),
                i("a.png", 5),
                i("x.crdownload", 12),
            ],
            3,
        );
        let names: Vec<&str> = got.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["new.zip", "a.png", "b.png"]);
    }

    #[test]
    fn read_lists_a_directory() {
        let d = std::env::temp_dir().join(format!("wol-dl-{}", std::process::id()));
        std::fs::create_dir_all(d.join("folder")).unwrap();
        std::fs::write(d.join("a.txt"), "x").unwrap();
        std::fs::write(d.join("b.part"), "x").unwrap();
        let got = read(&d.to_string_lossy(), 10);
        let mut names: Vec<&str> = got.iter().map(|i| i.name.as_str()).collect();
        names.sort();
        assert_eq!(names, ["a.txt", "folder"]);
        assert!(got.iter().any(|i| i.dir));
        assert!(read("/nonexistent/x", 5).is_empty());
        let _ = std::fs::remove_dir_all(d);
    }
}

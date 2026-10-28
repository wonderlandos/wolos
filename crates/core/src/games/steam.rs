//! Steam facts for programs that run outside Steam: where the client and its libraries live, and
//! which Proton installs in them carry DXVK/vkd3d-proton. Steam itself is the native package
//! (`/usr/bin/steam`) and manages Proton; nothing here installs or launches anything for it.
use std::path::{Path, PathBuf};

/// Steam's install root for this user: `~/.steam/root` (a symlink Steam maintains), else
/// `~/.steam/steam`, else `~/.local/share/Steam`.
pub fn steam_root(home: &Path, exists: &dyn Fn(&Path) -> bool) -> PathBuf {
    let c = [".steam/root", ".steam/steam", ".local/share/Steam"].map(|r| home.join(r));
    c.iter().find(|p| exists(p)).unwrap_or(&c[1]).to_path_buf()
}

/// `"path"` values of `steamapps/libraryfolders.vdf`.
pub fn library_paths(vdf: &str) -> Vec<String> {
    vdf.lines()
        .filter_map(|l| {
            let q: Vec<&str> = l.split('"').collect();
            // "path"<ws>"value" -> ["", path, ws, value, ""]
            (q.len() >= 5 && q[1] == "path").then(|| q[3].replace("\\\\", "\\"))
        })
        .collect()
}

/// Steam library roots (each holds `steamapps/`), the client root first, no duplicates.
pub fn libraries(steam_root: &Path, read: &dyn Fn(&Path) -> Option<String>) -> Vec<PathBuf> {
    let mut out = vec![steam_root.to_path_buf()];
    let vdf = read(&steam_root.join("steamapps/libraryfolders.vdf")).unwrap_or_default();
    for p in library_paths(&vdf).into_iter().map(PathBuf::from) {
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

/// Proton directory names (`steamapps/common/<name>`) best first: Experimental, then the numbered
/// releases newest first. Other names are ignored.
pub fn proton_order(names: &[String]) -> Vec<String> {
    let mut found: Vec<(u32, u32, &String)> = names
        .iter()
        .filter_map(|n| {
            if n == "Proton - Experimental" {
                return Some((u32::MAX, 0, n));
            }
            let v = n.strip_prefix("Proton ")?;
            let v = v.split(' ').next()?;
            let (major, minor) = v.split_once('.').unwrap_or((v, "0"));
            Some((major.parse().ok()?, minor.parse().ok()?, n))
        })
        .collect();
    found.sort_by(|a, b| (b.0, b.1, b.2).cmp(&(a.0, a.1, a.2)));
    found.into_iter().map(|f| f.2.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steam_root_preference() {
        let home = Path::new("/home/u");
        for (present, want) in [
            (vec![".steam/root", ".steam/steam"], ".steam/root"),
            (vec![".steam/steam", ".local/share/Steam"], ".steam/steam"),
            (vec![".local/share/Steam"], ".local/share/Steam"),
            (vec![], ".steam/steam"),
        ] {
            let exists = |p: &Path| present.iter().any(|r| home.join(r) == p);
            assert_eq!(steam_root(home, &exists), home.join(want), "{present:?}");
        }
    }

    #[test]
    fn libraries_from_libraryfolders_vdf() {
        let vdf = r#""libraryfolders"
{
	"0"
	{
		"path"		"/home/u/.steam/steam"
		"label"		""
		"apps"
		{
			"228980"		"1021"
		}
	}
	"1"
	{
		"path"		"/mnt/games/SteamLibrary"
	}
}
"#;
        assert_eq!(
            library_paths(vdf),
            ["/home/u/.steam/steam", "/mnt/games/SteamLibrary"]
        );
        let root = Path::new("/home/u/.steam/steam");
        let read = |p: &Path| (p == root.join("steamapps/libraryfolders.vdf")).then(|| vdf.into());
        assert_eq!(
            libraries(root, &read),
            [root, Path::new("/mnt/games/SteamLibrary")]
        );
        assert_eq!(libraries(root, &|_| None), [root]);
    }

    #[test]
    fn proton_directories_by_preference() {
        let names: Vec<String> = [
            "Proton 9.0 (Beta)",
            "Proton 10.0",
            "Proton - Experimental",
            "Proton 9.0",
            "Proton EasyAntiCheat Runtime",
            "Steamworks Common Redistributables",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(
            proton_order(&names),
            [
                "Proton - Experimental",
                "Proton 10.0",
                "Proton 9.0 (Beta)",
                "Proton 9.0"
            ]
        );
    }
}

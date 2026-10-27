//! DXVK, vkd3d-proton and dxvk-nvapi for a prefix. wine-proton ships none of them; wol-run copies
//! their Windows DLLs into `system32`/`syswow64` and loads them native-first, as Proton does.
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Component {
    /// Direct3D 8-11 on Vulkan.
    Dxvk,
    /// Direct3D 12 on Vulkan.
    Vkd3d,
    /// NVIDIA's NVAPI (DLSS, Reflex) for DXVK; NVIDIA GPUs only.
    Nvapi,
}

impl Component {
    pub const ALL: [Component; 3] = [Component::Dxvk, Component::Vkd3d, Component::Nvapi];
    /// Directory under a Valve Proton's `files/lib/wine/`.
    pub fn proton_dir(self) -> &'static str {
        match self {
            Component::Dxvk => "dxvk",
            Component::Vkd3d => "vkd3d-proton",
            Component::Nvapi => "nvapi",
        }
    }
    /// (64-bit, 32-bit) subdirectories of an upstream release tarball.
    pub fn release_dirs(self) -> (&'static str, &'static str) {
        match self {
            Component::Vkd3d => ("x64", "x86"),
            _ => ("x64", "x32"),
        }
    }
    /// Variable naming an extracted release (overrides Proton's copy).
    pub fn env(self) -> &'static str {
        match self {
            Component::Dxvk => "WOL_DXVK_DIR",
            Component::Vkd3d => "WOL_VKD3D_DIR",
            Component::Nvapi => "WOL_NVAPI_DIR",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub component: Component,
    pub x64: PathBuf,
    pub x32: PathBuf,
    /// Where it came from (log line).
    pub origin: String,
}

/// First source per component: `$WOL_*_DIR` (release layout), then each Valve Proton directory
/// (`files/lib/wine/<c>/{x86_64,i386}-windows`), in order.
pub fn find(
    var: &dyn Fn(&str) -> Option<String>,
    proton_dirs: &[PathBuf],
    exists: &dyn Fn(&Path) -> bool,
) -> Vec<Source> {
    Component::ALL
        .iter()
        .filter_map(|&c| {
            let from_env = var(c.env()).map(|d| {
                let (a, b) = c.release_dirs();
                let d = PathBuf::from(d);
                Source {
                    component: c,
                    x64: d.join(a),
                    x32: d.join(b),
                    origin: d.to_string_lossy().into_owned(),
                }
            });
            from_env.filter(|s| exists(&s.x64)).or_else(|| {
                proton_dirs.iter().find_map(|p| {
                    let d = p.join("files/lib/wine").join(c.proton_dir());
                    exists(&d.join("x86_64-windows")).then(|| Source {
                        component: c,
                        x64: d.join("x86_64-windows"),
                        x32: d.join("i386-windows"),
                        origin: p
                            .file_name()
                            .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
                    })
                })
            })
        })
        .collect()
}

/// (from, to) copies into the prefix and the DLL names to load native-first.
pub fn plan(
    sources: &[Source],
    list: &dyn Fn(&Path) -> Vec<PathBuf>,
    prefix: &Path,
) -> (Vec<(PathBuf, PathBuf)>, Vec<String>) {
    let win = prefix.join("drive_c/windows");
    let mut copies = Vec::new();
    let mut names = Vec::new();
    for s in sources {
        for (dir, dest) in [(&s.x64, "system32"), (&s.x32, "syswow64")] {
            for f in list(dir) {
                let Some(file) = f.file_name().map(|n| n.to_string_lossy().to_lowercase()) else {
                    continue;
                };
                let Some(stem) = file.strip_suffix(".dll") else {
                    continue;
                };
                names.push(stem.to_owned());
                copies.push((f.clone(), win.join(dest).join(&file)));
            }
        }
    }
    names.sort();
    names.dedup();
    (copies, names)
}

/// `WINEDLLOVERRIDES`: ours native-first, the user's after (later entries win).
pub fn overrides(names: &[String], user: Option<&str>) -> Option<String> {
    let ours = (!names.is_empty()).then(|| format!("{}=n,b", names.join(",")));
    match (ours, user.filter(|u| !u.is_empty())) {
        (Some(o), Some(u)) => Some(format!("{o};{u}")),
        (o, u) => o.or(u.map(String::from)),
    }
}

/// Copy where the destination is missing, a different size, or older. Returns files copied.
pub fn copy_changed(copies: &[(PathBuf, PathBuf)]) -> std::io::Result<usize> {
    let mut n = 0;
    for (from, to) in copies {
        let src = std::fs::metadata(from)?;
        let fresh = std::fs::metadata(to).is_ok_and(|d| {
            d.len() == src.len()
                && matches!((d.modified(), src.modified()), (Ok(a), Ok(b)) if a >= b)
        });
        if fresh {
            continue;
        }
        if let Some(dir) = to.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = to.with_extension("dll.wol-tmp");
        std::fs::copy(from, &tmp)?;
        std::fs::rename(&tmp, to)?;
        n += 1;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_env_first_then_proton() {
        let proton = PathBuf::from("/s/steamapps/common/Proton 10.0");
        let present = [
            "/opt/dxvk-3.1.1/x64",
            "/s/steamapps/common/Proton 10.0/files/lib/wine/dxvk/x86_64-windows",
            "/s/steamapps/common/Proton 10.0/files/lib/wine/vkd3d-proton/x86_64-windows",
        ];
        let exists = |p: &Path| present.iter().any(|x| Path::new(x) == p);
        let var = |k: &str| (k == "WOL_DXVK_DIR").then(|| "/opt/dxvk-3.1.1".to_string());
        let got = find(&var, std::slice::from_ref(&proton), &exists);
        let summary: Vec<(Component, &str, &str)> = got
            .iter()
            .map(|s| {
                (
                    s.component,
                    s.origin.as_str(),
                    s.x32.file_name().unwrap().to_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (Component::Dxvk, "/opt/dxvk-3.1.1", "x32"),
                (Component::Vkd3d, "Proton 10.0", "i386-windows"),
            ]
        );
        let none = find(&|_| None, &[], &exists);
        assert!(none.is_empty());
        let stale = |k: &str| (k == "WOL_DXVK_DIR").then(|| "/gone".to_string());
        assert_eq!(
            find(&stale, std::slice::from_ref(&proton), &exists)[0].origin,
            "Proton 10.0",
            "a missing WOL_DXVK_DIR falls back"
        );
    }

    #[test]
    fn copies_and_overrides() {
        let s = Source {
            component: Component::Dxvk,
            x64: "/d/x64".into(),
            x32: "/d/x32".into(),
            origin: "d".into(),
        };
        let list = |d: &Path| -> Vec<PathBuf> {
            ["D3D11.dll", "dxgi.dll", "README"]
                .iter()
                .map(|f| d.join(f))
                .collect()
        };
        let (copies, names) = plan(&[s], &list, Path::new("/p/pfx"));
        assert_eq!(names, ["d3d11", "dxgi"]);
        assert_eq!(
            copies[0],
            (
                PathBuf::from("/d/x64/D3D11.dll"),
                PathBuf::from("/p/pfx/drive_c/windows/system32/d3d11.dll")
            )
        );
        assert_eq!(
            copies[3].1,
            Path::new("/p/pfx/drive_c/windows/syswow64/dxgi.dll")
        );
        assert_eq!(copies.len(), 4);
        for (user, want) in [
            (None, Some("d3d11,dxgi=n,b")),
            (Some("winhttp=n,b"), Some("d3d11,dxgi=n,b;winhttp=n,b")),
            (Some(""), Some("d3d11,dxgi=n,b")),
        ] {
            assert_eq!(overrides(&names, user).as_deref(), want, "{user:?}");
        }
        assert_eq!(overrides(&[], Some("a=b")).as_deref(), Some("a=b"));
        assert_eq!(overrides(&[], None), None);
    }

    #[test]
    fn copy_only_what_changed() {
        let d = std::env::temp_dir().join(format!("wol-dlls-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("src/d3d11.dll"), "v1").unwrap();
        let c = vec![(d.join("src/d3d11.dll"), d.join("pfx/system32/d3d11.dll"))];
        assert_eq!(copy_changed(&c).unwrap(), 1);
        assert_eq!(copy_changed(&c).unwrap(), 0);
        std::fs::write(d.join("src/d3d11.dll"), "v2 longer").unwrap();
        assert_eq!(copy_changed(&c).unwrap(), 1);
        assert_eq!(
            std::fs::read_to_string(d.join("pfx/system32/d3d11.dll")).unwrap(),
            "v2 longer"
        );
        std::fs::remove_dir_all(d).unwrap();
    }
}

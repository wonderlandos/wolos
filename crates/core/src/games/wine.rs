//! wine-proton (emulators/wine-proton): Valve's Proton Wine built as a native FreeBSD program.
//! Layouts differ by release: 10.x ships `bin/wine64` (64-bit), `bin/wine` (32-bit, needs the
//! i386 package root) and `bin-wow64/wine`; 11.x is a WoW64 build with one `bin/wine`.
use std::path::{Path, PathBuf};

pub const ROOT: &str = "/usr/local/wine-proton";
/// Under `$HOME`: 32-bit wine-proton installed by `bin/pkg32.sh` (10.x only; FreeBSD 15 builds no
/// i386 packages).
pub const I386_ROOT: &str = ".i386-wine-pkg";

/// `wine --version` major.minor (`wine-10.0`, `wine-11.0 (Staging)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
}

impl Version {
    pub fn parse(text: &str) -> Option<Self> {
        let v = text.trim().strip_prefix("wine-")?;
        let v = v.split([' ', '-']).next()?;
        let mut it = v.split('.');
        let major = it.next()?.parse().ok()?;
        let minor = it.next().map_or(Some(0), |m| m.parse().ok())?;
        Some(Self { major, minor })
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

/// Machine of a PE executable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeArch {
    I386,
    Amd64,
    Other,
}

/// PE machine from the first bytes of a file (512 are enough for normal headers). None: not PE.
pub fn pe_arch(head: &[u8]) -> Option<PeArch> {
    if head.get(..2)? != b"MZ" {
        return None;
    }
    let off = u32::from_le_bytes(head.get(0x3c..0x40)?.try_into().ok()?) as usize;
    if head.get(off..off + 4)? != b"PE\0\0" {
        return None;
    }
    let machine = u16::from_le_bytes(head.get(off + 4..off + 6)?.try_into().ok()?);
    Some(match machine {
        0x14c => PeArch::I386,
        0x8664 => PeArch::Amd64,
        _ => PeArch::Other,
    })
}

/// The loader that runs a program of machine `pe` (None = unknown, treated as 64-bit).
/// 11.x: `bin/wine` for everything (WoW64). 10.x: `bin/wine64`; 32-bit programs use the i386
/// package when installed, else the WoW64 loader in `bin-wow64`.
pub fn loader(
    exists: &dyn Fn(&Path) -> bool,
    root: &Path,
    home: &Path,
    pe: Option<PeArch>,
) -> Option<PathBuf> {
    let wine = root.join("bin/wine");
    let wine64 = root.join("bin/wine64");
    if !exists(&wine64) {
        return exists(&wine).then_some(wine);
    }
    if pe != Some(PeArch::I386) {
        return Some(wine64);
    }
    let rel = root.strip_prefix("/").unwrap_or(root);
    let i386 = home.join(I386_ROOT).join(rel).join("bin/wine");
    let wow64 = root.join("bin-wow64/wine");
    Some(if exists(&i386) || !exists(&wow64) {
        wine
    } else {
        wow64
    })
}

/// `wineserver` that belongs to a loader (same directory).
pub fn wineserver(loader: &Path) -> PathBuf {
    loader.with_file_name("wineserver")
}

/// Synchronisation wine-proton uses on FreeBSD.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Sync {
    /// eventfd objects (`WINEESYNC=1`), 10.x and older.
    Esync,
    /// wineserver round trips: 11.x removed esync, and its fsync (futex_waitv) and ntsync paths
    /// compile only on Linux.
    Server,
}

pub fn sync(v: Version) -> Sync {
    if v.major >= 11 {
        Sync::Server
    } else {
        Sync::Esync
    }
}

/// How 32-bit Windows programs run for this release.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Win32 {
    /// 11.x: built in, no 32-bit host libraries.
    Wow64,
    /// 10.x with the i386 package at `~/.i386-wine-pkg`.
    I386Package,
    /// 10.x without it: `bin-wow64` (experimental on FreeBSD).
    Wow64Experimental,
    Missing,
}

pub fn win32(v: Version, has_i386_package: bool, has_bin_wow64: bool) -> Win32 {
    match () {
        _ if v.major >= 11 => Win32::Wow64,
        _ if has_i386_package => Win32::I386Package,
        _ if has_bin_wow64 => Win32::Wow64Experimental,
        _ => Win32::Missing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        let v = |a, b| Some(Version { major: a, minor: b });
        for (text, want) in [
            ("wine-10.0\n", v(10, 0)),
            ("wine-11.0 (Staging)", v(11, 0)),
            ("wine-9.0-rc1", v(9, 0)),
            ("wine-11", v(11, 0)),
            ("wine-x.y", None),
            ("10.0", None),
            ("", None),
        ] {
            assert_eq!(Version::parse(text), want, "{text:?}");
        }
        assert_eq!(v(10, 0).unwrap().to_string(), "10.0");
        assert!(v(10, 0) < v(11, 0));
    }

    fn pe(machine: u16) -> Vec<u8> {
        let mut b = vec![0u8; 0x100];
        b[..2].copy_from_slice(b"MZ");
        b[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        b[0x80..0x84].copy_from_slice(b"PE\0\0");
        b[0x84..0x86].copy_from_slice(&machine.to_le_bytes());
        b
    }

    #[test]
    fn pe_machines() {
        assert_eq!(pe_arch(&pe(0x14c)), Some(PeArch::I386));
        assert_eq!(pe_arch(&pe(0x8664)), Some(PeArch::Amd64));
        assert_eq!(pe_arch(&pe(0xaa64)), Some(PeArch::Other));
        assert_eq!(pe_arch(b"\x7fELF"), None);
        assert_eq!(pe_arch(&pe(0x14c)[..0x60]), None, "truncated header");
        let mut bad = pe(0x14c);
        bad[0x80] = b'X';
        assert_eq!(pe_arch(&bad), None);
    }

    #[test]
    fn loader_per_layout_and_machine() {
        let root = Path::new(ROOT);
        let home = Path::new("/home/u");
        let i386 = "/home/u/.i386-wine-pkg/usr/local/wine-proton/bin/wine";
        let w = |s: &str| Some(PathBuf::from(format!("{ROOT}/{s}")));
        use PeArch::*;
        // (files present, program machine, loader)
        let cases: [(&[&str], Option<PeArch>, Option<PathBuf>); 8] = [
            (&["bin/wine"], Some(Amd64), w("bin/wine")),
            (&["bin/wine"], Some(I386), w("bin/wine")),
            (&["bin/wine", "bin/wine64"], Some(Amd64), w("bin/wine64")),
            (&["bin/wine", "bin/wine64"], None, w("bin/wine64")),
            (
                &["bin/wine", "bin/wine64", "bin-wow64/wine"],
                Some(I386),
                w("bin-wow64/wine"),
            ),
            (
                &["bin/wine", "bin/wine64", "bin-wow64/wine", i386],
                Some(I386),
                w("bin/wine"),
            ),
            (&["bin/wine", "bin/wine64"], Some(I386), w("bin/wine")),
            (&[], Some(Amd64), None),
        ];
        for (files, machine, want) in cases {
            let exists = |p: &Path| {
                files.iter().any(|f| {
                    let full = if f.starts_with('/') {
                        PathBuf::from(f)
                    } else {
                        root.join(f)
                    };
                    full == p
                })
            };
            assert_eq!(
                loader(&exists, root, home, machine),
                want,
                "{files:?} {machine:?}"
            );
        }
        assert_eq!(
            wineserver(Path::new("/usr/local/wine-proton/bin-wow64/wine")),
            Path::new("/usr/local/wine-proton/bin-wow64/wineserver")
        );
    }

    #[test]
    fn sync_and_32_bit_per_release() {
        let v = |a| Version { major: a, minor: 0 };
        assert_eq!(sync(v(9)), Sync::Esync);
        assert_eq!(sync(v(10)), Sync::Esync);
        assert_eq!(sync(v(11)), Sync::Server);
        for (ver, pkg, wow, want) in [
            (11, false, false, Win32::Wow64),
            (10, true, true, Win32::I386Package),
            (10, false, true, Win32::Wow64Experimental),
            (10, false, false, Win32::Missing),
        ] {
            assert_eq!(win32(v(ver), pkg, wow), want);
        }
    }
}

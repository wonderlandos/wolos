//! wine on Arch: the native `wine` package (WoW64: one `/usr/bin/wine` runs 32- and 64-bit
//! programs) for Windows programs outside Steam, or umu-launcher when wine is not installed.
//! Steam games use the Proton that Steam installs; nothing here touches them.
pub const BIN: &str = "/usr/bin/wine";
/// umu-launcher (multilib): runs a program on Proton without the Steam client.
pub const UMU_BIN: &str = "/usr/bin/umu-run";
/// Game id umu uses when no store id applies (no per-game fixes).
pub const UMU_GAME_ID: &str = "umu-default";

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
}

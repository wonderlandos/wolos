// SPDX-License-Identifier: BSD-2-Clause
//! "Give me more time" (Settings > Accessibility > Timeouts, QA-ACC-6): one multiplier for every
//! wait the desktop puts on the person, kept in `$XDG_CONFIG_HOME/wol/timeouts` as `2`, `5` or
//! `never` (no file, or anything else: normal). The shell stretches its approval sheets, banners,
//! toasts, offers and countdowns by it; `wol` and wold stretch the waits behind those sheets by the
//! same figure, so a sheet that is still open is never answered by a requester that gave up.
use std::path::Path;
use std::time::Duration;

/// The file, relative to `$XDG_CONFIG_HOME`.
pub const FILE: &str = "wol/timeouts";
/// The longest any one wait lasts, `never` included (seconds): nothing waits longer than an hour
/// for a sheet nobody is looking at.
pub const NEVER_SECS: u32 = 3600;
/// The most of the file read (bytes).
const MAX_FILE: u64 = 32;

/// How long the account asked the desktop to wait on it: the multiplier of Give me more time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Patience {
    /// The waits as designed.
    #[default]
    Normal,
    /// Twice as long.
    Double,
    /// Five times as long.
    Five,
    /// Banners and toasts stay until dismissed; sheets wait [`NEVER_SECS`].
    Never,
}

impl Patience {
    /// Every choice, in the order Settings lists them.
    pub const ALL: [Patience; 4] = [
        Patience::Normal,
        Patience::Double,
        Patience::Five,
        Patience::Never,
    ];

    /// The file's text: `2`, `5`, `never` (also `x2`, `x5`, with spaces or a final newline).
    pub fn parse(text: &str) -> Patience {
        let word = text.trim().to_ascii_lowercase();
        let word = word.trim_start_matches('x');
        Patience::ALL
            .into_iter()
            .find(|p| !word.is_empty() && p.word() == word)
            .unwrap_or_default()
    }

    /// What the file holds; empty for normal (the file is removed instead).
    pub fn word(self) -> &'static str {
        match self {
            Patience::Normal => "",
            Patience::Double => "2",
            Patience::Five => "5",
            Patience::Never => "never",
        }
    }

    /// The multiplier; None = no limit.
    pub fn factor(self) -> Option<u32> {
        match self {
            Patience::Normal => Some(1),
            Patience::Double => Some(2),
            Patience::Five => Some(5),
            Patience::Never => None,
        }
    }

    /// `base` seconds stretched, never past [`NEVER_SECS`] (a request is not held for an hour
    /// because five times its time is more).
    pub fn secs(self, base: u32) -> u32 {
        match self.factor() {
            Some(f) => base.saturating_mul(f).min(NEVER_SECS.max(base)),
            None => NEVER_SECS.max(base),
        }
    }

    /// `base` seconds stretched for a wait that must end soon (a display change that is put back
    /// when nobody keeps it): the factor applies, and `never` and the stretch stop at `cap`
    /// (at least `base`).
    pub fn capped_secs(self, base: u32, cap: u32) -> u32 {
        let cap = cap.max(base);
        match self.factor() {
            Some(f) => base.saturating_mul(f).min(cap),
            None => cap,
        }
    }

    /// A timer of `base`, stretched; None = no timer (`never`).
    pub fn timer(self, base: Duration) -> Option<Duration> {
        self.factor().map(|f| base.saturating_mul(f))
    }

    /// A timer of `base` that must end (a copied code leaves the clipboard): stretched, and for
    /// `never` the longest wait, [`NEVER_SECS`] / 6 = ten minutes.
    pub fn bounded(self, base: Duration) -> Duration {
        self.timer(base)
            .unwrap_or(Duration::from_secs(u64::from(NEVER_SECS / 6)))
            .max(base)
    }

    /// The setting in `config_home` (the account's own `$XDG_CONFIG_HOME`).
    pub fn read(config_home: &Path) -> Patience {
        let Some(t) = crate::bounded::read_text(&config_home.join(FILE), MAX_FILE) else {
            return Patience::Normal;
        };
        Patience::parse(&t)
    }

    /// The setting of account `uid` from its file `path` for a process that runs as root (wold):
    /// opened without following a link or waiting on a pipe, and used only when it is a small
    /// regular file the account owns. Anything else is normal.
    pub fn read_owned(path: &Path, uid: u32) -> Patience {
        use std::io::Read;
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let Ok(f) = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(path)
        else {
            return Patience::Normal;
        };
        let Ok(m) = f.metadata() else {
            return Patience::Normal;
        };
        if !m.is_file() || m.uid() != uid || m.len() > MAX_FILE {
            return Patience::Normal;
        }
        let mut s = String::new();
        match f.take(MAX_FILE).read_to_string(&mut s) {
            Ok(_) => Patience::parse(&s),
            Err(_) => Patience::Normal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_says_two_five_or_never_and_anything_else_is_normal() {
        for (t, want) in [
            ("2\n", Patience::Double),
            (" x2 ", Patience::Double),
            ("5", Patience::Five),
            ("X5", Patience::Five),
            ("never\n", Patience::Never),
            ("Never", Patience::Never),
            ("", Patience::Normal),
            ("3", Patience::Normal),
            ("0", Patience::Normal),
            ("forever", Patience::Normal),
        ] {
            assert_eq!(Patience::parse(t), want, "{t:?}");
        }
        for p in Patience::ALL {
            assert_eq!(Patience::parse(p.word()), p);
        }
    }

    #[test]
    fn waits_stretch_and_stay_bounded() {
        assert_eq!(Patience::Normal.secs(50), 50);
        assert_eq!(Patience::Double.secs(50), 100);
        assert_eq!(Patience::Five.secs(50), 250);
        assert_eq!(Patience::Never.secs(50), NEVER_SECS);
        // A wait already past the bound is not cut.
        assert_eq!(Patience::Double.secs(4000), 4000);
        assert_eq!(Patience::Five.secs(900), 3600);
        assert_eq!(
            Patience::Normal.timer(Duration::from_secs(5)),
            Some(Duration::from_secs(5))
        );
        assert_eq!(
            Patience::Five.timer(Duration::from_secs(5)),
            Some(Duration::from_secs(25))
        );
        assert_eq!(Patience::Never.timer(Duration::from_secs(5)), None);
        // A wait that must end: stretched to the cap, never past it.
        assert_eq!(Patience::Normal.capped_secs(15, 120), 15);
        assert_eq!(Patience::Double.capped_secs(15, 120), 30);
        assert_eq!(Patience::Five.capped_secs(15, 120), 75);
        assert_eq!(Patience::Five.capped_secs(60, 120), 120);
        assert_eq!(Patience::Never.capped_secs(15, 120), 120);
        // The clipboard still empties.
        assert_eq!(
            Patience::Never.bounded(Duration::from_secs(60)),
            Duration::from_secs(600)
        );
        assert_eq!(
            Patience::Double.bounded(Duration::from_secs(60)),
            Duration::from_secs(120)
        );
    }

    #[test]
    fn a_root_reader_takes_only_a_small_regular_file_of_the_account() {
        use std::os::unix::fs::symlink;
        let d = std::env::temp_dir().join(format!("wol-patience-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let uid = unsafe { libc::getuid() };
        let f = d.join("timeouts");
        std::fs::write(&f, "5\n").unwrap();
        assert_eq!(Patience::read_owned(&f, uid), Patience::Five);
        // Another account's file, a link, a big file, a missing file, a pipe: normal.
        assert_eq!(Patience::read_owned(&f, uid + 1), Patience::Normal);
        let l = d.join("link");
        symlink(&f, &l).unwrap();
        assert_eq!(Patience::read_owned(&l, uid), Patience::Normal);
        let big = d.join("big");
        std::fs::write(&big, "5".repeat(64)).unwrap();
        assert_eq!(Patience::read_owned(&big, uid), Patience::Normal);
        assert_eq!(Patience::read_owned(&d.join("none"), uid), Patience::Normal);
        let fifo = d.join("fifo");
        let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        assert_eq!(Patience::read_owned(&fifo, uid), Patience::Normal);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn read_finds_the_file_under_the_config_home() {
        let d = std::env::temp_dir().join(format!("wol-patience-home-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("wol")).unwrap();
        assert_eq!(Patience::read(&d), Patience::Normal);
        std::fs::write(d.join(FILE), "never\n").unwrap();
        assert_eq!(Patience::read(&d), Patience::Never);
        let _ = std::fs::remove_dir_all(&d);
    }
}

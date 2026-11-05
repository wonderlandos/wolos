// SPDX-License-Identifier: BSD-2-Clause
//! The shell new terminal windows open in (Settings > General, "Shell in the terminal"; ADR 0032
//! "Console"). One word in `$XDG_CONFIG_HOME/`[`FILE`]; foot, ghostty and kitty start
//! [`LAUNCHER`], which reads it and starts that shell, or zsh when the word is unknown or the
//! shell is not installed. The account's login shell stays zsh (`settings::users::LOGIN_SHELL`):
//! the session and ssh hand it POSIX text, which pwsh, fish or nu would not run.
use std::path::{Path, PathBuf};

/// Relative to `$XDG_CONFIG_HOME`.
pub const FILE: &str = "wolos/terminal-shell";
/// What the terminals run (`desktop/overlay/usr/bin/wol-term-shell`).
pub const LAUNCHER: &str = "/usr/bin/wol-term-shell";
/// No file, an unknown word or a shell that is not installed.
pub const DEFAULT: &str = "zsh";
/// The programs, in menu order; a shell's word in [`FILE`] is its file name ([`id`]). `~/` is the
/// person's home: pwsh is installed per person by the Console module's recipe. [`LAUNCHER`] has
/// the same table (a test keeps the two equal).
pub const SHELLS: [&str; 6] = [
    "/usr/bin/zsh",
    "/usr/bin/bash",
    "/usr/bin/fish",
    "/usr/bin/nu",
    // The image ships rsh (QA-PWR-1: it was missing here, so the word fell back to zsh).
    "/usr/bin/rsh",
    "~/.local/bin/pwsh",
];

/// The word for one of [`SHELLS`]: its file name.
pub fn id(program: &'static str) -> &'static str {
    program.rsplit('/').next().unwrap_or(program)
}

/// Every word, in menu order.
pub fn ids() -> impl Iterator<Item = &'static str> {
    SHELLS.iter().map(|p| id(p))
}

/// The program for `word`, with `~/` resolved against `home`; None for an unknown word.
pub fn program(word: &str, home: &Path) -> Option<PathBuf> {
    let p = SHELLS.iter().find(|p| id(p) == word)?;
    Some(match p.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => PathBuf::from(p),
    })
}

/// The shells of [`SHELLS`] installed for the person with `home`, in menu order; `executable`
/// says whether a program file can be run (the caller reads the disk; this module does not).
pub fn installed(home: &Path, executable: impl Fn(&Path) -> bool) -> Vec<&'static str> {
    ids()
        .filter(|w| program(w, home).is_some_and(|p| executable(&p)))
        .collect()
}

/// The word saved in `text` (the file's content), if it is one of [`SHELLS`].
pub fn parse(text: &str) -> Option<&'static str> {
    let w = text.lines().next()?.trim();
    ids().find(|id| *id == w)
}

/// What a new terminal window opens: the saved word when that shell is in `installed`, else
/// [`DEFAULT`] (what [`LAUNCHER`] does).
pub fn effective(text: Option<&str>, installed: &[&str]) -> &'static str {
    text.and_then(parse)
        .filter(|id| installed.contains(id))
        .unwrap_or(DEFAULT)
}

/// The file's content for `id`.
pub fn render(id: &str) -> String {
    format!("{id}\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_programs_and_fallback() {
        let home = Path::new("/home/ana");
        assert_eq!(
            program("pwsh", home).unwrap(),
            Path::new("/home/ana/.local/bin/pwsh")
        );
        assert_eq!(program("zsh", home).unwrap(), Path::new("/usr/bin/zsh"));
        assert!(program("csh", home).is_none());
        assert_eq!(parse("pwsh\n"), Some("pwsh"));
        assert_eq!(parse("  fish  \nrest"), Some("fish"));
        assert_eq!(parse("tcsh\n"), None);
        assert_eq!(parse(""), None);
        let have = ["zsh", "bash", "pwsh"];
        assert_eq!(effective(Some("pwsh\n"), &have), "pwsh");
        assert_eq!(effective(Some("fish\n"), &have), "zsh", "not installed");
        assert_eq!(effective(Some("rm -rf\n"), &have), "zsh", "unknown");
        assert_eq!(effective(None, &have), "zsh");
        assert_eq!(render("nu"), "nu\n");
        assert_eq!(id(SHELLS[0]), DEFAULT);
        // QA-PWR-1: rsh ships on the image (rsh 1.0.0-1, /usr/bin/rsh); its word is a shell.
        assert_eq!(program("rsh", home).unwrap(), Path::new("/usr/bin/rsh"));
        assert_eq!(effective(Some("rsh\n"), &["zsh", "rsh"]), "rsh");
    }

    #[test]
    fn installed_asks_the_caller() {
        let home = Path::new("/h");
        let have = installed(home, |p| {
            p == Path::new("/usr/bin/zsh") || p == Path::new("/h/.local/bin/pwsh")
        });
        assert_eq!(have, ["zsh", "pwsh"]);
        assert_eq!(
            ids().collect::<Vec<_>>(),
            ["zsh", "bash", "fish", "nu", "rsh", "pwsh"]
        );
    }
}

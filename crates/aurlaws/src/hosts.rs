// SPDX-License-Identifier: GPL-2.0-or-later
//! Does a source URL belong to the project the PKGBUILD declares?

/// The lowercase host of a URL, without VCS prefix, credentials or port.
pub fn host(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let auth = rest.split(['/', '?', '#']).next()?;
    let h = auth.rsplit('@').next()?.split(':').next()?;
    (!h.is_empty()).then(|| h.to_ascii_lowercase())
}

fn registrable(h: &str) -> String {
    let parts: Vec<&str> = h.split('.').collect();
    let keep = if parts.len() >= 3
        && matches!(parts[parts.len() - 2], "co" | "com" | "org" | "net" | "ac")
        && parts[parts.len() - 1].len() == 2
    {
        3
    } else {
        2
    };
    parts[parts.len().saturating_sub(keep)..].join(".")
}

const CODE_HOSTS: &[&str] = &[
    "github.com",
    "gitlab.com",
    "codeberg.org",
    "bitbucket.org",
    "sr.ht",
    "git.sr.ht",
    "gitee.com",
];

/// `owner/repo` on a shared code host, where the host alone says nothing about the project.
fn owner(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let mut p = rest.split('/');
    p.next()?;
    p.next()
        .filter(|o| !o.is_empty())
        .map(|o| o.to_ascii_lowercase())
}

/// Registries and distro mirrors that host other projects' releases as their job.
const MIRRORS: &[&str] = &[
    "files.pythonhosted.org",
    "pypi.io",
    "pypi.org",
    "static.crates.io",
    "crates.io",
    "registry.npmjs.org",
    "registry.yarnpkg.com",
    "rubygems.org",
    "hackage.haskell.org",
    "proxy.golang.org",
    "cran.r-project.org",
    "sourceforge.net",
    "kernel.org",
    "gnu.org",
    "gnome.org",
    "kde.org",
    "freedesktop.org",
    "archlinux.org",
    "debian.org",
    "ubuntu.com",
    "fedoraproject.org",
];

/// May this source URL serve a package that declares `declared` as its home?
pub fn on_upstream(declared: &str, source: &str, pkgnames: &[String]) -> bool {
    let Some(h) = host(source) else { return true };
    if host(declared).is_none() || same_project(declared, source, pkgnames) {
        return true;
    }
    MIRRORS
        .iter()
        .any(|m| h == *m || h.ends_with(&format!(".{m}")))
}

/// The same site, or the same owner on a shared code host, or a repo named after the package.
pub fn same_project(declared: &str, source: &str, pkgnames: &[String]) -> bool {
    let (Some(a), Some(b)) = (host(declared), host(source)) else {
        return false;
    };
    let shared = CODE_HOSTS.iter().any(|c| registrable(c) == registrable(&b));
    if !shared {
        return registrable(&a) == registrable(&b);
    }
    if registrable(&a) == registrable(&b) && owner(declared) == owner(source) {
        return true;
    }
    let tail = source
        .split('#')
        .next()
        .unwrap_or("")
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("")
        .trim_end_matches(".git")
        .to_ascii_lowercase();
    pkgnames.iter().any(|n| {
        let core = n
            .trim_end_matches("-git")
            .trim_end_matches("-bin")
            .to_ascii_lowercase();
        tail == core
    })
}

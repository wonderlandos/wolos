// SPDX-License-Identifier: GPL-2.0-or-later
//! Does a source URL belong to the project the PKGBUILD declares?

/// The lowercase host of a URL, without VCS prefix, credentials or port.
pub fn host(url: &str) -> Option<String> {
    if url.starts_with("file://") {
        return None;
    }
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
    "gitee.com",
    "googlesource.com",
];

/// GitHub serves a repo's files from these too; the path still starts with owner and repo.
const ALIASES: &[&str] = &[
    "raw.githubusercontent.com",
    "codeload.github.com",
    "objects.githubusercontent.com",
    "patch-diff.githubusercontent.com",
    "api.github.com",
];

/// `(owner, repo)` on a shared code host, where the host alone says nothing about the project.
fn repo(url: &str) -> Option<(String, String)> {
    let rest = url.split_once("://")?.1;
    let mut p = rest.split(['/', '#', '?']);
    let h = p.next()?;
    let mut first = p.next();
    if matches!(
        (h, first),
        ("api.github.com", Some("repos")) | ("patch-diff.githubusercontent.com", Some("raw"))
    ) {
        first = p.next();
    }
    let owner = first
        .filter(|o| !o.is_empty())?
        .trim_start_matches('~')
        .to_ascii_lowercase();
    let name = p
        .next()
        .unwrap_or("")
        .trim_end_matches(".git")
        .to_ascii_lowercase();
    Some((owner, name))
}

fn canonical(h: &str) -> &str {
    if ALIASES.contains(&h) {
        "github.com"
    } else {
        h
    }
}

fn on_code_host(h: &str) -> bool {
    let r = registrable(h);
    ALIASES.contains(&h) || CODE_HOSTS.iter().any(|c| registrable(c) == r)
}

fn core(name: &str) -> String {
    let n = name.trim_end_matches("-git").trim_end_matches("-bin");
    ["ttf-", "otf-", "python-", "lib32-"]
        .iter()
        .find_map(|p| n.strip_prefix(p))
        .unwrap_or(n)
        .to_ascii_lowercase()
}

/// Words too common to tie a package to anyone.
const VAGUE: &[&str] = &[
    "alpha", "beta", "browser", "client", "code", "desktop", "dkms", "drive", "extra", "fonts",
    "font", "free", "linux", "lib32", "nightly", "nodejs", "open", "origin", "patch", "patched",
    "perl", "python", "release", "ruby", "rust", "server", "stable", "tools", "utils",
];

/// The words of a package's names that could name its maker.
pub fn words(pkgnames: &[String]) -> Vec<String> {
    let mut out: Vec<String> = pkgnames
        .iter()
        .flat_map(|n| n.split(['-', '_', '.']))
        .filter(|w| w.len() >= 4 && !VAGUE.contains(w))
        .map(str::to_ascii_lowercase)
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The words of the declared home: `brave` in brave.com, `sinelaw` in sinelaw.github.io.
fn home_words(h: &str) -> Vec<String> {
    let skip = [
        "www",
        "docs",
        "developer",
        "developers",
        "download",
        "downloads",
        "apps",
        "github",
        "gitlab",
        "codeberg",
        "sourceforge",
        "netlify",
        "pages",
    ];
    let site = registrable(h);
    let name = site.split('.').next().unwrap_or("");
    h.split('.')
        .filter(|l| (l.len() >= 4 || *l == name) && !skip.contains(l))
        .map(str::to_owned)
        .collect()
}

/// The project a shared hosting page names: `enlightenment` in sourceforge.net/projects/enlightenment.
fn hosted_project(declared: &str) -> Option<String> {
    let path = declared.split_once("://")?.1;
    let rest = path.split_once("sourceforge.net/projects/")?.1;
    let p = rest.split(['/', '?', '#']).next()?;
    (!p.is_empty()).then(|| p.to_ascii_lowercase())
}

/// A repo's public pages live on `owner.gitlab.io`.
fn pages_owner(h: &str) -> Option<&str> {
    h.strip_suffix(".gitlab.io")
        .or_else(|| h.strip_suffix(".github.io"))
        .filter(|o| !o.contains('.'))
}

/// `host/path` of a repository address, for comparing two ways of writing it.
fn repo_path(url: &str) -> String {
    let u = url.split_once("://").map(|x| x.1).unwrap_or(url);
    let u = u.rsplit('@').next().unwrap_or(u);
    u.split(['#', '?'])
        .next()
        .unwrap_or(u)
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .to_ascii_lowercase()
}

/// Is this source one of the submodules upstream's own `.gitmodules` lists?
pub fn named_by_upstream(submodules: &[String], source: &str) -> bool {
    let want = repo_path(source);
    submodules.iter().any(|s| repo_path(s) == want)
}

fn squash(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
}

/// Same maker by name alone, between two sides that each know a little: exact for an owner on a
/// shared host, where a name is free to take, and looser only once a pin has fixed the bytes.
fn kin(name: &str, home: &[String], pkg: &[String], pinned: bool) -> bool {
    let x = squash(name);
    let plain = name.to_ascii_lowercase();
    let exact = home.iter().chain(pkg).any(|w| x == *w || x == squash(w));
    let close = home
        .iter()
        .filter(|w| w.len() >= 4)
        .any(|w| x.starts_with(w.as_str()) || (x.len() >= 5 && w.starts_with(x.as_str())))
        || home
            .iter()
            .chain(pkg)
            .any(|w| w.len() >= 5 && x.contains(w.as_str()))
        || pkg
            .iter()
            .any(|w| x.starts_with(w.as_str()) || plain.split('-').any(|t| t == w));
    exact || (pinned && close)
}

/// A licence, a document or a signature: read, never run, unless a law sees it run.
pub fn paperwork(name: &str) -> bool {
    let n = name
        .split(['?', '#'])
        .next()
        .unwrap_or(name)
        .rsplit('/')
        .next()
        .unwrap_or(name)
        .to_ascii_lowercase();
    ["license", "licence", "copying", "notice", "readme"]
        .iter()
        .any(|p| n.starts_with(p))
        || [
            ".md", ".rst", ".txt", ".pdf", ".rtf", ".html", ".asc", ".sig", ".sign",
        ]
        .iter()
        .any(|e| n.ends_with(e))
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
    "sf.net",
    "gentoo.org",
    "kernel.org",
    "gnu.org",
    "gnome.org",
    "kde.org",
    "freedesktop.org",
    "archlinux.org",
    "debian.org",
    "debian.net",
    "ubuntu.com",
    "fedoraproject.org",
];

/// May this source URL serve a package that declares `declared` as its home?
/// On the declared host under another owner counts only when a checksum or commit pins it: the
/// host is right, and the pin says the bytes are the ones the maintainer saw.
pub fn on_upstream(declared: &str, source: &str, pkgnames: &[String], pinned: bool) -> bool {
    let Some(h) = host(source) else { return true };
    if host(declared).is_none() || same_project(declared, source, pkgnames) {
        return true;
    }
    if pinned && host(declared).is_some_and(|d| canonical(&d) == canonical(&h)) {
        return true;
    }
    if related(declared, source, pkgnames, pinned) {
        return true;
    }
    MIRRORS
        .iter()
        .any(|m| h == *m || h.ends_with(&format!(".{m}")))
}

/// The maker's own account on a shared host, or the maker's own second domain when pinned.
fn related(declared: &str, source: &str, pkgnames: &[String], pinned: bool) -> bool {
    let (Some(a), Some(b)) = (host(declared), host(source)) else {
        return false;
    };
    let (mut home, pkg) = (home_words(&a), words(pkgnames));
    home.extend(hosted_project(declared));
    if pages_owner(&b).is_some_and(|o| repo(declared).is_some_and(|d| d.0 == o)) {
        return true;
    }
    if on_code_host(&b) {
        return repo(source).is_some_and(|(owner, name)| {
            kin(&owner, &home, &pkg, pinned)
                || home.contains(&name)
                || pinned && named_by_maker(&owner, &name, &pkg)
        });
    }
    let site = registrable(&b);
    // The same name under another ending is how a look-alike is made, so that alone is not kin.
    let label = site.split('.').next().unwrap_or("");
    if pinned && hosted_project(declared).is_some_and(|p| p == label) {
        return true;
    }
    pinned && !home.iter().any(|w| w == label) && kin(label, &home, &pkg, true)
        || pinned && pkg.iter().any(|w| w == label)
}

/// `nordsecurity/nordvpn-linux` for nordvpn: the repo carries the product's word and the account
/// starts the same way. A stranger's `brave-patch` has the word and not the account.
fn named_by_maker(owner: &str, repo: &str, pkg: &[String]) -> bool {
    repo.split('-').any(|t| {
        pkg.iter()
            .any(|w| w == t && w.len() >= 4 && owner.starts_with(&w[..4]))
    })
}

/// The same site, or the same owner on a shared code host, or a repo named after the package.
/// A file's name proves nothing, because anyone can name a file anything.
pub fn same_project(declared: &str, source: &str, pkgnames: &[String]) -> bool {
    let (Some(a), Some(b)) = (host(declared), host(source)) else {
        return false;
    };
    if !on_code_host(&b) {
        return registrable(&a) == registrable(&b);
    }
    let Some((owner, name)) = repo(source) else {
        return false;
    };
    if on_code_host(&a) && repo(declared).is_some_and(|d| d.0 == owner) {
        return true;
    }
    !name.is_empty()
        && pkgnames
            .iter()
            .any(|n| core(n) == name || core(n).starts_with(&format!("{name}-")))
}

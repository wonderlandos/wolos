//! AppImages as first-class apps: adopt or install into `~/Applications`, read the embedded update
//! information (`.upd_info`) and signature (`.sha256_sig`, `.sig_key`), update by zsync delta,
//! keep the previous file for rollback. Runs as the person; the AppImage itself is never executed.
use crate::context::Context;
use crate::ffi::{Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

/// ELF section name -> (file offset, size).
pub fn sections(b: &[u8]) -> Option<BTreeMap<String, (usize, usize)>> {
    if b.get(..4)? != b"\x7fELF" || *b.get(4)? != 2 || *b.get(5)? != 1 {
        return None; // 64-bit little-endian only (x86_64, aarch64)
    }
    let u16_at = |o: usize| Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?) as usize);
    let u32_at = |o: usize| Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?) as usize);
    let u64_at = |o: usize| Some(u64::from_le_bytes(b.get(o..o + 8)?.try_into().ok()?) as usize);
    let shoff = u64_at(0x28)?;
    let (shent, shnum, shstr) = (u16_at(0x3a)?, u16_at(0x3c)?, u16_at(0x3e)?);
    if shent < 64 || shnum > 4096 {
        return None;
    }
    let sh = |i: usize| shoff.checked_add(i.checked_mul(shent)?);
    let str_off = u64_at(sh(shstr)? + 0x18)?;
    let mut out = BTreeMap::new();
    for i in 0..shnum {
        let h = sh(i)?;
        let name_at = str_off.checked_add(u32_at(h)?)?;
        let name_end = name_at + b.get(name_at..)?.iter().position(|c| *c == 0)?;
        let name = String::from_utf8_lossy(&b[name_at..name_end]).into_owned();
        out.insert(name, (u64_at(h + 0x18)?, u64_at(h + 0x20)?));
    }
    Some(out)
}

/// Type 2 AppImage: ELF with `AI\x02` at offset 8.
pub fn is_appimage(b: &[u8]) -> bool {
    b.starts_with(b"\x7fELF") && b.get(8..11) == Some(b"AI\x02")
}

/// Section contents up to the first NUL.
fn section_text(b: &[u8], s: &BTreeMap<String, (usize, usize)>, name: &str) -> Option<String> {
    let (o, n) = *s.get(name)?;
    let raw = b.get(o..o.checked_add(n)?)?;
    let end = raw.iter().position(|c| *c == 0).unwrap_or(raw.len());
    let t = String::from_utf8_lossy(&raw[..end]).trim().to_owned();
    (!t.is_empty()).then_some(t)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum UpdateInfo {
    Zsync {
        url: String,
    },
    GhReleasesZsync {
        owner: String,
        repo: String,
        tag: String,
        file: String,
    },
}

impl UpdateInfo {
    /// The AppImage update information string (`zsync|URL`, `gh-releases-zsync|o|r|tag|glob`).
    pub fn parse(s: &str) -> Option<UpdateInfo> {
        let f: Vec<&str> = s.trim().split('|').collect();
        match f[..] {
            ["zsync", url] => Some(UpdateInfo::Zsync { url: url.into() }),
            ["gh-releases-zsync", o, r, t, file]
                if [o, r, t, file]
                    .iter()
                    .all(|x| !x.is_empty() && !x.contains('/')) =>
            {
                Some(UpdateInfo::GhReleasesZsync {
                    owner: o.into(),
                    repo: r.into(),
                    tag: t.into(),
                    file: file.into(),
                })
            }
            _ => None,
        }
    }
}

/// `*` wildcard match (AppImage file globs).
pub fn glob(pat: &str, s: &str) -> bool {
    let parts: Vec<&str> = pat.split('*').collect();
    if parts.len() == 1 {
        return pat == s;
    }
    let mut rest = s;
    for (i, p) in parts.iter().enumerate() {
        if i == 0 {
            let Some(r) = rest.strip_prefix(p) else {
                return false;
            };
            rest = r;
        } else if i == parts.len() - 1 {
            return rest.ends_with(p);
        } else {
            let Some(at) = rest.find(p) else { return false };
            rest = &rest[at + p.len()..];
        }
    }
    true
}

/// What wopkg learns from an AppImage without running it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Inspect {
    pub appimage: bool,
    pub sha256: String,
    pub sha1: String,
    pub size: u64,
    pub update: Option<UpdateInfo>,
    pub update_raw: Option<String>,
    pub signed: bool,
    /// SHA-256 of the embedded public key (pinned at install).
    pub key_pin: Option<String>,
    /// The digest the embedded signature signs (hex SHA-256 with the signature sections zeroed).
    pub signed_digest: String,
}

pub fn inspect(path: &Path) -> Result<Inspect> {
    let b = std::fs::read(path).map_err(|_| {
        Error::new(
            crate::ffi::Status::NotFound,
            "That AppImage cannot be read.",
        )
    })?;
    let s = sections(&b).unwrap_or_default();
    let sig = section_text(&b, &s, ".sha256_sig");
    let key = section_text(&b, &s, ".sig_key");
    let mut zeroed = b.clone();
    for n in [".sha256_sig", ".sig_key"] {
        if let Some(&(o, len)) = s.get(n)
            && let Some(r) = zeroed.get_mut(o..o + len)
        {
            r.fill(0);
        }
    }
    let raw = section_text(&b, &s, ".upd_info");
    Ok(Inspect {
        appimage: is_appimage(&b),
        sha256: crate::fetch::hex(&Sha256::digest(&b)),
        sha1: crate::fetch::hex(&sha1::Sha1::digest(&b)),
        size: b.len() as u64,
        update: raw.as_deref().and_then(UpdateInfo::parse),
        update_raw: raw,
        signed: sig.is_some() && key.is_some(),
        key_pin: key.map(|k| crate::fetch::hex(&Sha256::digest(k.as_bytes()))),
        signed_digest: crate::fetch::hex(&Sha256::digest(&zeroed)),
    })
}

/// Check the embedded signature with gpgv against the embedded key (only after the key matched the
/// pin). The key goes into a throwaway keyring; nothing touches the person's GnuPG home.
pub fn verify_signature(path: &Path, info: &Inspect, pin: Option<&str>) -> Result<bool> {
    match (info.signed, pin) {
        (false, None) => return Ok(false),
        (false, Some(_)) => {
            return Err(Error::untrusted(
                "This update is not signed, but the app was. It was not installed.",
            ));
        }
        (true, Some(p)) if info.key_pin.as_deref() != Some(p) => {
            return Err(Error::untrusted(
                "This update is signed with a different key than the installed app. It was not installed.",
            ));
        }
        _ => {}
    }
    let b = std::fs::read(path).map_err(|_| Error::unavailable("That AppImage cannot be read."))?;
    let s = sections(&b).unwrap_or_default();
    let (sig, key) = (
        section_text(&b, &s, ".sha256_sig").unwrap_or_default(),
        section_text(&b, &s, ".sig_key").unwrap_or_default(),
    );
    let dir = std::env::temp_dir().join(format!("wopkg-gpg-{}-{}", std::process::id(), info.sha1));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)
        .map_err(|_| Error::unavailable("The signature cannot be checked."))?;
    let w = |n: &str, t: &[u8]| std::fs::write(dir.join(n), t);
    w("key.asc", key.as_bytes())
        .and_then(|_| w("sig.asc", sig.as_bytes()))
        .and_then(|_| w("digest", info.signed_digest.as_bytes()))
        .map_err(|_| Error::unavailable("The signature cannot be checked."))?;
    let run = |argv: &[&str]| {
        std::process::Command::new(argv[0])
            .args(&argv[1..])
            .env("GNUPGHOME", &dir)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    };
    let d = |n: &str| dir.join(n).to_string_lossy().into_owned();
    let ok = run(&[
        "gpg",
        "--batch",
        "--yes",
        "--dearmor",
        "-o",
        &d("key.gpg"),
        &d("key.asc"),
    ]) && run(&[
        "gpgv",
        "--keyring",
        &d("key.gpg"),
        &d("sig.asc"),
        &d("digest"),
    ]);
    let _ = std::fs::remove_dir_all(&dir);
    if ok {
        Ok(true)
    } else {
        Err(Error::untrusted(
            "The AppImage's signature does not match. It was not installed.",
        ))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct App {
    pub name: String,
    pub path: PathBuf,
    pub sha256: String,
    #[serde(default)]
    pub update: Option<String>,
    #[serde(default)]
    pub key_pin: Option<String>,
    #[serde(default)]
    pub signed: bool,
    /// Where it came from when installed by URL.
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Registry {
    pub apps: Vec<App>,
}

impl Registry {
    fn path(ctx: &Context) -> PathBuf {
        ctx.state.join("appimages.json")
    }
    pub fn load(ctx: &Context) -> Registry {
        std::fs::read(Self::path(ctx))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    pub fn save(&self, ctx: &Context) -> Result<()> {
        crate::journal::write_atomic(
            &Self::path(ctx),
            serde_json::to_string_pretty(self)
                .unwrap_or_default()
                .as_bytes(),
        )
    }
    pub fn upsert(&mut self, a: App) {
        self.apps.retain(|x| x.name != a.name);
        self.apps.push(a);
        self.apps.sort_by(|a, b| a.name.cmp(&b.name));
    }
}

/// `Foo-1.2.3-x86_64.AppImage` -> `Foo`.
pub fn name_from_file(file: &str) -> String {
    let stem = file
        .strip_suffix(".AppImage")
        .or_else(|| file.strip_suffix(".appimage"))
        .unwrap_or(file);
    let cut = stem
        .char_indices()
        .find(|(i, c)| {
            (*c == '-' || *c == '_' || *c == '.')
                && stem[i + 1..].starts_with(|d: char| {
                    d.is_ascii_digit()
                        || d == 'v' && stem[i + 2..].starts_with(|e: char| e.is_ascii_digit())
                })
        })
        .map(|(i, _)| i)
        .unwrap_or(stem.len());
    let n = stem[..cut].trim_end_matches(['-', '_', '.']).to_owned();
    let n = n
        .strip_suffix("-x86_64")
        .or_else(|| n.strip_suffix("-amd64"))
        .unwrap_or(&n)
        .to_owned();
    if n.is_empty() { stem.to_owned() } else { n }
}

pub fn valid_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 64
        && !n.starts_with('.')
        && n.chars()
            .all(|c| c.is_alphanumeric() || "._- +".contains(c))
}

fn desktop_entry(ctx: &Context, a: &App) -> Result<()> {
    let dir = ctx.home.join(".local/share/applications");
    let file = dir.join(format!(
        "wopkg-appimage-{}.desktop",
        a.name.to_lowercase().replace(' ', "-")
    ));
    let text = format!(
        "[Desktop Entry]\nType=Application\nName={}\nExec=\"{}\" %U\nIcon=application-x-executable\nCategories=Utility;\nX-WolOS-Source=appimage\n",
        a.name,
        a.path.display()
    );
    std::fs::create_dir_all(&dir).ok();
    crate::journal::write_atomic(&file, text.as_bytes())
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct InstallRequest {
    /// Download from here (HTTPS) ...
    pub url: Option<String>,
    /// ... or adopt this file already on disk (moved into `~/Applications` unless already there).
    pub path: Option<PathBuf>,
    pub name: Option<String>,
    pub sha256: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Installed {
    pub app: App,
    pub signed: bool,
    pub journal: String,
}

fn make_exec(p: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755))
        .map_err(|_| Error::unavailable("The app cannot be made runnable."))
}

pub fn install(
    ctx: &Context,
    r: &InstallRequest,
    progress: &mut dyn FnMut(&serde_json::Value),
) -> Result<Installed> {
    let staging = ctx.apps.join(".wopkg-staging");
    std::fs::create_dir_all(&staging)
        .map_err(|_| Error::unavailable("~/Applications cannot be created."))?;
    let (src, file_name) = match (&r.url, &r.path) {
        (Some(u), None) => {
            let file = u
                .rsplit('/')
                .next()
                .filter(|f| !f.is_empty())
                .unwrap_or("app.AppImage")
                .to_owned();
            let dest = staging.join(&file);
            let item = crate::fetch::Item {
                url: u.clone(),
                dest: dest.clone(),
                sha256: r.sha256.clone(),
                size: None,
            };
            let d = crate::fetch::many(
                &ctx.net,
                &crate::fetch::Request {
                    items: vec![item],
                    jobs: Some(1),
                },
                progress,
            );
            if let Some(crate::fetch::Outcome::Failed { message }) =
                d.first().map(|d| d.outcome.clone())
            {
                return Err(Error::unavailable(message));
            }
            (dest, file)
        }
        (None, Some(p)) => (
            p.clone(),
            p.file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ),
        _ => return Err(Error::invalid("Give a URL or a file to install.")),
    };
    let info = inspect(&src)?;
    if !info.appimage {
        return Err(Error::invalid("That file is not an AppImage."));
    }
    if let Some(w) = &r.sha256
        && !w.eq_ignore_ascii_case(&info.sha256)
    {
        return Err(Error::untrusted(
            "The AppImage does not match its checksum.",
        ));
    }
    let signed = verify_signature(&src, &info, None)?;
    let name = r.name.clone().unwrap_or_else(|| name_from_file(&file_name));
    if !valid_name(&name) {
        return Err(Error::invalid(format!(
            "“{name}” cannot be used as an app name."
        )));
    }
    let mut reg = Registry::load(ctx);
    let dest = if src.parent() == Some(ctx.apps.as_path()) {
        src.clone()
    } else {
        ctx.apps.join(format!("{name}.AppImage"))
    };
    let prev = reg.apps.iter().find(|a| a.name == name).cloned();
    let mut undo = None;
    if dest != src {
        if dest.exists() {
            let p = backup_path(&dest);
            std::fs::rename(&dest, &p)
                .map_err(|_| Error::unavailable("The installed copy cannot be kept for undo."))?;
            undo = Some(p);
        }
        move_file(&src, &dest)?;
    }
    make_exec(&dest)?;
    let app = App {
        name: name.clone(),
        path: dest.clone(),
        sha256: info.sha256.clone(),
        update: info.update_raw.clone(),
        key_pin: info.key_pin.clone(),
        signed,
        url: r.url.clone(),
    };
    reg.upsert(app.clone());
    reg.save(ctx)?;
    desktop_entry(ctx, &app)?;
    let id = crate::journal::record(
        ctx,
        crate::journal::Entry::new(
            "appimage",
            if prev.is_some() { "update" } else { "install" },
            &name,
            prev.as_ref().map(|p| p.sha256.clone()),
            Some(info.sha256),
            undo.map(|p| crate::journal::Undo::Restore {
                from: p,
                to: dest.clone(),
            })
            .or(Some(crate::journal::Undo::Remove { path: dest.clone() })),
        ),
    )?;
    Ok(Installed {
        app,
        signed,
        journal: id,
    })
}

fn backup_path(p: &Path) -> PathBuf {
    PathBuf::from(format!("{}.prev", p.display()))
}

fn move_file(from: &Path, to: &Path) -> Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to)
        .map(|_| ())
        .and_then(|_| std::fs::remove_file(from))
        .map_err(|_| Error::unavailable("The app cannot be moved into ~/Applications."))
}

/// Find the `.zsync` URL for an app's update information.
pub fn zsync_url(ctx: &Context, u: &UpdateInfo, api_base: &str) -> Result<String> {
    match u {
        UpdateInfo::Zsync { url } => Ok(url.clone()),
        UpdateInfo::GhReleasesZsync {
            owner,
            repo,
            tag,
            file,
        } => {
            let path = if tag == "latest" {
                "releases/latest".to_owned()
            } else {
                format!("releases/tags/{tag}")
            };
            let api = format!("{api_base}/repos/{owner}/{repo}/{path}");
            ctx.net.check_url(&api)?;
            let resp = ctx
                .net
                .agent
                .get(&api)
                .header("Accept", "application/vnd.github+json")
                .call()
                .map_err(|e| Error::unavailable(format!("GitHub could not be reached ({e}).")))?;
            if resp.status().as_u16() != 200 {
                return Err(Error::unavailable(format!(
                    "GitHub has no release for {owner}/{repo}."
                )));
            }
            let mut body = String::new();
            resp.into_body()
                .into_reader()
                .take(4 << 20)
                .read_to_string(&mut body)
                .map_err(|_| Error::unavailable("GitHub's answer was cut off."))?;
            let v: serde_json::Value = serde_json::from_str(&body)
                .map_err(|_| Error::unavailable("GitHub sent an unreadable answer."))?;
            // More than one match happens (a re-spin next to the original); the shortest name is the
            // plain one, which is the file its own control file describes.
            v["assets"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|a| a["name"].as_str().is_some_and(|n| glob(file, n)))
                .min_by_key(|a| a["name"].as_str().map_or(usize::MAX, str::len))
                .and_then(|a| a["browser_download_url"].as_str().map(str::to_owned))
                .ok_or_else(|| {
                    Error::new(
                        crate::ffi::Status::NotFound,
                        format!("The latest {repo} release has no update file."),
                    )
                })
        }
    }
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct UpdateRequest {
    /// Only this app (default: every registered app).
    pub name: Option<String>,
    #[serde(default)]
    pub check_only: bool,
    /// Also register AppImages found in `~/Applications` that wopkg does not know yet.
    #[serde(default)]
    pub adopt: bool,
    /// GitHub API base (tests).
    pub github_api: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum UpdateState {
    UpToDate,
    Available {
        size: u64,
    },
    Updated {
        reused_bytes: u64,
        fetched_bytes: u64,
        journal: String,
    },
    NoUpdateInfo,
    Failed {
        message: String,
    },
}

#[derive(Clone, Debug, Serialize)]
pub struct UpdateResult {
    pub name: String,
    #[serde(flatten)]
    pub state: UpdateState,
}

/// Remove an installed AppImage: the file goes to `<file>.prev` (undo puts it back), the menu
/// entry and the registry line go away.
pub fn remove(ctx: &Context, name: &str) -> Result<String> {
    let mut reg = Registry::load(ctx);
    let a = reg
        .apps
        .iter()
        .find(|a| a.name == name)
        .cloned()
        .ok_or_else(|| {
            Error::new(
                crate::ffi::Status::NotFound,
                format!("No AppImage called “{name}” is installed."),
            )
        })?;
    let prev = backup_path(&a.path);
    let _ = std::fs::remove_file(&prev);
    std::fs::rename(&a.path, &prev)
        .map_err(|_| Error::unavailable(format!("{name} cannot be moved away.")))?;
    let desktop = ctx.home.join(format!(
        ".local/share/applications/wopkg-appimage-{}.desktop",
        a.name.to_lowercase().replace(' ', "-")
    ));
    let _ = std::fs::remove_file(desktop);
    reg.apps.retain(|x| x.name != name);
    reg.save(ctx)?;
    crate::journal::record(
        ctx,
        crate::journal::Entry::new(
            "appimage",
            "remove",
            name,
            Some(a.sha256.clone()),
            None,
            Some(crate::journal::Undo::Restore {
                from: prev,
                to: a.path.clone(),
            }),
        ),
    )
}

/// Register AppImages in `~/Applications` that are not yet known (they get updates too).
pub fn adopt(ctx: &Context) -> Vec<String> {
    let mut reg = Registry::load(ctx);
    let mut added = Vec::new();
    for e in std::fs::read_dir(&ctx.apps).into_iter().flatten().flatten() {
        let p = e.path();
        let f = e.file_name().to_string_lossy().into_owned();
        if !f.to_lowercase().ends_with(".appimage") || reg.apps.iter().any(|a| a.path == p) {
            continue;
        }
        let Ok(info) = inspect(&p) else { continue };
        if !info.appimage {
            continue;
        }
        let name = name_from_file(&f);
        if !valid_name(&name) || reg.apps.iter().any(|a| a.name == name) {
            continue;
        }
        reg.upsert(App {
            name: name.clone(),
            path: p,
            sha256: info.sha256,
            update: info.update_raw,
            key_pin: info.key_pin,
            signed: info.signed,
            url: None,
        });
        added.push(name);
    }
    if !added.is_empty() {
        let _ = reg.save(ctx);
    }
    added
}

pub fn update(
    ctx: &Context,
    r: &UpdateRequest,
    progress: &mut dyn FnMut(&serde_json::Value),
) -> Result<Vec<UpdateResult>> {
    if r.adopt {
        adopt(ctx);
    }
    let api = r
        .github_api
        .clone()
        .unwrap_or("https://api.github.com".into());
    let reg = Registry::load(ctx);
    let mut out = Vec::new();
    for a in reg
        .apps
        .iter()
        .filter(|a| r.name.as_deref().is_none_or(|n| n == a.name))
    {
        let state = match update_one(ctx, a, &api, r.check_only, progress) {
            Ok(s) => s,
            Err(e) => UpdateState::Failed { message: e.message },
        };
        progress(&serde_json::json!({"event":"appimage","name":a.name,"result":state}));
        out.push(UpdateResult {
            name: a.name.clone(),
            state,
        });
    }
    if r.name.is_some() && out.is_empty() {
        return Err(Error::new(
            crate::ffi::Status::NotFound,
            "No AppImage with that name is installed.",
        ));
    }
    Ok(out)
}

fn update_one(
    ctx: &Context,
    a: &App,
    api: &str,
    check_only: bool,
    progress: &mut dyn FnMut(&serde_json::Value),
) -> Result<UpdateState> {
    let Some(u) = a.update.as_deref().and_then(UpdateInfo::parse) else {
        return Ok(UpdateState::NoUpdateInfo);
    };
    let zurl = zsync_url(ctx, &u, api)?;
    ctx.net.check_url(&zurl)?;
    let resp = ctx.net.agent.get(&zurl).call().map_err(|e| {
        Error::unavailable(format!("The update server could not be reached ({e})."))
    })?;
    if resp.status().as_u16() != 200 {
        return Err(Error::unavailable(format!(
            "The update server answered {}.",
            resp.status().as_u16()
        )));
    }
    let mut zbytes = Vec::new();
    resp.into_body()
        .into_reader()
        .take(64 << 20)
        .read_to_end(&mut zbytes)
        .map_err(|_| Error::unavailable("The update description was cut off."))?;
    let c = crate::zsync::parse(&zbytes)?;
    let cur_sha1 = crate::zsync::sha1_file(&a.path).ok_or_else(|| {
        Error::new(
            crate::ffi::Status::NotFound,
            format!("{} is missing from ~/Applications.", a.name),
        )
    })?;
    if cur_sha1 == c.sha1 {
        return Ok(UpdateState::UpToDate);
    }
    if check_only {
        return Ok(UpdateState::Available { size: c.length });
    }
    let file_url = crate::zsync::resolve(&zurl, &c.url);
    let staging = ctx.apps.join(".wopkg-staging");
    std::fs::create_dir_all(&staging)
        .map_err(|_| Error::unavailable("~/Applications cannot be written."))?;
    let new = staging.join(format!("{}.new", a.name));
    let stats = crate::zsync::build(&ctx.net, &c, &file_url, &a.path, &new, progress)?;
    let info = inspect(&new)?;
    if !info.appimage {
        let _ = std::fs::remove_file(&new);
        return Err(Error::untrusted(
            "The update is not an AppImage. It was not installed.",
        ));
    }
    if let Err(e) = verify_signature(&new, &info, a.key_pin.as_deref()) {
        let _ = std::fs::remove_file(&new);
        return Err(e);
    }
    make_exec(&new)?;
    let prev = backup_path(&a.path);
    let _ = std::fs::remove_file(&prev);
    std::fs::hard_link(&a.path, &prev)
        .or_else(|_| std::fs::copy(&a.path, &prev).map(|_| ()))
        .map_err(|_| Error::unavailable("The installed copy cannot be kept for undo."))?;
    std::fs::rename(&new, &a.path)
        .map_err(|_| Error::unavailable("The update cannot be put in place."))?;
    let mut reg = Registry::load(ctx);
    let mut na = a.clone();
    na.sha256 = info.sha256.clone();
    na.update = info.update_raw.clone().or(na.update);
    if na.key_pin.is_none() {
        na.key_pin = info.key_pin.clone();
        na.signed = info.signed;
    }
    reg.upsert(na);
    reg.save(ctx)?;
    let id = crate::journal::record(
        ctx,
        crate::journal::Entry::new(
            "appimage",
            "update",
            &a.name,
            Some(a.sha256.clone()),
            Some(info.sha256),
            Some(crate::journal::Undo::Restore {
                from: prev,
                to: a.path.clone(),
            }),
        ),
    )?;
    Ok(UpdateState::Updated {
        reused_bytes: stats.reused_bytes,
        fetched_bytes: stats.fetched_bytes,
        journal: id,
    })
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::testutil::{Served, temp};

    /// A minimal 64-bit ELF AppImage with the given sections (name, contents).
    pub fn fake_appimage(payload: &[u8], secs: &[(&str, &[u8])]) -> Vec<u8> {
        let mut b = vec![0u8; 64];
        b[..4].copy_from_slice(b"\x7fELF");
        b[4] = 2;
        b[5] = 1;
        b[8..11].copy_from_slice(b"AI\x02");
        b.extend_from_slice(payload);
        let mut strtab = vec![0u8];
        let mut hdrs = vec![(0u32, 0usize, 0usize)];
        for (n, data) in secs {
            let name_off = strtab.len() as u32;
            strtab.extend_from_slice(n.as_bytes());
            strtab.push(0);
            hdrs.push((name_off, b.len(), data.len()));
            b.extend_from_slice(data);
        }
        let sh_name = strtab.len() as u32;
        strtab.extend_from_slice(b".shstrtab\0");
        hdrs.push((sh_name, b.len(), strtab.len()));
        b.extend_from_slice(&strtab);
        let shoff = b.len();
        for (name, off, size) in &hdrs {
            let mut h = [0u8; 64];
            h[..4].copy_from_slice(&name.to_le_bytes());
            h[0x18..0x20].copy_from_slice(&(*off as u64).to_le_bytes());
            h[0x20..0x28].copy_from_slice(&(*size as u64).to_le_bytes());
            b.extend_from_slice(&h);
        }
        b[0x28..0x30].copy_from_slice(&(shoff as u64).to_le_bytes());
        b[0x3a..0x3c].copy_from_slice(&64u16.to_le_bytes());
        b[0x3c..0x3e].copy_from_slice(&(hdrs.len() as u16).to_le_bytes());
        b[0x3e..0x40].copy_from_slice(&((hdrs.len() - 1) as u16).to_le_bytes());
        b
    }

    pub fn ctx(d: &Path) -> Context {
        Context::new(crate::context::Config {
            home: Some(d.to_path_buf()),
            cache_dir: Some(d.join("cache")),
            state_dir: Some(d.join("state")),
            data_dir: Some(d.join("data")),
            allow_loopback_http: true,
            ..Default::default()
        })
        .unwrap()
    }

    #[test]
    fn update_info_forms() {
        let cases = [
            (
                "zsync|https://x/a.zsync",
                Some(UpdateInfo::Zsync {
                    url: "https://x/a.zsync".into(),
                }),
            ),
            (
                "gh-releases-zsync|o|r|latest|App-*x86_64.AppImage.zsync",
                Some(UpdateInfo::GhReleasesZsync {
                    owner: "o".into(),
                    repo: "r".into(),
                    tag: "latest".into(),
                    file: "App-*x86_64.AppImage.zsync".into(),
                }),
            ),
            ("gh-releases-zsync|o/../x|r|latest|f", None),
            ("bintray-zsync|a|b|c|d", None),
            ("", None),
        ];
        for (s, want) in cases {
            assert_eq!(UpdateInfo::parse(s), want, "{s}");
        }
        assert!(glob(
            "App-*x86_64.AppImage.zsync",
            "App-1.2-x86_64.AppImage.zsync"
        ));
        assert!(!glob(
            "App-*x86_64.AppImage.zsync",
            "App-1.2-aarch64.AppImage.zsync"
        ));
        assert!(glob("*", "x") && glob("a*b*c", "aXbYc") && !glob("a*b*c", "aXcYb"));
    }

    #[test]
    fn names_from_files() {
        for (f, n) in [
            ("Krita-5.2.6-x86_64.AppImage", "Krita"),
            ("Obsidian-1.6.7.AppImage", "Obsidian"),
            ("nvim.appimage", "nvim"),
            ("LM-Studio-0.3.5.AppImage", "LM-Studio"),
            ("app-v2.0.AppImage", "app"),
        ] {
            assert_eq!(name_from_file(f), n, "{f}");
        }
    }

    #[test]
    fn inspect_reads_sections_without_running() {
        let b = fake_appimage(
            b"payload",
            &[
                (".upd_info", b"zsync|https://e/x.zsync\0\0\0"),
                (".sha256_sig", b"\0\0"),
                (".sig_key", b""),
            ],
        );
        let d = temp("ai-inspect");
        std::fs::write(d.join("a"), &b).unwrap();
        let i = inspect(&d.join("a")).unwrap();
        assert!(i.appimage && !i.signed);
        assert_eq!(
            i.update,
            Some(UpdateInfo::Zsync {
                url: "https://e/x.zsync".into()
            })
        );
        assert!(!verify_signature(&d.join("a"), &i, None).unwrap());
        assert!(
            verify_signature(&d.join("a"), &i, Some("pin")).is_err(),
            "unsigned after signed is refused"
        );
    }

    #[test]
    fn install_adopt_update_rollback() {
        let d = temp("ai-flow");
        let c = ctx(&d);
        let payload_v1 = crate::zsync::tests::noise(150_000, 2);
        let mut payload_v2 = payload_v1.clone();
        payload_v2[70_000..70_016].copy_from_slice(b"version-two-here");
        // The update information inside both files names the loopback server, so learn its port first.
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        let u = format!("zsync|http://127.0.0.1:{port}/App.AppImage.zsync");
        let new = fake_appimage(&payload_v2, &[(".upd_info", u.as_bytes())]);
        let old = fake_appimage(&payload_v1, &[(".upd_info", u.as_bytes())]);
        let z = crate::zsync::make(&new, "App.AppImage", "App.AppImage", 2048);
        let _srv = crate::testutil::serve_on(
            port,
            vec![
                ("/App.AppImage", Served::bytes(new.clone())),
                ("/App.AppImage.zsync", Served::bytes(z)),
            ],
        );
        let srv_holder = (new, old);
        std::fs::create_dir_all(&c.apps).unwrap();
        std::fs::write(c.apps.join("App-1.0-x86_64.AppImage"), &srv_holder.1).unwrap();
        assert_eq!(adopt(&c), ["App"]);
        let r = update(
            &c,
            &UpdateRequest {
                check_only: true,
                ..Default::default()
            },
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(
            r[0].state,
            UpdateState::Available {
                size: srv_holder.0.len() as u64
            }
        );
        let r = update(&c, &UpdateRequest::default(), &mut |_| {}).unwrap();
        let UpdateState::Updated {
            fetched_bytes,
            journal,
            ..
        } = &r[0].state
        else {
            panic!("{:?}", r[0].state)
        };
        assert!(
            *fetched_bytes < 10_000,
            "delta, not a full download: {fetched_bytes}"
        );
        let path = c.apps.join("App-1.0-x86_64.AppImage");
        assert_eq!(std::fs::read(&path).unwrap(), srv_holder.0);
        let r = update(&c, &UpdateRequest::default(), &mut |_| {}).unwrap();
        assert_eq!(r[0].state, UpdateState::UpToDate);
        crate::journal::rollback(&c, journal).unwrap();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            srv_holder.1,
            "rollback restores the previous file"
        );
    }

    #[test]
    fn the_plain_zsync_wins_when_a_release_has_two_that_match() {
        // Real release: `App-2.8-1-x86_64.AppImage.zsync` points at `App-2.8-x86_64.AppImage`.
        let body = r#"{"assets":[
          {"name":"App-2.8-1-x86_64.AppImage.zsync","browser_download_url":"https://e/App-2.8-1-x86_64.AppImage.zsync"},
          {"name":"App-2.8-x86_64.AppImage.zsync","browser_download_url":"https://e/App-2.8-x86_64.AppImage.zsync"}]}"#;
        let srv = crate::testutil::serve(vec![(
            "/repos/o/r/releases/latest",
            Served::bytes(body.as_bytes().to_vec()),
        )]);
        let d = temp("ai-glob");
        let c = ctx(&d);
        let u =
            UpdateInfo::parse("gh-releases-zsync|o|r|latest|App-*x86_64.AppImage.zsync").unwrap();
        assert_eq!(
            zsync_url(&c, &u, &srv.url("")).unwrap(),
            "https://e/App-2.8-x86_64.AppImage.zsync"
        );
    }
}

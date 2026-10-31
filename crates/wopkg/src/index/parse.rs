//! Source inputs -> records: pacman sync databases (tar, gzip or zstd), Flathub AppStream XML
//! (`.gz` or plain), the AUR metadata dump (`packages-meta-v1.json[.gz]`), the `.wol` index
//! (`index.json`, ADR 0011), the recipe index and the AppImage feed.
use super::{Kind, Record, flatpak_key, package_key};
use crate::ffi::{Error, Result};
use std::io::{BufReader, Read};

/// gzip, zstd or plain bytes by magic number.
pub fn decompress(bytes: &[u8]) -> Result<Vec<u8>> {
    let bad = |_| Error::untrusted("A downloaded index is damaged.");
    if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut out = Vec::with_capacity(bytes.len() * 6);
        flate2::read::MultiGzDecoder::new(bytes)
            .read_to_end(&mut out)
            .map_err(bad)?;
        Ok(out)
    } else if bytes.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        let mut out = Vec::with_capacity(bytes.len() * 6);
        ruzstd::decoding::StreamingDecoder::new(bytes)
            .map_err(|_| Error::untrusted("A downloaded index is damaged."))?
            .read_to_end(&mut out)
            .map_err(bad)?;
        Ok(out)
    } else {
        Ok(bytes.to_vec())
    }
}

fn octal(f: &[u8]) -> usize {
    f.iter()
        .take_while(|b| **b != 0 && **b != b' ')
        .fold(0usize, |a, b| {
            a.saturating_mul(8)
                .saturating_add(b.wrapping_sub(b'0') as usize & 7)
        })
}

/// Regular files of a ustar/pax/GNU tar archive as (path, contents).
pub fn tar_files(tar: &[u8]) -> Vec<(String, &[u8])> {
    let mut out = Vec::new();
    let (mut at, mut long): (usize, Option<String>) = (0, None);
    while at + 512 <= tar.len() {
        let h = &tar[at..at + 512];
        if h.iter().all(|b| *b == 0) {
            break;
        }
        let size = octal(&h[124..136]);
        let kind = h[156];
        let body_at = at + 512;
        let body = tar.get(body_at..body_at + size).unwrap_or(&[]);
        let field = |r: std::ops::Range<usize>| {
            String::from_utf8_lossy(&h[r])
                .trim_end_matches('\0')
                .to_owned()
        };
        let mut name = field(0..100);
        let prefix = field(345..500);
        if &h[257..262] == b"ustar" && !prefix.is_empty() {
            name = format!("{prefix}/{name}");
        }
        match kind {
            b'L' => {
                long = Some(
                    String::from_utf8_lossy(body)
                        .trim_end_matches('\0')
                        .to_owned(),
                )
            }
            b'x' => {
                let t = String::from_utf8_lossy(body);
                long = t.lines().find_map(|l| {
                    l.split_once(' ')?
                        .1
                        .strip_prefix("path=")
                        .map(str::to_owned)
                });
            }
            b'0' | 0 => {
                out.push((long.take().unwrap_or(name), body));
            }
            _ => long = None,
        }
        at = body_at + size.div_ceil(512) * 512;
    }
    out
}

/// `%KEY%` blocks of a pacman `desc` file.
pub fn desc_fields(text: &str) -> Vec<(&str, Vec<&str>)> {
    let mut out: Vec<(&str, Vec<&str>)> = Vec::new();
    for l in text.lines() {
        if l.len() > 2 && l.starts_with('%') && l.ends_with('%') {
            out.push((&l[1..l.len() - 1], Vec::new()));
        } else if !l.is_empty()
            && let Some(last) = out.last_mut()
        {
            last.1.push(l);
        }
    }
    out
}

fn field<'a>(f: &'a [(&str, Vec<&'a str>)], k: &str) -> &'a str {
    f.iter()
        .find(|(n, _)| *n == k)
        .and_then(|(_, v)| v.first().copied())
        .unwrap_or("")
}

/// A pacman sync database (`<repo>.db`).
pub fn pacman_db(repo: &str, bytes: &[u8]) -> Result<Vec<Record>> {
    let tar = decompress(bytes)?;
    let mut out = Vec::new();
    for (path, body) in tar_files(&tar) {
        if !path.ends_with("/desc") {
            continue;
        }
        let text = String::from_utf8_lossy(body);
        let f = desc_fields(&text);
        let name = field(&f, "NAME");
        if name.is_empty() {
            continue;
        }
        out.push(Record {
            kind: Kind::Pacman,
            repo: repo.into(),
            name: name.into(),
            id: name.into(),
            version: field(&f, "VERSION").into(),
            summary: field(&f, "DESC").into(),
            key: package_key(name),
            votes: 0,
        });
    }
    if out.is_empty() && !tar.is_empty() {
        return Err(Error::untrusted(format!(
            "The {repo} package list is damaged."
        )));
    }
    Ok(out)
}

/// `&name;` / `&#N;` / `&#xN;` -> text.
fn entity(name: &str) -> String {
    match name {
        "amp" => "&".into(),
        "lt" => "<".into(),
        "gt" => ">".into(),
        "quot" => "\"".into(),
        "apos" => "'".into(),
        n => n
            .strip_prefix("#x")
            .and_then(|h| u32::from_str_radix(h, 16).ok())
            .or_else(|| n.strip_prefix('#').and_then(|d| d.parse().ok()))
            .and_then(char::from_u32)
            .map(String::from)
            .unwrap_or_default(),
    }
}

/// Flathub AppStream (`appstream.xml[.gz]`): desktop and console applications.
pub fn appstream(remote: &str, bytes: &[u8]) -> Result<Vec<Record>> {
    use quick_xml::events::Event;
    let xml = decompress(bytes)?;
    let mut r = quick_xml::Reader::from_reader(BufReader::new(&xml[..]));
    let mut buf = Vec::new();
    let mut out = Vec::new();
    let (mut depth, mut comp_depth) = (0usize, None::<usize>);
    let mut cur: Option<(String, String, String, String, String)> = None; // id name summary version bundle
    let mut field: Option<&'static str> = None;
    loop {
        match r.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                depth += 1;
                let n = e.name();
                let lang = e
                    .attributes()
                    .flatten()
                    .any(|a| a.key.as_ref() == b"xml:lang");
                match n.as_ref() {
                    b"component" if comp_depth.is_none() => {
                        let t = e
                            .attributes()
                            .flatten()
                            .find(|a| a.key.as_ref() == b"type")
                            .map(|a| a.value.into_owned())
                            .unwrap_or_default();
                        if t == b"desktop-application"
                            || t == b"desktop"
                            || t == b"console-application"
                        {
                            comp_depth = Some(depth);
                            cur = Some(Default::default());
                        }
                    }
                    b"id" | b"name" | b"summary" | b"bundle"
                        if comp_depth == Some(depth - 1) && !lang =>
                    {
                        let f = match n.as_ref() {
                            b"id" => "id",
                            b"name" => "name",
                            b"summary" => "summary",
                            _ => "bundle",
                        };
                        // Only the first untranslated element of each kind counts.
                        let filled = cur.as_ref().is_some_and(|c| {
                            !match f {
                                "id" => &c.0,
                                "name" => &c.1,
                                "summary" => &c.2,
                                _ => &c.4,
                            }
                            .is_empty()
                        });
                        field = (!filled).then_some(f);
                    }
                    // `<release version=..>` with a description inside.
                    b"release" => {
                        if let Some(c) = cur.as_mut()
                            && c.3.is_empty()
                            && let Some(v) = e.attributes().flatten().find(|a| a.key.as_ref() == b"version")
                        {
                            c.3 = String::from_utf8_lossy(&v.value).into_owned();
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(e)) => {
                if e.name().as_ref() == b"release"
                    && let (Some(c), Some(_)) = (cur.as_mut(), comp_depth)
                    && c.3.is_empty()
                    && let Some(v) = e
                        .attributes()
                        .flatten()
                        .find(|a| a.key.as_ref() == b"version")
                {
                    c.3 = String::from_utf8_lossy(&v.value).into_owned();
                }
            }
            Ok(ev @ (Event::Text(_) | Event::GeneralRef(_))) => {
                if let (Some(f), Some(c)) = (field, cur.as_mut()) {
                    let s = match ev {
                        Event::Text(t) => t.decode().map(|s| s.into_owned()).unwrap_or_default(),
                        Event::GeneralRef(r) => entity(&r.decode().unwrap_or_default()),
                        _ => String::new(),
                    };
                    let slot = match f {
                        "id" => &mut c.0,
                        "name" => &mut c.1,
                        "summary" => &mut c.2,
                        _ => &mut c.4,
                    };
                    slot.push_str(&s);
                }
            }
            Ok(Event::End(e)) => {
                field = None;
                if e.name().as_ref() == b"component" && comp_depth == Some(depth) {
                    if let Some((id, name, summary, version, bundle)) = cur.take() {
                        let (id, name, summary, bundle) = (
                            id.trim().to_owned(),
                            name.trim().to_owned(),
                            summary.trim().to_owned(),
                            bundle.trim().to_owned(),
                        );
                        // Flatpak ref `app/<id>/<arch>/<branch>` names the installable id.
                        let app = bundle
                            .split('/')
                            .nth(1)
                            .map(str::to_owned)
                            .unwrap_or_else(|| id.trim_end_matches(".desktop").to_owned());
                        if !app.is_empty() {
                            let name = if name.is_empty() { app.clone() } else { name };
                            out.push(Record {
                                kind: Kind::Flatpak,
                                repo: remote.into(),
                                key: flatpak_key(&app, &name),
                                name,
                                id: app,
                                version,
                                summary,
                                votes: 0,
                            });
                        }
                    }
                    comp_depth = None;
                }
                depth = depth.saturating_sub(1);
            }
            Ok(Event::Eof) => break,
            Err(_) => return Err(Error::untrusted("The Flathub app list is damaged.")),
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}

#[derive(serde::Deserialize)]
struct Aur {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Version", default)]
    version: String,
    #[serde(rename = "Description", default)]
    description: Option<String>,
    #[serde(rename = "NumVotes", default)]
    votes: u32,
}

/// The AUR metadata dump: a JSON array of package objects.
pub fn aur(bytes: &[u8]) -> Result<Vec<Record>> {
    let json = decompress(bytes)?;
    let pkgs: Vec<Aur> = serde_json::from_slice(&json)
        .map_err(|_| Error::untrusted("The AUR package list is damaged."))?;
    Ok(pkgs
        .into_iter()
        .map(|p| Record {
            kind: Kind::Aur,
            repo: "aur".into(),
            key: package_key(&p.name),
            id: p.name.clone(),
            name: p.name,
            version: p.version,
            summary: p.description.unwrap_or_default(),
            votes: p.votes,
        })
        .collect())
}

/// `.wol` index (`archicture/modules/packages.md`).
pub fn wol_index(bytes: &[u8]) -> Result<Vec<Record>> {
    let idx = std::str::from_utf8(bytes)
        .ok()
        .and_then(|t| shared::package::Index::parse(t).ok())
        .ok_or_else(|| Error::untrusted("The WolOS app list is damaged."))?;
    Ok(idx
        .packages
        .into_iter()
        .map(|e| Record {
            kind: Kind::Wol,
            repo: "wol".into(),
            key: package_key(&e.package),
            name: if e.name.is_empty() {
                e.package.clone()
            } else {
                e.name.clone()
            },
            id: e.id.clone(),
            version: e.version.clone(),
            summary: e.summary.clone(),
            votes: 0,
        })
        .collect())
}

#[derive(serde::Deserialize)]
struct Recipes {
    recipes: Vec<RecipeEntry>,
}
#[derive(serde::Deserialize)]
struct RecipeEntry {
    name: String,
    version: String,
    #[serde(default)]
    summary: String,
}

/// Recipe index (`{"recipes":[{"name","version","summary"}]}`, `archicture/modules/wopkg.md`).
pub fn recipes(bytes: &[u8]) -> Result<Vec<Record>> {
    let r: Recipes = serde_json::from_slice(bytes)
        .map_err(|_| Error::untrusted("The recipe list is damaged."))?;
    Ok(r.recipes
        .into_iter()
        .map(|e| Record {
            kind: Kind::Recipe,
            repo: "recipes".into(),
            key: package_key(&e.name),
            id: e.name.clone(),
            name: e.name,
            version: e.version,
            summary: e.summary,
            votes: 0,
        })
        .collect())
}

#[derive(serde::Deserialize)]
struct Feed {
    items: Vec<FeedItem>,
}
#[derive(serde::Deserialize)]
struct FeedItem {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    links: Option<Vec<FeedLink>>,
}
#[derive(serde::Deserialize)]
struct FeedLink {
    #[serde(rename = "type")]
    kind: String,
    url: String,
}

/// AppImage catalogue (`appimage.github.io/feed.json`): items with a GitHub download link.
pub fn appimage_feed(bytes: &[u8]) -> Result<Vec<Record>> {
    let f: Feed = serde_json::from_slice(bytes)
        .map_err(|_| Error::untrusted("The AppImage list is damaged."))?;
    Ok(f.items
        .into_iter()
        .filter_map(|i| {
            let link = i.links.as_ref()?.iter().find(|l| l.kind == "GitHub")?;
            Some(Record {
                kind: Kind::Appimage,
                repo: "appimagehub".into(),
                key: package_key(&i.name),
                id: format!("gh:{}", link.url),
                name: i.name,
                version: String::new(),
                summary: i
                    .description
                    .unwrap_or_default()
                    .lines()
                    .next()
                    .unwrap_or("")
                    .chars()
                    .take(200)
                    .collect(),
                votes: 0,
            })
        })
        .collect())
}

#[cfg(test)]
pub mod tests_support {
    use std::io::Write;

    pub fn tar(files: &[(&str, &str)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (name, body) in files {
            let mut h = [0u8; 512];
            h[..name.len()].copy_from_slice(name.as_bytes());
            h[100..107].copy_from_slice(b"0000644");
            h[124..135].copy_from_slice(format!("{:011o}", body.len()).as_bytes());
            h[156] = b'0';
            h[257..262].copy_from_slice(b"ustar");
            out.extend_from_slice(&h);
            out.extend_from_slice(body.as_bytes());
            out.resize(out.len().div_ceil(512) * 512, 0);
        }
        out.resize(out.len() + 1024, 0);
        out
    }

    pub fn gz(b: &[u8]) -> Vec<u8> {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        e.write_all(b).unwrap();
        e.finish().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::{gz, tar};
    use super::*;

    #[test]
    fn pacman_database_gzip_and_plain() {
        let t = tar(&[
            (
                "firefox-140.0-1/desc",
                "%FILENAME%\nfirefox-140.0-1-x86_64.pkg.tar.zst\n\n%NAME%\nfirefox\n\n%VERSION%\n140.0-1\n\n%DESC%\nFast, Private & Safe Web Browser\n\n",
            ),
            (
                "vim-9.1-1/desc",
                "%NAME%\nvim\n\n%VERSION%\n9.1-1\n\n%DESC%\nVi Improved\n",
            ),
        ]);
        for bytes in [gz(&t), t.clone()] {
            let r = pacman_db("extra", &bytes).unwrap();
            assert_eq!(r.len(), 2);
            assert_eq!(
                (r[0].name.as_str(), r[0].version.as_str()),
                ("firefox", "140.0-1")
            );
            assert_eq!(r[1].summary, "Vi Improved");
        }
        assert!(pacman_db("extra", &gz(b"not a tar at all, really not")).is_err());
    }

    #[test]
    fn long_names_via_pax() {
        let long = format!("{}/desc", "a".repeat(120));
        let pax = format!("{} path={long}\n", long.len() + 7 + 3);
        let mut t = Vec::new();
        let mut h = [0u8; 512];
        h[..9].copy_from_slice(b"PaxHeader");
        h[124..135].copy_from_slice(format!("{:011o}", pax.len()).as_bytes());
        h[156] = b'x';
        t.extend_from_slice(&h);
        t.extend_from_slice(pax.as_bytes());
        t.resize(t.len().div_ceil(512) * 512, 0);
        t.extend(tar(&[("short", "%NAME%\nx\n")]));
        let f = tar_files(&t);
        assert_eq!(f[0].0, long);
    }

    #[test]
    fn flathub_appstream() {
        let xml = r#"<?xml version="1.0"?><components origin="flathub">
<component type="desktop-application"><id>org.mozilla.firefox</id><name>Firefox</name><name xml:lang="de">Feuerfuchs</name>
<summary>Fast, Private &amp; Safe Web Browser</summary><developer><name>Mozilla</name></developer>
<bundle type="flatpak">app/org.mozilla.firefox/x86_64/stable</bundle><releases><release version="140.0" timestamp="1"><description><p>New</p></description></release><release version="139.0"/></releases></component>
<component type="runtime"><id>org.gnome.Platform</id><name>GNOME</name></component>
<component type="desktop"><id>com.spotify.Client.desktop</id><name>Spotify</name><summary>Music</summary></component>
</components>"#;
        let r = appstream("flathub", &gz(xml.as_bytes())).unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].name, "Firefox");
        assert_eq!(r[0].summary, "Fast, Private & Safe Web Browser");
        assert_eq!(
            (r[0].id.as_str(), r[0].version.as_str(), r[0].key.as_str()),
            ("org.mozilla.firefox", "140.0", "firefox")
        );
        assert_eq!(
            (r[1].id.as_str(), r[1].key.as_str()),
            ("com.spotify.Client", "spotify")
        );
    }

    #[test]
    fn aur_dump_wol_recipes_feed() {
        let a = aur(br#"[{"Name":"paru-bin","Version":"2.0-1","Description":"AUR helper","NumVotes":42},{"Name":"x","Version":"1","Description":null}]"#).unwrap();
        assert_eq!((a[0].key.as_str(), a[0].votes), ("paru", 42));
        assert_eq!(a[1].summary, "");
        let wol = format!(
            r#"{{"format":1,"packages":[{{"id":"io.x.Raider","name":"File Shredder","summary":"s","version":"3.1.0","package":"raider","abi":"x","url":"https://x/r.wol","sha256":"{}","size":1}}]}}"#,
            "a".repeat(64)
        );
        let w = wol_index(wol.as_bytes()).unwrap();
        assert_eq!(
            (w[0].name.as_str(), w[0].id.as_str(), w[0].key.as_str()),
            ("File Shredder", "io.x.Raider", "raider")
        );
        let r = recipes(
            br#"{"recipes":[{"name":"bend","version":"0.2.38","summary":"Massively parallel"}]}"#,
        )
        .unwrap();
        assert_eq!(r[0].kind, Kind::Recipe);
        let f = appimage_feed(br#"{"items":[{"name":"Krita","description":"Paint\nmore","links":[{"type":"GitHub","url":"KDE/krita"}]},{"name":"NoLink","links":[]},{"name":"Null","links":null}]}"#).unwrap();
        assert_eq!(
            (f.len(), f[0].id.as_str(), f[0].summary.as_str()),
            (1, "gh:KDE/krita", "Paint")
        );
    }
}

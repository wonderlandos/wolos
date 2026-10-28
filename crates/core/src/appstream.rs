//! App Store details from AppStream catalogue XML: Flathub's local copy (`flatpak` keeps it under
//! `appstream/flathub/<arch>/active/`) and Arch's `/usr/share/swcatalog/xml/*.xml.gz`. Reads only
//! the untranslated text, the screenshot closest to the details page width and the cached icon.

/// One app as the details page shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Details {
    pub id: String,
    pub name: String,
    pub summary: String,
    /// Paragraphs; list items start with "• ".
    pub description: Vec<String>,
    pub developer: Option<String>,
    /// Screenshot URLs, default first.
    pub screenshots: Vec<String>,
    /// Cached icon file name (`<icons dir>/128x128/<name>`), the largest offered.
    pub icon: Option<(u32, String)>,
}

/// Flathub's local AppStream (user then system install): (xml, icons dir).
pub fn flathub_paths(home: &str) -> [(String, String); 2] {
    let arch = std::env::consts::ARCH;
    let at = |root: &str| {
        let d = format!("{root}/appstream/{}/{arch}/active", crate::flatpak::REMOTE);
        (format!("{d}/appstream.xml"), format!("{d}/icons"))
    };
    [
        at(&format!("{home}/{}", crate::flatpak::USER_DIR)),
        at(crate::flatpak::SYSTEM_DIR),
    ]
}

/// Arch's AppStream for native packages (gzip; read with `gzip -dc`).
pub const NATIVE_XML: [&str; 2] = [
    "/usr/share/swcatalog/xml/extra.xml.gz",
    "/usr/share/swcatalog/xml/core.xml.gz",
];

/// Width the details page shows screenshots at; the closest thumbnail wins.
pub const SHOT_WIDTH: u32 = 752;

/// Where a downloaded screenshot is kept: `<cache>/wolos/store/<fnv-1a of the url>.<ext>`.
/// None for anything but an https URL.
pub fn shot_cache(cache_home: &str, url: &str) -> Option<String> {
    if !url.starts_with("https://") || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in url.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    let ext = match url
        .rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some(e @ ("png" | "jpg" | "jpeg" | "webp")) => e.to_owned(),
        _ => "png".into(),
    };
    Some(format!("{cache_home}/wolos/store/{h:016x}.{ext}"))
}

/// `$XDG_CACHE_HOME`, else `~/.cache`.
pub fn cache_home() -> String {
    std::env::var("XDG_CACHE_HOME")
        .ok()
        .filter(|d| d.starts_with('/'))
        .unwrap_or_else(|| format!("{}/.cache", std::env::var("HOME").unwrap_or_default()))
}

/// The component whose `<id>` is `id`.
pub fn by_id(xml: &str, id: &str) -> Option<Details> {
    component(xml, &format!("<id>{}</id>", escape(id)))
}

/// The component that `<pkgname>` names (Arch's catalogue).
pub fn by_pkgname(xml: &str, pkg: &str) -> Option<Details> {
    component(xml, &format!("<pkgname>{}</pkgname>", escape(pkg)))
}

fn component(xml: &str, needle: &str) -> Option<Details> {
    let at = xml.find(needle)?;
    let start = xml[..at].rfind("<component")?;
    let end = at + xml[at..].find("</component>")?;
    Some(parse(&xml[start..end]))
}

fn parse(c: &str) -> Details {
    // Text of the developer block is not the app's name.
    let (dev, rest) = cut(c, "developer");
    let developer = dev
        .as_deref()
        .and_then(|d| plain(d, "name"))
        .or_else(|| plain(&rest, "developer_name"));
    let (shots, rest) = cut(&rest, "screenshots");
    Details {
        id: plain(&rest, "id").unwrap_or_default(),
        name: plain(&rest, "name").unwrap_or_default(),
        summary: plain(&rest, "summary").unwrap_or_default(),
        description: plain_raw(&rest, "description")
            .map(|d| paragraphs(&d))
            .unwrap_or_default(),
        developer,
        screenshots: shots.as_deref().map(screenshots).unwrap_or_default(),
        icon: cached_icon(&rest),
    }
}

/// (inner text of the first `<tag>…</tag>`, the rest without it).
fn cut(s: &str, tag: &str) -> (Option<String>, String) {
    let close = format!("</{tag}>");
    let start = [format!("<{tag}>"), format!("<{tag} ")]
        .iter()
        .filter_map(|o| s.find(o.as_str()))
        .min();
    let Some(a) = start else {
        return (None, s.to_owned());
    };
    let inner = a + s[a..].find('>').map_or(0, |i| i + 1);
    match s[inner..].find(&close) {
        Some(b) => (
            Some(s[inner..inner + b].to_owned()),
            format!("{}{}", &s[..a], &s[inner + b + close.len()..]),
        ),
        None => (None, s.to_owned()),
    }
}

/// Inner XML of the first untranslated `<tag>` (no attributes).
fn plain_raw(s: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let a = s.find(&open)? + open.len();
    let b = a + s[a..].find(&format!("</{tag}>"))?;
    Some(s[a..b].to_owned())
}

fn plain(s: &str, tag: &str) -> Option<String> {
    plain_raw(s, tag)
        .map(|t| squash(&unescape(&t)))
        .filter(|t| !t.is_empty())
}

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `<p>` and `<li>` blocks as text.
fn paragraphs(d: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = d;
    while let Some(i) = rest.find('<') {
        let tail = &rest[i..];
        let (tag, bullet) = if tail.starts_with("<p>") {
            ("p", "")
        } else if tail.starts_with("<li>") {
            ("li", "• ")
        } else {
            rest = &tail[1..];
            continue;
        };
        let open = tag.len() + 2;
        let Some(end) = tail.find(&format!("</{tag}>")) else {
            break;
        };
        let text = squash(&unescape(&strip_tags(&tail[open..end])));
        if !text.is_empty() {
            out.push(format!("{bullet}{text}"));
        }
        rest = &tail[end..];
    }
    out
}

fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut inside = false;
    for ch in s.chars() {
        match ch {
            '<' => inside = true,
            '>' => inside = false,
            c if !inside => out.push(c),
            _ => {}
        }
    }
    out
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let k = format!(" {name}=\"");
    let a = tag.find(&k)? + k.len();
    let b = a + tag[a..].find('"')?;
    Some(tag[a..b].to_owned())
}

/// Each `<screenshot>`: the thumbnail closest to [`SHOT_WIDTH`], else the source image.
fn screenshots(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    for shot in s.split("<screenshot").skip(1) {
        let shot = shot.split("</screenshot>").next().unwrap_or("");
        let mut best: Option<(u32, String)> = None;
        let mut source = None;
        for img in shot.split("<image").skip(1) {
            let Some((tag, rest)) = img.split_once('>') else {
                continue;
            };
            let url = unescape(rest.split("</image>").next().unwrap_or("").trim());
            if !url.starts_with("https://") {
                continue;
            }
            if attr(tag, "type").as_deref() == Some("source") {
                source = Some(url);
                continue;
            }
            let w: u32 = attr(tag, "width").and_then(|w| w.parse().ok()).unwrap_or(0);
            let d = w.abs_diff(SHOT_WIDTH);
            if best.as_ref().is_none_or(|(bd, _)| d < *bd) {
                best = Some((d, url));
            }
        }
        if let Some(u) = best.map(|b| b.1).or(source) {
            out.push(u);
        }
    }
    out
}

/// Largest `<icon type="cached">` (scale 1): (size, file name).
fn cached_icon(s: &str) -> Option<(u32, String)> {
    let mut best: Option<(u32, String)> = None;
    for icon in s.split("<icon ").skip(1) {
        let Some((tag, rest)) = icon.split_once('>') else {
            continue;
        };
        if attr(tag, "type").as_deref() != Some("cached") || attr(tag, "scale").is_some() {
            continue;
        }
        let file = unescape(rest.split("</icon>").next().unwrap_or("").trim());
        let size: u32 = attr(tag, "width").and_then(|w| w.parse().ok()).unwrap_or(0);
        if file.contains('/') || file.is_empty() {
            continue;
        }
        if best.as_ref().is_none_or(|(b, _)| size > *b) {
            best = Some((size, file));
        }
    }
    best
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let Some(end) = tail.find(';').filter(|e| *e <= 10) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let ent = &tail[1..end];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            e if e.starts_with("#x") => u32::from_str_radix(&e[2..], 16)
                .ok()
                .and_then(char::from_u32),
            e if e.starts_with('#') => e[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &tail[end + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<?xml version="1.0"?>
<components>
  <component type="desktop-application">
    <id>org.other.App</id>
    <name>Other</name>
  </component>
  <component type="desktop-application">
    <id>org.gimp.GIMP</id>
    <pkgname>gimp</pkgname>
    <name xml:lang="de">GNU Bildbearbeitung</name>
    <name>GNU Image Manipulation Program</name>
    <summary>High-end image creation &amp; manipulation</summary>
    <developer id="org.gimp">
      <name xml:lang="bg">Екипът</name>
      <name>The GIMP team</name>
    </developer>
    <description xml:lang="de"><p>Deutsch</p></description>
    <description>
      <p>
      GIMP is an acronym for <em>GNU</em> Image
      Manipulation Program.
    </p>
      <ul><li>Paint</li><li>Retouch &quot;photos&quot;</li></ul>
    </description>
    <icon height="64" type="cached" width="64">org.gimp.GIMP.png</icon>
    <icon height="128" type="cached" width="128">org.gimp.GIMP.png</icon>
    <icon height="128" scale="2" type="cached" width="128">org.gimp.GIMP.png</icon>
    <icon type="remote">https://example.org/x.png</icon>
    <screenshots>
      <screenshot type="default">
        <caption>Scene 4</caption>
        <image height="1080" type="source" width="1920">https://dl.flathub.org/a_orig.png</image>
        <image height="702" type="thumbnail" width="1248">https://dl.flathub.org/a_1248.png</image>
        <image height="423" type="thumbnail" width="752">https://dl.flathub.org/a_752.png</image>
        <image height="126" type="thumbnail" width="224">https://dl.flathub.org/a_224.png</image>
      </screenshot>
      <screenshot>
        <image type="source">https://dl.flathub.org/b_orig.png</image>
      </screenshot>
      <screenshot><image type="source">http://insecure/c.png</image></screenshot>
    </screenshots>
  </component>
</components>"#;

    #[test]
    fn reads_the_untranslated_details() {
        let d = by_id(XML, "org.gimp.GIMP").unwrap();
        assert_eq!(d.id, "org.gimp.GIMP");
        assert_eq!(d.name, "GNU Image Manipulation Program");
        assert_eq!(d.summary, "High-end image creation & manipulation");
        assert_eq!(d.developer.as_deref(), Some("The GIMP team"));
        assert_eq!(
            d.description,
            [
                "GIMP is an acronym for GNU Image Manipulation Program.",
                "• Paint",
                "• Retouch \"photos\""
            ]
        );
        assert_eq!(d.icon, Some((128, "org.gimp.GIMP.png".into())));
        assert_eq!(
            d.screenshots,
            [
                "https://dl.flathub.org/a_752.png",
                "https://dl.flathub.org/b_orig.png"
            ],
            "closest thumbnail, else source; https only"
        );
        assert_eq!(by_pkgname(XML, "gimp").unwrap().name, d.name);
        assert_eq!(by_id(XML, "org.other.App").unwrap().name, "Other");
        assert_eq!(by_id(XML, "org.none"), None);
    }

    #[test]
    fn developer_name_tag_and_entities() {
        let x = "<component><id>a.b.C</id><name>A &#x26; B&#33;</name><developer_name>Ann</developer_name></component>";
        let d = by_id(x, "a.b.C").unwrap();
        assert_eq!(d.name, "A & B!");
        assert_eq!(d.developer.as_deref(), Some("Ann"));
        assert!(d.screenshots.is_empty() && d.description.is_empty());
    }

    #[test]
    fn screenshots_cache_by_url() {
        let a = shot_cache("/home/ann/.cache", "https://dl.flathub.org/a_752.png").unwrap();
        assert!(a.starts_with("/home/ann/.cache/wolos/store/") && a.ends_with(".png"));
        assert_eq!(
            shot_cache("/c", "https://dl.flathub.org/a_752.png"),
            shot_cache("/c", "https://dl.flathub.org/a_752.png")
        );
        assert_ne!(
            shot_cache("/c", "https://x/a.png"),
            shot_cache("/c", "https://x/b.png")
        );
        assert!(
            shot_cache("/c", "https://x/a.JPG")
                .unwrap()
                .ends_with(".jpg")
        );
        assert!(shot_cache("/c", "https://x/a").unwrap().ends_with(".png"));
        assert_eq!(shot_cache("/c", "http://x/a.png"), None);
        assert_eq!(shot_cache("/c", "file:///etc/passwd"), None);
    }

    #[test]
    fn flathub_paths_user_first() {
        let [u, s] = flathub_paths("/home/ann");
        assert!(u.0.starts_with("/home/ann/.local/share/flatpak/appstream/flathub/"));
        assert!(u.0.ends_with("/active/appstream.xml"));
        assert!(s.1.starts_with("/var/lib/flatpak/appstream/flathub/"));
        assert!(s.1.ends_with("/active/icons"));
    }
}

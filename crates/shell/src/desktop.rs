//! Freedesktop `.desktop` entries: the apps Spotlight and the dock can launch.
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub exec: Vec<String>,
    pub icon: String,
}

/// None for hidden, non-application or Exec-less entries.
pub fn parse(id: &str, text: &str) -> Option<Entry> {
    let (mut name, mut exec, mut icon, mut in_main, mut app) =
        (None, None, String::new(), false, false);
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_main = line == "[Desktop Entry]";
            continue;
        }
        if !in_main {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        match k.trim() {
            "Name" => name = Some(v.trim().to_string()),
            "Exec" => exec = Some(v.trim().to_string()),
            "Icon" => icon = v.trim().to_string(),
            "Type" => app = v.trim() == "Application",
            "NoDisplay" | "Hidden" if v.trim() == "true" => return None,
            _ => {}
        }
    }
    let argv = exec_argv(&exec?);
    (app && !argv.is_empty()).then(|| Entry {
        id: id.into(),
        name: name.unwrap_or_else(|| id.into()),
        exec: argv,
        icon,
    })
}

/// Splits Exec, drops field codes (%f %U ...), honours double quotes.
pub fn exec_argv(exec: &str) -> Vec<String> {
    let (mut out, mut cur, mut quoted) = (Vec::new(), String::new(), false);
    for c in exec.chars() {
        match c {
            '"' => quoted = !quoted,
            ' ' if !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out.retain(|a| !(a.len() == 2 && a.starts_with('%')));
    out
}

/// XDG data dirs (+ /usr/local/share for FreeBSD), first id wins, sorted by name.
pub fn load_all() -> Vec<Entry> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Ok(h) = std::env::var("XDG_DATA_HOME") {
        dirs.push(h.into());
    } else if let Ok(h) = std::env::var("HOME") {
        dirs.push(Path::new(&h).join(".local/share"));
    }
    let sys =
        std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    dirs.extend(sys.split(':').filter(|s| !s.is_empty()).map(PathBuf::from));
    let mut seen = std::collections::HashSet::new();
    let mut files = Vec::new();
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(d.join("applications")) else {
            continue;
        };
        for f in rd.flatten() {
            let id = f.file_name().to_string_lossy().into_owned();
            if id.ends_with(".desktop") && seen.insert(id.clone()) {
                files.push((id, f.path()));
            }
        }
    }
    let mut out: Vec<Entry> = par_map(&files, |(id, p)| {
        std::fs::read_to_string(p).ok().and_then(|t| parse(id, &t))
    })
    .into_iter()
    .flatten()
    .collect();
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

/// Fork/join map, order kept (`shared::par`).
pub fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    shared::par::fold(
        items,
        16,
        |s| s.split_at(s.len() / 2),
        |s| s.iter().map(&f).collect::<Vec<R>>(),
        |mut a, b| {
            a.extend(b);
            a
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_application() {
        let e = parse("foot.desktop", "[Desktop Entry]\nType=Application\nName=Foot\nExec=foot --server %U\nIcon=foot\n[Desktop Action x]\nName=Other\n").unwrap();
        assert_eq!((e.name.as_str(), e.icon.as_str()), ("Foot", "foot"));
        assert_eq!(e.exec, ["foot", "--server"]);
    }

    #[test]
    fn skips_hidden_and_links() {
        assert!(
            parse(
                "a",
                "[Desktop Entry]\nType=Application\nName=A\nExec=a\nNoDisplay=true"
            )
            .is_none()
        );
        assert!(parse("b", "[Desktop Entry]\nType=Link\nName=B\nExec=b").is_none());
        assert!(parse("c", "[Desktop Entry]\nType=Application\nName=C").is_none());
    }

    #[test]
    fn par_map_keeps_order() {
        let v: Vec<u32> = (0..1000).collect();
        assert_eq!(
            par_map(&v, |x| x * 2),
            v.iter().map(|x| x * 2).collect::<Vec<_>>()
        );
    }

    #[test]
    fn exec_quotes() {
        assert_eq!(
            exec_argv(r#""/opt/My App/run" -x %f"#),
            ["/opt/My App/run", "-x"]
        );
    }
}

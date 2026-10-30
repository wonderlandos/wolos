//! `wol widgets`: what is on the desktop, add and remove, reload the shell, and validate a widget
//! package (archicture/modules/widgets.md). The layout file and packages go through
//! `wol_core::widgets`, the same code the desktop uses; a running shell rereads them on `reload`.
use crate::cli::Invocation;
use crate::run::{Env, SHELL_SOCKET, emit_json, fail};
use serde_json::{Value, json};
use shared::errors::ErrorCode;
use shared::widgets::grid::{Grid, METRICS};
use shared::widgets::manifest::Source;
use shared::widgets::{Builtin, CUSTOM_PREFIX, Desktop, Kind, Placed, Size};
use std::path::{Path, PathBuf};
use wol_core::widgets::{self as cw, Package};

pub fn execute(inv: &Invocation, env: &mut dyn Env) -> i32 {
    let as_json = inv.flag("json") || !env.stdout_tty();
    match inv.action {
        "widgets.list" => list(env, as_json),
        "widgets.add" => add(env, inv.get("kind").unwrap_or_default(), inv.get("size")),
        "widgets.remove" => remove(env, inv.get("id").unwrap_or_default()),
        "widgets.reload" => {
            if reload(env) {
                0
            } else {
                fail(env, ErrorCode::Unavailable, "the desktop is not running")
            }
        }
        "widgets.validate" => validate(env, inv.get("package").unwrap_or_default()),
        other => fail(env, ErrorCode::Failed, &format!("no handler for {other}")),
    }
}

fn data_home(env: &dyn Env) -> PathBuf {
    env.var("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| env.home().join(".local/share"))
}

fn packages(env: &dyn Env) -> Vec<Package> {
    cw::load_packages(&cw::package_dirs(&data_home(env)))
}

fn layout_path(env: &dyn Env) -> PathBuf {
    cw::desktop_path(&env.config_home())
}

/// The saved layout; a file that does not parse is an error (never replaced by a command).
fn read_desktop(env: &dyn Env) -> Result<Desktop, String> {
    match env.read_file(&layout_path(env)) {
        None => Ok(Desktop::default()),
        Some(text) => Desktop::parse(&text).map_err(|e| {
            format!(
                "{e}. Fix {} or move it away, then try again.",
                layout_path(env).display()
            )
        }),
    }
}

fn write_desktop(env: &mut dyn Env, d: &Desktop) -> Result<(), String> {
    let path = layout_path(env);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    env.write_file(&path, &d.to_json())
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// Asks a running desktop to reread widgets.json and the packages; false when none runs.
fn reload(env: &mut dyn Env) -> bool {
    let Some(rt) = env.var("XDG_RUNTIME_DIR") else {
        return false;
    };
    env.request(&Path::new(&rt).join(SHELL_SOCKET), b"reload\n")
        .is_ok()
}

fn kind_name(k: &Kind, pkgs: &[Package]) -> String {
    match k {
        Kind::Builtin(b) => b.name().into(),
        Kind::Custom(id) => pkgs
            .iter()
            .find(|p| &p.id == id)
            .map_or_else(|| id.clone(), |p| p.name().into()),
        Kind::Unknown(s) => s.clone(),
    }
}

/// Sizes a kind comes in (None: not installed, or its package has problems).
fn sizes_of(k: &Kind, pkgs: &[Package]) -> Option<Vec<Size>> {
    match k {
        Kind::Builtin(_) => Some(Size::ALL.to_vec()),
        Kind::Custom(id) => pkgs
            .iter()
            .find(|p| &p.id == id)
            .and_then(|p| p.manifest.as_ref().ok())
            .map(|m| m.sizes.clone()),
        Kind::Unknown(_) => None,
    }
}

fn size_list(s: &[Size]) -> String {
    s.iter().map(|s| s.id()).collect::<Vec<_>>().join(", ")
}

fn list(env: &mut dyn Env, as_json: bool) -> i32 {
    let desk = match read_desktop(env) {
        Ok(d) => d,
        Err(e) => return fail(env, ErrorCode::Invalid, &e),
    };
    let pkgs = packages(env);
    if as_json {
        let mut available: Vec<Value> = Builtin::ALL
            .iter()
            .map(|b| {
                json!({"kind": b.id(), "id": b.id(), "name": b.name(),
                       "description": b.description(), "sizes": size_ids(&Size::ALL),
                       "builtin": true, "valid": true})
            })
            .collect();
        for p in &pkgs {
            let (description, sizes, problems) = match &p.manifest {
                Ok(m) => (m.description.clone(), size_ids(&m.sizes), Vec::new()),
                Err(ps) => (
                    String::new(),
                    Vec::new(),
                    ps.iter().map(|x| x.to_string()).collect(),
                ),
            };
            available.push(json!({
                "kind": format!("{CUSTOM_PREFIX}{}", p.id), "id": p.id, "name": p.name(),
                "description": description, "sizes": sizes, "builtin": false,
                "system": p.system, "dir": p.dir, "valid": p.manifest.is_ok(),
                "problems": problems,
            }));
        }
        emit_json(
            env,
            &json!({"placed": desk.widgets, "available": available}),
        );
        return 0;
    }
    let mut out = String::new();
    if desk.widgets.is_empty() {
        out.push_str("No widgets on the desktop. Add one: wol widgets add clock\n");
    } else {
        out.push_str("On the desktop:\n");
        out.push_str(&format!(
            "  {:<16} {:<16} {:<7} {}\n",
            "ID", "WIDGET", "SIZE", "CELL"
        ));
        for w in &desk.widgets {
            out.push_str(&format!(
                "  {:<16} {:<16} {:<7} {},{}\n",
                w.id,
                kind_name(&w.kind, &pkgs),
                w.size.id(),
                w.col,
                w.row
            ));
        }
    }
    out.push_str("\nYou can add:\n");
    for b in Builtin::ALL {
        out.push_str(&format!(
            "  {:<16} {:<16} {}\n",
            b.id(),
            b.name(),
            b.description()
        ));
    }
    for p in &pkgs {
        let about = match &p.manifest {
            Ok(m) => m.description.clone(),
            Err(ps) => format!(
                "{} problem{}: wol widgets validate {}",
                ps.len(),
                if ps.len() == 1 { "" } else { "s" },
                p.id
            ),
        };
        out.push_str(&format!("  {:<16} {:<16} {about}\n", p.id, p.name()));
    }
    env.out(&out);
    0
}

fn size_ids(s: &[Size]) -> Vec<&'static str> {
    s.iter().map(|s| s.id()).collect()
}

/// `clock`, `game-friends` or `custom:game-friends`.
fn parse_kind(s: &str, pkgs: &[Package]) -> Result<Kind, String> {
    if let Some(b) = Builtin::parse(s) {
        return Ok(Kind::Builtin(b));
    }
    let id = s.strip_prefix(CUSTOM_PREFIX).unwrap_or(s);
    match pkgs.iter().find(|p| p.id == id) {
        Some(p) if p.manifest.is_err() => Err(format!(
            "{} has problems; see wol widgets validate {}",
            p.name(),
            p.id
        )),
        Some(p) => Ok(Kind::Custom(p.id.clone())),
        None => {
            let names: Vec<&str> = Builtin::ALL
                .iter()
                .map(|b| b.id())
                .chain(pkgs.iter().map(|p| p.id.as_str()))
                .collect();
            Err(format!(
                "no widget named '{s}'. Try one of: {}",
                names.join(", ")
            ))
        }
    }
}

fn add(env: &mut dyn Env, kind: &str, size: Option<&str>) -> i32 {
    let pkgs = packages(env);
    let kind = match parse_kind(kind, &pkgs) {
        Ok(k) => k,
        Err(e) => return fail(env, ErrorCode::NotFound, &e),
    };
    let offered = sizes_of(&kind, &pkgs).unwrap_or_default();
    let size = match size {
        None if offered.contains(&Size::Small) => Size::Small,
        None => match offered.first() {
            Some(s) => *s,
            None => return fail(env, ErrorCode::Invalid, "this widget offers no size"),
        },
        Some(s) => match Size::parse(s) {
            Some(s) if offered.contains(&s) => s,
            Some(_) => {
                let name = kind_name(&kind, &pkgs);
                return fail(
                    env,
                    ErrorCode::Invalid,
                    &format!("{name} comes in {}", size_list(&offered)),
                );
            }
            None => return fail(env, ErrorCode::Usage, "size is small, medium or large"),
        },
    };
    let mut desk = match read_desktop(env) {
        Ok(d) => d,
        Err(e) => return fail(env, ErrorCode::Invalid, &e),
    };
    let Some((col, row)) = Grid::headless(METRICS).free_spot(&desk.widgets, None, size, (0, 0))
    else {
        return fail(
            env,
            ErrorCode::Failed,
            "no room on the desktop. Remove a widget or pick a smaller size.",
        );
    };
    let id = desk.new_id(&kind);
    let name = kind_name(&kind, &pkgs);
    desk.widgets.push(Placed {
        id: id.clone(),
        kind,
        size,
        col,
        row,
        options: Default::default(),
        display: None,
    });
    if let Err(e) = write_desktop(env, &desk) {
        return fail(env, ErrorCode::Failed, &e);
    }
    let when = if reload(env) {
        "It is on the desktop now."
    } else {
        "It shows up the next time the desktop starts."
    };
    env.out(&format!("Added {name} ({}) as {id}. {when}\n", size.id()));
    0
}

fn remove(env: &mut dyn Env, id: &str) -> i32 {
    let mut desk = match read_desktop(env) {
        Ok(d) => d,
        Err(e) => return fail(env, ErrorCode::Invalid, &e),
    };
    if desk.remove(id).is_none() {
        let ids: Vec<&str> = desk.widgets.iter().map(|w| w.id.as_str()).collect();
        let on = if ids.is_empty() {
            "the desktop has none".to_string()
        } else {
            ids.join(", ")
        };
        return fail(
            env,
            ErrorCode::NotFound,
            &format!("no widget with id '{id}' on the desktop ({on})"),
        );
    }
    if let Err(e) = write_desktop(env, &desk) {
        return fail(env, ErrorCode::Failed, &e);
    }
    reload(env);
    env.out(&format!("Removed {id}.\n"));
    0
}

/// A package by folder (`./my-widget`, an absolute path) or by installed id.
fn find_package(env: &dyn Env, arg: &str) -> Option<Package> {
    let p = Path::new(arg);
    if arg.contains('/') || p.join(shared::widgets::MANIFEST_FILE).is_file() {
        let dir = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        return dir.is_dir().then(|| cw::load_package(&dir, false));
    }
    packages(env).into_iter().find(|x| x.id == arg)
}

fn validate(env: &mut dyn Env, arg: &str) -> i32 {
    let Some(pkg) = find_package(env, arg) else {
        return fail(
            env,
            ErrorCode::NotFound,
            &format!("no widget package '{arg}' (an id from wol widgets, or a folder)"),
        );
    };
    let m = match &pkg.manifest {
        Ok(m) => m,
        Err(problems) => {
            let mut out = format!(
                "{}: {} problem{} in {}\n",
                pkg.name(),
                problems.len(),
                if problems.len() == 1 { "" } else { "s" },
                pkg.dir.join(shared::widgets::MANIFEST_FILE).display()
            );
            for p in problems {
                out.push_str(&format!("  {p}\n"));
            }
            env.out(&out);
            return ErrorCode::Invalid.exit();
        }
    };
    let mut issues: Vec<String> = Vec::new();
    if let Some(sample) = &m.sample {
        issues.extend(
            m.unresolved(sample)
                .into_iter()
                .map(|u| format!("sample: {u}")),
        );
    }
    let mut ran = String::new();
    if let Some(src) = &m.source {
        let desk = read_desktop(env).unwrap_or_default();
        let id = pkg.id.clone();
        let host = src.host();
        if let (Source::Http { .. }, Some(h)) = (src, &host)
            && !desk.allows(&id, h)
        {
            ran = format!(" The source reads {h}; it runs once you allow it on the widget's card.");
        } else {
            let t0 = std::time::Instant::now();
            match cw::read_source(src, &pkg.dir, &env.home(), |h| desk.allows(&id, h)) {
                Ok(data) => {
                    ran = format!(" The source ran in {} ms.", t0.elapsed().as_millis());
                    issues.extend(
                        m.unresolved(&data)
                            .into_iter()
                            .map(|u| format!("source output: {u}")),
                    );
                }
                Err(e) => issues.push(format!("source: {}", e.message())),
            }
        }
    }
    if issues.is_empty() {
        env.out(&format!(
            "{} is valid: {}.{ran}\n",
            m.name,
            size_list(&m.sizes)
        ));
        return 0;
    }
    let mut out = format!(
        "{}: {} problem{} (the widget.json itself is valid):\n",
        m.name,
        issues.len(),
        if issues.len() == 1 { "" } else { "s" }
    );
    for i in &issues {
        out.push_str(&format!("  {i}\n"));
    }
    env.out(&out);
    ErrorCode::Invalid.exit()
}

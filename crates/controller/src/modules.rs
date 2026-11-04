// SPDX-License-Identifier: GPL-2.0-or-later
//! `wol modules list|status|install|remove|enable` (ADR 0032, docs/modules/modules.md). Registry
//! and state logic in `wol_core::modules`; packages change through wol-admind (`pkg-install`,
//! `pkg-remove` of `wolos-module-<id>`, the administrator password as for `wol restore activate`);
//! the user-side step (`enable`: wopkg recipes, mise tools, the record's `post` argv) runs here as
//! the person and is recorded in `$XDG_STATE_HOME/wolos/modules.json`.
use crate::cli::Invocation;
use crate::run::{Env, admin_send, emit_json, fail};
use serde_json::json;
use shared::admin::{AdminAction, AdminRequest};
use shared::errors::ErrorCode;
use std::path::{Path, PathBuf};
use wol_core::modules::{self, Enabled, Facts, Module, Status};

/// Overrides for tests and images: registry directory, pending file.
pub const DIR_VAR: &str = "WOL_MODULES_DIR";
pub const PENDING_VAR: &str = "WOL_MODULES_PENDING";

pub fn execute(inv: &Invocation, env: &mut dyn Env) -> i32 {
    let json = inv.flag("json") || !env.stdout_tty();
    let id = inv.get("id").unwrap_or_default().to_string();
    match inv.action {
        "modules.list" => list(env, json),
        "modules.status" => status(env, inv.get("id"), json),
        "modules.install" => install(env, &id, json),
        "modules.remove" => remove(env, &id, json),
        "modules.enable" => enable(env, &id, json),
        other => fail(env, ErrorCode::Failed, &format!("no handler for {other}")),
    }
}

fn dir(env: &dyn Env) -> PathBuf {
    env.var(DIR_VAR)
        .map_or_else(|| PathBuf::from(modules::DIR), PathBuf::from)
}

fn pending_path(env: &dyn Env) -> PathBuf {
    env.var(PENDING_VAR)
        .map_or_else(modules::pending_file, PathBuf::from)
}

/// Every record in the registry directory.
fn registry(env: &dyn Env) -> Vec<Module> {
    let d = dir(env);
    modules::load(&d, &|p| env.list_dir(p), &|p| env.read_file(p))
}

/// Installed packages (`pacman -Qq`), the user's enablement file, the pending ids.
fn facts(env: &mut dyn Env) -> Facts {
    let installed = env
        .capture(&["pacman", "-Qq"])
        .map(|t| modules::installed_packages(&t))
        .unwrap_or_default();
    let enabled = Enabled::parse(env.read_file(&state_file(env)).as_deref());
    let pending = modules::parse_pending(env.read_file(&pending_path(env)).as_deref());
    Facts {
        installed,
        enabled,
        pending,
    }
}

fn state_file(env: &dyn Env) -> PathBuf {
    modules::state_file(&env.state_home())
}

/// The record for `id`, or the error already reported (exit code).
fn find(env: &mut dyn Env, id: &str) -> Result<Module, i32> {
    if !modules::valid_id(id) {
        return Err(fail(
            env,
            ErrorCode::Usage,
            &format!("bad module id '{id}'"),
        ));
    }
    let reg = registry(env);
    match reg.into_iter().find(|m| m.id == id) {
        Some(m) => Ok(m),
        None => Err(fail(
            env,
            ErrorCode::NotFound,
            &format!("no module '{id}' (wol modules list shows them)"),
        )),
    }
}

fn table(rows: &[Status]) -> String {
    let mut t = format!("{:<12} {:<10} {:>8}  {}\n", "ID", "STATE", "SIZE", "NAME");
    for s in rows {
        t.push_str(&format!(
            "{:<12} {:<10} {:>5} MB  {}\n",
            s.id,
            s.state(),
            s.size_mb,
            s.name
        ));
        if s.installed && !s.enabled && s.needs_enable {
            t.push_str(&format!(
                "{:<12} finish setup: wol modules enable {}\n",
                "", s.id
            ));
        }
    }
    t
}

fn list(env: &mut dyn Env, json: bool) -> i32 {
    let reg = registry(env);
    let f = facts(env);
    let mut rows = modules::list(&reg, &f);
    if json {
        for (s, m) in rows.iter_mut().zip(&reg) {
            s.backend = backend_for(env, m);
        }
        emit_json(env, &serde_json::to_value(&rows).unwrap_or_default());
    } else if rows.is_empty() {
        env.out("no optional modules on this computer\n");
    } else {
        env.out(&table(&rows));
    }
    0
}

fn status(env: &mut dyn Env, id: Option<&str>, json: bool) -> i32 {
    let Some(id) = id else {
        return list(env, json);
    };
    let m = match find(env, id) {
        Ok(m) => m,
        Err(code) => return code,
    };
    let f = facts(env);
    let mut s = modules::status(&m, &f);
    if json {
        s.backend = backend_for(env, &m);
        emit_json(env, &serde_json::to_value(&s).unwrap_or_default());
    } else {
        let mut t = format!("{} ({}): {}\n{}\n", s.name, s.id, s.state(), s.summary);
        if !m.packages.is_empty() {
            t.push_str(&format!("packages: {}\n", m.packages.join(" ")));
        }
        if !m.recipes.is_empty() {
            t.push_str(&format!("recipes: {}\n", m.recipes.join(" ")));
        }
        if !m.needs.is_empty() {
            t.push_str(&format!("needs: {}\n", m.needs.join(", ")));
        }
        t.push_str(&format!("download: {} MB\n", s.size_mb));
        env.out(&t);
    }
    0
}

/// The record's backend package for this machine (`wol hw compute`); the hardware is only read
/// for a record that has a `backend`.
fn backend_for(env: &mut dyn Env, m: &Module) -> Option<String> {
    m.backend.as_ref()?;
    let snap = env.hardware();
    let c = crate::hw::compute(env, &snap);
    m.backend_package(c.backend)
}

fn pkg_request(action: AdminAction, m: &Module) -> AdminRequest {
    AdminRequest {
        action,
        args: vec![m.package()],
    }
}

/// Administrator: `pacman -S` the meta package through wol-admind. The user-side step is separate
/// (`enable`) so the same two steps serve the CLI, Settings and, later, helpers. `json`: one object
/// `{id, installed, was_installed, needs_enable}` instead of text (audit S4-9).
fn install(env: &mut dyn Env, id: &str, json: bool) -> i32 {
    let m = match find(env, id) {
        Ok(m) => m,
        Err(code) => return code,
    };
    let f = facts(env);
    let before = modules::status(&m, &f);
    // The record's per-machine package (`backend`, ADR 0032): `<stem>-<cuda|rocm|vulkan>` from
    // `wol hw compute`, none on a CPU-only machine. Installed with the meta package, or alone
    // when the meta package is there and the backend is not. The backend goes first, as in
    // Settings (`install_packages` in the shell): a failed backend leaves the module not
    // installed, where the other order would leave one that looks ready without it.
    let backend = backend_for(env, &m);
    let backend_missing = backend.clone().filter(|p| !f.installed.contains(p));
    if let Some(p) = &backend_missing {
        let req = AdminRequest {
            action: AdminAction::PkgInstall,
            args: vec![p.clone()],
        };
        let code = admin_send(env, &req);
        if code != 0 {
            return code;
        }
    }
    if !before.installed {
        let code = admin_send(env, &pkg_request(AdminAction::PkgInstall, &m));
        if code != 0 {
            return code;
        }
    }
    let needs_enable = m.has_user_step() && !before.enabled;
    if json {
        emit_json(
            env,
            &json!({"id": m.id, "installed": true, "was_installed": before.installed,
                    "backend": backend, "needs_enable": needs_enable}),
        );
        return 0;
    }
    if before.installed && backend_missing.is_none() {
        env.out(&format!("{} is already installed\n", m.name));
    } else if before.installed {
        env.out(&format!(
            "installed {} for {}\n",
            backend_missing.as_deref().unwrap_or_default(),
            m.name
        ));
    } else {
        env.out(&format!("installed {} ({})\n", m.name, m.package()));
    }
    if needs_enable {
        env.out(&format!("finish setup: wol modules enable {}\n", m.id));
    }
    0
}

/// The user-side undo of `enable`, in reverse order: the record's `unpost`, Flatpaks, mise tools,
/// then the wopkg recipes. Each idempotent (audit C8-6).
fn undo_steps(m: &Module) -> Vec<Vec<String>> {
    let mut steps: Vec<Vec<String>> = Vec::new();
    if !m.unpost.is_empty() {
        steps.push(m.unpost.clone());
    }
    for a in m.flatpaks.iter().rev() {
        steps.push(vec![
            "flatpak".into(),
            "uninstall".into(),
            "--user".into(),
            "-y".into(),
            "--noninteractive".into(),
            a.clone(),
        ]);
    }
    for t in m.mise.iter().rev() {
        steps.push(vec!["mise".into(), "unuse".into(), "-g".into(), t.clone()]);
    }
    for r in m.recipes.iter().rev() {
        steps.push(vec!["wopkg".into(), "remove".into(), r.clone()]);
    }
    steps
}

/// Undo `enable` and `install` in reverse: the user-side steps ([`undo_steps`], each reported),
/// then, through wol-admind, `pacman -Rs` of the record's installed backend packages (ai-local:
/// `ollama-<gpu>`, installed explicitly) and of the meta package. A step that does not finish
/// keeps the enablement record and fails the verb (audit S4-9), so `remove` can be run again.
fn remove(env: &mut dyn Env, id: &str, json: bool) -> i32 {
    let m = match find(env, id) {
        Ok(m) => m,
        Err(code) => return code,
    };
    let f = facts(env);
    let before = modules::status(&m, &f);
    let mut failed: Vec<String> = Vec::new();
    let mut undone: Vec<String> = Vec::new();
    if before.enabled {
        for argv in undo_steps(&m) {
            let line = argv.join(" ");
            if matches!(env.exec(&argv), Ok(0)) {
                if !json {
                    env.out(&format!("undone: {line}\n"));
                }
                undone.push(line);
            } else {
                // A recipe is named by itself (the S4-9 contract), any other step by its argv.
                let what = match argv.as_slice() {
                    [w, r, name] if w == "wopkg" && r == "remove" => name.clone(),
                    _ => line,
                };
                env.err(&format!("wol: {what}: did not finish\n"));
                failed.push(what);
            }
        }
    }
    let backends = m.installed_backends(&f.installed);
    for p in &backends {
        let req = AdminRequest {
            action: AdminAction::PkgRemove,
            args: vec![p.clone()],
        };
        let code = admin_send(env, &req);
        if code != 0 {
            return code;
        }
        if !json {
            env.out(&format!("removed {p}\n"));
        }
    }
    if before.installed {
        let code = admin_send(env, &pkg_request(AdminAction::PkgRemove, &m));
        if code != 0 {
            return code;
        }
    }
    if before.enabled && failed.is_empty() {
        let mut e = f.enabled.clone();
        if e.set(&m.id, false) {
            write_state(env, &e);
        }
    }
    if json {
        emit_json(
            env,
            &json!({"id": m.id, "removed": failed.is_empty(), "was_installed": before.installed,
                    "backends": backends, "undone": undone, "failed": failed}),
        );
    } else if !failed.is_empty() {
        env.out(&format!(
            "{} is only partly removed: run wol modules remove {} again\n",
            m.name, m.id
        ));
    } else if before.installed {
        env.out(&format!("removed {} ({})\n", m.name, m.package()));
    } else if backends.is_empty() {
        env.out(&format!("{} was not installed\n", m.name));
    }
    if failed.is_empty() {
        0
    } else {
        ErrorCode::Failed.exit()
    }
}

/// QA-PLUG-28: a step of `enable` that failed, in words: what did not come (a download names the
/// network), how to try again, then the exact step for whoever helps (`detail`).
fn step_failed(m: &modules::Module, argv: &[String], detail: &str) -> String {
    let download = matches!(
        argv.first().map(String::as_str),
        Some("wopkg" | "mise" | "flatpak")
    );
    let what = argv.last().map_or("", String::as_str);
    let lead = if download {
        format!(
            "{} could not finish setting up: {what} did not download. Check that you are online, then try again",
            m.name
        )
    } else {
        format!("{} could not finish its setup step. Try again", m.name)
    };
    format!("{lead}: wol modules enable {} ({detail}).", m.id)
}

/// QA-PLUG-79: the sentence `enable` would end with for a module that is not installed, for a
/// caller that wants it before asking the person anything (the MCP server: a sheet for a change
/// that has nothing to do is a question for nothing). None: installed, or no such module (the
/// verb says so itself).
pub(crate) fn not_installed_sentence(env: &mut dyn Env, id: &str) -> Option<String> {
    if !modules::valid_id(id) {
        return None;
    }
    let m = registry(env).into_iter().find(|m| m.id == id)?;
    let f = facts(env);
    (!modules::status(&m, &f).installed).then(|| {
        format!(
            "{} is not installed yet: wol modules install {}",
            m.name, m.id
        )
    })
}

/// Why `enable` refuses root (QA-PLUG-60): its steps set up the person's own account.
fn root_refusal(env: &mut dyn Env) -> Option<i32> {
    env.is_root().then(|| {
        fail(
            env,
            ErrorCode::Denied,
            &format!("wol modules enable: {}", crate::mcp::AS_ROOT),
        )
    })
}

/// As the user: wopkg recipes, mise tools, Flatpaks, then the record's `post` argv; all idempotent
/// by contract. Recorded in the enablement file only when every step passed.
fn enable(env: &mut dyn Env, id: &str, json: bool) -> i32 {
    if let Some(code) = root_refusal(env) {
        return code;
    }
    let m = match find(env, id) {
        Ok(m) => m,
        Err(code) => return code,
    };
    let f = facts(env);
    let s = modules::status(&m, &f);
    if !s.installed {
        return fail(
            env,
            ErrorCode::NotFound,
            &format!(
                "{} is not installed yet: wol modules install {}",
                m.name, m.id
            ),
        );
    }
    let mut steps: Vec<Vec<String>> = Vec::new();
    for r in &m.recipes {
        steps.push(vec!["wopkg".into(), "install".into(), r.clone()]);
    }
    for t in &m.mise {
        steps.push(vec!["mise".into(), "use".into(), "-g".into(), t.clone()]);
    }
    for a in &m.flatpaks {
        steps.push(vec![
            "flatpak".into(),
            "install".into(),
            "--user".into(),
            "-y".into(),
            "--noninteractive".into(),
            "flathub".into(),
            a.clone(),
        ]);
    }
    if !m.post.is_empty() {
        steps.push(m.post.clone());
    }
    let mut ran: Vec<String> = Vec::new();
    for argv in &steps {
        let line = argv.join(" ");
        match env.exec(argv) {
            Ok(0) => ran.push(line),
            Ok(c) => {
                let msg = step_failed(&m, argv, &format!("`{line}` exited {c}"));
                return fail(env, ErrorCode::Failed, &msg);
            }
            Err(e) => {
                let msg = step_failed(&m, argv, &format!("`{line}`: {e}"));
                return fail(env, ErrorCode::Unavailable, &msg);
            }
        }
    }
    let mut e = f.enabled.clone();
    if e.set(&m.id, true) {
        write_state(env, &e);
    }
    if json {
        emit_json(env, &json!({"id": m.id, "enabled": true, "ran": ran}));
    } else if ran.is_empty() {
        env.out(&format!(
            "{} is ready; nothing to set up for this user\n",
            m.name
        ));
    } else {
        env.out(&format!("{} is set up for this user\n", m.name));
    }
    0
}

fn write_state(env: &mut dyn Env, e: &Enabled) {
    let path = state_file(env);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(err) = env.write_file(Path::new(&path), &e.to_json()) {
        env.err(&format!("wol: {}: {err}\n", path.display()));
    }
}

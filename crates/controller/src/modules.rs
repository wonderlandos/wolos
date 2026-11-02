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
        "modules.install" => install(env, &id),
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
    let rows = modules::list(&reg, &f);
    if json {
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
    let s = modules::status(&m, &f);
    if json {
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

fn pkg_request(action: AdminAction, m: &Module) -> AdminRequest {
    AdminRequest {
        action,
        args: vec![m.package()],
    }
}

/// Administrator: `pacman -S` the meta package through wol-admind. The user-side step is separate
/// (`enable`) so the same two steps serve the CLI, Settings and, later, helpers.
fn install(env: &mut dyn Env, id: &str) -> i32 {
    let m = match find(env, id) {
        Ok(m) => m,
        Err(code) => return code,
    };
    let f = facts(env);
    let before = modules::status(&m, &f);
    if before.installed {
        env.out(&format!("{} is already installed\n", m.name));
    } else {
        let code = admin_send(env, &pkg_request(AdminAction::PkgInstall, &m));
        if code != 0 {
            return code;
        }
        env.out(&format!("installed {} ({})\n", m.name, m.package()));
    }
    if m.has_user_step() && !before.enabled {
        env.out(&format!("finish setup: wol modules enable {}\n", m.id));
    }
    0
}

/// Administrator: `pacman -Rs` the meta package through wol-admind, then the user-side parts
/// (`wopkg remove` each recipe) and the enablement record.
fn remove(env: &mut dyn Env, id: &str, json: bool) -> i32 {
    let m = match find(env, id) {
        Ok(m) => m,
        Err(code) => return code,
    };
    let f = facts(env);
    let before = modules::status(&m, &f);
    if before.installed {
        let code = admin_send(env, &pkg_request(AdminAction::PkgRemove, &m));
        if code != 0 {
            return code;
        }
    }
    let mut failed: Vec<String> = Vec::new();
    if before.enabled {
        for r in &m.recipes {
            let argv = vec!["wopkg".to_string(), "remove".into(), r.clone()];
            if !matches!(env.exec(&argv), Ok(0)) {
                failed.push(r.clone());
            }
        }
        let mut e = f.enabled.clone();
        if e.set(&m.id, false) {
            write_state(env, &e);
        }
    }
    for r in &failed {
        env.err(&format!("wol: {r}: wopkg remove did not finish\n"));
    }
    if json {
        emit_json(
            env,
            &json!({"id": m.id, "removed": true, "was_installed": before.installed, "failed": failed}),
        );
    } else if before.installed {
        env.out(&format!("removed {} ({})\n", m.name, m.package()));
    } else {
        env.out(&format!("{} was not installed\n", m.name));
    }
    0
}

/// As the user: wopkg recipes, mise tools, Flatpaks, then the record's `post` argv; all idempotent
/// by contract. Recorded in the enablement file only when every step passed.
fn enable(env: &mut dyn Env, id: &str, json: bool) -> i32 {
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
                return fail(
                    env,
                    ErrorCode::Failed,
                    &format!("{}: `{line}` failed (exit {c})", m.name),
                );
            }
            Err(e) => {
                return fail(
                    env,
                    ErrorCode::Unavailable,
                    &format!("{}: `{line}`: {e}", m.name),
                );
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

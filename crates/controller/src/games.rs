//! `wol games`: the games doctor and one-click Steam. Logic in
//! `wol_core::games` (archicture/modules/games.md).
use crate::cli::Invocation;
use crate::run::{Env, fail};
use shared::errors::ErrorCode;
use wol_core::games::doctor::{self, Check, Status};
use wol_core::games::notice;

/// Steam's App Store icon (design/icons/store, installed by image/build.sh).
pub const STEAM_ICON: &str = "/usr/share/wolos/store-icons/steam.svg";

pub fn execute(inv: &Invocation, env: &mut dyn Env) -> i32 {
    let json = inv.flag("json") || !env.stdout_tty();
    match inv.action {
        "games.doctor" => doctor(env, json),
        "games.steam" => launch(env, inv.get("url")),
        other => fail(env, ErrorCode::Failed, &format!("no handler for {other}")),
    }
}

fn doctor(env: &mut dyn Env, json: bool) -> i32 {
    let all = doctor::checks(&env.games_facts());
    let ready = doctor::ready(&all);
    if json {
        let v = serde_json::json!({
            "ready": ready,
            "steam_ready": doctor::steam_blockers(&all).is_empty(),
            "checks": all,
        });
        env.out(&format!(
            "{}\n",
            serde_json::to_string_pretty(&v).unwrap_or_default()
        ));
    } else {
        env.out(&report(&all));
    }
    if ready { 0 } else { ErrorCode::Failed.exit() }
}

/// One line per check, its fix under it, then the verdict.
pub fn report(all: &[Check]) -> String {
    let mut t = String::new();
    for c in all {
        t.push_str(&format!(
            "{:<5} {:<20} {}\n",
            c.status.name(),
            c.id,
            c.detail
        ));
        if !c.fix.is_empty() && c.status != Status::Ok {
            t.push_str(&format!("{:<26} fix: {}\n", "", c.fix));
        }
    }
    let failed = all.iter().filter(|c| c.status == Status::Fail).count();
    t.push_str(&if failed == 0 {
        "Ready: Steam and Windows games can run.\n".to_string()
    } else {
        format!("Not ready: {failed} problem(s). Run the fix lines above, then check again.\n")
    });
    t
}

/// Started from the Dock, Launchpad or Spotlight nobody reads stdout: say it in a notification.
fn notify(env: &mut dyn Env, (title, body): notice::Notice) {
    if env.stdout_tty() {
        return;
    }
    let icon = format!("--icon={STEAM_ICON}");
    let argv = [
        "notify-send",
        "--app-name=Steam",
        icon.as_str(),
        "--",
        title,
        body,
    ];
    let _ = env.exec(&argv.map(String::from));
}

/// Preflight, then start the native Steam client (Proton comes through Steam).
fn launch(env: &mut dyn Env, url: Option<&str>) -> i32 {
    if env.is_root() {
        return fail(
            env,
            ErrorCode::Denied,
            "Steam runs as your account, not as root",
        );
    }
    let facts = env.games_facts();
    let all = doctor::checks(&facts);
    let blockers = doctor::steam_blockers(&all);
    if !blockers.is_empty() {
        let mut msg = String::from("wol: Steam cannot start yet:\n");
        for c in &blockers {
            msg.push_str(&format!("  {}\n    fix: {}\n", c.detail, c.fix));
        }
        env.err(&msg);
        notify(env, notice::blocked(&blockers));
        return ErrorCode::Unavailable.exit();
    }
    // Native Steam downloads its client itself the first time, in its own updater window.
    if !facts.steam_bootstrapped {
        if env.stdout_tty() {
            env.out("Getting Steam ready: the first start downloads the Steam client.\n");
        }
        notify(env, notice::PREPARING);
    }
    let mut argv = vec![doctor::STEAM_BIN.to_string()];
    argv.extend(url.map(String::from));
    crate::run::exec(env, &argv)
}

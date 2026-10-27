//! `wol games`: the games doctor, the WolOS Proton tool for Steam, and one-click Steam. Logic in
//! `wol_core::games` (archicture/modules/games.md).
use crate::cli::Invocation;
use crate::run::{Env, fail};
use shared::errors::ErrorCode;
use std::path::PathBuf;
use wol_core::games::doctor::{self, Check, Status};
use wol_core::games::notice;
use wol_core::games::steam::{self, Installed};

/// lsu-bootstrap refuses accounts in group wheel without this flag. A WolOS account is the
/// machine's administrator, and Steam games run as that person on every desktop OS anyway.
pub const LSU_WHEEL_FLAG: &str = "--allow-stealing-my-passwords,-browser-history-and-ssh-keys";

/// Steam's App Store icon (design/icons/store, installed by image/build.sh).
pub const STEAM_ICON: &str = "/usr/local/share/wolos/store-icons/steam.svg";

pub fn lsu_bootstrap() -> String {
    format!("{}/bin/lsu-bootstrap", steam::LSU)
}

pub fn lsu_steam() -> String {
    format!("{}/bin/steam", steam::LSU)
}

pub fn execute(inv: &Invocation, env: &mut dyn Env) -> i32 {
    let json = inv.flag("json") || !env.stdout_tty();
    match inv.action {
        "games.doctor" => doctor(env, json),
        "games.steam-setup" => setup(env),
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

fn tool_dir(env: &dyn Env) -> Option<(PathBuf, PathBuf)> {
    let home = env.home();
    if home.as_os_str().is_empty() {
        return None;
    }
    let root = steam::steam_root(&home, &|p| p.exists());
    Some((steam::tool_dir(&root), root))
}

fn setup(env: &mut dyn Env) -> i32 {
    let Some((dir, root)) = tool_dir(env) else {
        return fail(env, ErrorCode::Usage, "HOME is not set");
    };
    match steam::install(&root, steam::WOL_RUN) {
        Ok(Installed::Current) => {
            env.out(&format!(
                "WolOS Proton is up to date in {}\n",
                dir.display()
            ));
            0
        }
        Ok(done) => {
            let verb = if done == Installed::Created {
                "Installed"
            } else {
                "Updated"
            };
            env.out(&format!(
                "{verb} WolOS Proton in {}.\nRestart Steam, then pick \"{}\" in a game's Properties > Compatibility.\n",
                dir.display(),
                steam::DISPLAY_NAME
            ));
            0
        }
        Err(e) => fail(
            env,
            ErrorCode::Failed,
            &format!("cannot write {}: {e}", dir.display()),
        ),
    }
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

/// Preflight, download the client the first time, refresh WolOS Proton, start Steam.
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
    if !facts.steam_bootstrapped {
        if env.stdout_tty() {
            env.out("Getting Steam ready: the first start downloads the Steam client.\n");
        }
        notify(env, notice::PREPARING);
        let mut argv = vec![lsu_bootstrap()];
        if facts.groups.iter().any(|g| g == "wheel") {
            argv.push(LSU_WHEEL_FLAG.into());
        }
        let why = match env.exec(&argv) {
            Ok(0) => None,
            Ok(c) => Some(format!("lsu-bootstrap exited {c}")),
            Err(e) => Some(format!("{}: {e}", argv[0])),
        };
        if let Some(why) = why {
            notify(env, notice::NO_DOWNLOAD);
            return fail(
                env,
                ErrorCode::Unavailable,
                &format!("the Steam client did not download ({why})"),
            );
        }
    }
    if let Some((dir, root)) = tool_dir(env)
        && let Err(e) = steam::install(&root, steam::WOL_RUN)
    {
        env.err(&format!(
            "wol: WolOS Proton not refreshed ({}: {e}); Steam starts anyway\n",
            dir.display()
        ));
    }
    let mut argv = vec![lsu_steam()];
    argv.extend(url.map(String::from));
    crate::run::exec(env, &argv)
}

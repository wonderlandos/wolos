//! `wol-app <verb>`: menu bar app/Window menu actions on the frontmost window (also bound to keys).
//! Verbs: settings hide hide-others quit force-quit minimise zoom fill centre `desktop N`, and the
//! app commands (copy paste undo … `appmenu::Cmd`) that ⌘ shortcuts tap into the app in front.
use shell::{appmenu, hypr};

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(act) = appmenu::parse_verb(&args.join(" ")) else {
        eprintln!(
            "usage: wol-app settings|hide|hide-others|quit|force-quit|minimise|zoom|fill|centre|desktop N|copy|paste|…"
        );
        return std::process::ExitCode::from(2);
    };
    if let appmenu::Act::Key(c) = act {
        // No window in front (empty desktop, a shell panel): the keyboard focus gets the app key.
        let front = hypr::request("j/activewindow").and_then(|j| hypr::parse_active_client(&j));
        let (class, address) = front.map(|f| (f.class, f.address)).unwrap_or_default();
        let panel = hypr::request("j/layers")
            .is_some_and(|j| appmenu::panel_has_keys(&hypr::parse_layers(&j)));
        if let Some(b) = appmenu::key_batch(c, &class, &address, panel) {
            hypr::request(&b);
        }
        return std::process::ExitCode::SUCCESS;
    }
    let Some(front) = hypr::request("j/activewindow").and_then(|j| hypr::parse_active_client(&j))
    else {
        return std::process::ExitCode::SUCCESS; // empty desktop: nothing to act on
    };
    let zone = hypr::request("j/monitors")
        .and_then(|j| hypr::parse_zone(&j))
        .unwrap_or_default();
    if let Some(b) = appmenu::batch(act, &front, &hypr::clients(), zone) {
        hypr::request(&b);
    }
    std::process::ExitCode::SUCCESS
}

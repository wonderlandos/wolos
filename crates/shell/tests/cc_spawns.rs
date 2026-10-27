//! Live (real GPU): the Control Center Screenshot/Record tiles spawned `wol capture <verb>` and the
//! CLI answered "unknown command 'capture'". Every argv the Control Center spawns must resolve to a
//! real command: a shell panel both the CLI and wol-shell accept, or a binary WolOS installs.
use std::path::Path;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
const SHELL_UI: &str = include_str!("../src/ui/mod.rs");

fn installed(prog: &str) -> bool {
    Path::new(ROOT)
        .join("desktop/overlay/usr/local/bin")
        .join(prog)
        .is_file()
        || Path::new(ROOT)
            .join("crates/shell/src/bin")
            .join(format!("{prog}.rs"))
            .is_file()
}

/// wol-shell answers `target`: a registered panel or a surface with its own handler.
fn shell_answers(target: &str) -> bool {
    SHELL_UI.contains(&format!("p.insert(\"{target}\"")) || target == shell::ui::capture::RECORD
}

#[test]
fn every_control_center_spawn_is_a_real_command() {
    let spawns = shell::ui::control_center::SPAWNS;
    assert!(spawns.len() >= 2, "{spawns:?}");
    for argv in spawns {
        match argv {
            ["wol", "shell", rest @ ..] => {
                let cmd = shell::protocol::parse(&rest.join(" "))
                    .unwrap_or_else(|| panic!("{argv:?}: wol-shell cannot parse it"));
                assert!(
                    shared::shell::PANELS.contains(&cmd.target.as_str()),
                    "{argv:?}: the wol CLI rejects '{}'",
                    cmd.target
                );
                assert!(
                    shell_answers(&cmd.target),
                    "{argv:?}: wol-shell has no '{}'",
                    cmd.target
                );
            }
            ["wol", sub, ..] => panic!("{argv:?}: only `wol shell` is spawned, not `wol {sub}`"),
            ["wol-settings", page] => assert!(
                shell::ui::settings::PAGES.contains(page),
                "{argv:?}: Settings has no page '{page}'"
            ),
            [prog, ..] => assert!(installed(prog), "{argv:?}: {prog} is not installed"),
            [] => panic!("empty argv"),
        }
    }
}

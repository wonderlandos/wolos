// SPDX-License-Identifier: GPL-2.0-or-later
//! A bundle whose Linux entry is a script runs it by its shebang, whatever its language (ADR
//! 0041): the launch plan is the entry itself plus the arguments, never an interpreter wol-run
//! picked, so a `#!/usr/bin/ruby` entry and a `#!/bin/sh` entry take the same path. The plan is
//! checked for both, then run; a missing interpreter skips the run with a message.
use shared::bundle::{Abi, Target};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

const ENTRIES: &[(&str, &str, &str)] = &[
    (
        "/bin/sh",
        "run.sh",
        "#!/bin/sh\n# SPDX-License-Identifier: GPL-2.0-or-later\nprintf 'sh %s %s\\n' \"$1\" \"$LD_LIBRARY_PATH\"\n",
    ),
    (
        "/usr/bin/ruby",
        "run.rb",
        "#!/usr/bin/ruby\n# SPDX-License-Identifier: GPL-2.0-or-later\n# frozen_string_literal: true\n\nputs \"ruby #{ARGV[0]} #{ENV['LD_LIBRARY_PATH']}\"\n",
    ),
];

#[test]
fn a_script_entry_runs_by_its_shebang_in_every_language() {
    let proton = proxy::Proton {
        loader: "/nonexistent/proton".into(),
        defaults: vec![],
    };
    for (interp, name, text) in ENTRIES {
        let dir =
            std::env::temp_dir().join(format!("wol-script-entry-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        let entry = dir.join("bin").join(name);
        std::fs::write(&entry, text).unwrap();
        std::fs::set_permissions(&entry, std::fs::Permissions::from_mode(0o755)).unwrap();
        let target = Target {
            abi: Abi::Linux,
            arch: "x86_64".into(),
            entry: format!("bin/{name}"),
        };
        let d = dir.to_str().unwrap();
        let plan = proxy::plan(
            d,
            &target,
            "org.example.Script",
            "/nonexistent",
            &["one".into()],
            &proton,
        );
        assert_eq!(
            plan.argv,
            [entry.to_string_lossy().into_owned(), "one".to_string()],
            "{name}: the entry itself, no interpreter chosen by wol-run"
        );
        if !Path::new(interp).exists() {
            eprintln!("{interp} not installed; {name} planned, not run");
            continue;
        }
        let out = Command::new(&plan.argv[0])
            .args(&plan.argv[1..])
            .envs(&plan.env)
            .output()
            .unwrap();
        assert!(out.status.success(), "{name}: {out:?}");
        let kind = name
            .rsplit('.')
            .next()
            .map(|e| if e == "rb" { "ruby" } else { "sh" })
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            format!("{kind} one {d}/lib\n"),
            "{name}"
        );
    }
}

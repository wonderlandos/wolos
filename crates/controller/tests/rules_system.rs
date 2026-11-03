// SPDX-License-Identifier: GPL-2.0-or-later
//! System-file rules (docs/RULES-system.md, ids RS-nnn): shipped scripts, systemd units, sysctl,
//! polkit, compositor config, .desktop files and pacman hooks are read from the repo and checked
//! against a catalogue of consistency rules. Nothing is executed except `sh -n` and `shellcheck`
//! (when the box has them). A rule the files do not meet yet carries a BASELINE
//! (`support/rules_system_base.rs`): the list only shrinks, a new violation fails the test, a fixed
//! one is reported on stderr so the entry is removed. Every `rs_nnn_*` test is one rule;
//! `doc_and_tests_agree` ties them to the document.
#[path = "support/rules_system_base.rs"]
mod base;
#[path = "support/siblings.rs"]
mod siblings;
use base::*;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

// ---------------------------------------------------------------- scripts RS-001..RS-029

#[test]
fn rs_000_scripts_found() {
    assert!(scripts().len() >= 25, "{:?}", scripts());
}

#[test]
fn rs_001_shebang_is_bin_sh() {
    let f = on_scripts(|_, t| t.lines().next() != Some("#!/bin/sh"));
    ratchet("RS-001", f, B_SHEBANG);
}

#[test]
fn rs_002_spdx_on_line_two() {
    let f = on_scripts(|_, t| {
        !t.lines()
            .nth(1)
            .unwrap_or("")
            .contains("SPDX-License-Identifier: ")
    });
    ratchet("RS-002", f, &[]);
}

#[test]
fn rs_003_header_says_what_why_how() {
    // heuristic: at least two comment lines (>= 60 characters together) after the SPDX line
    let f = on_scripts(|_, t| {
        let h: Vec<&str> = t.lines().skip(2).take_while(|l| is_comment(l)).collect();
        h.len() < 2 || h.iter().map(|l| l.len()).sum::<usize>() < 60
    });
    ratchet("RS-003", f, B_HEADER);
}

#[test]
fn rs_004_strict_mode() {
    let f = on_scripts(|_, t| {
        let documented = t.lines().any(|l| is_comment(l) && l.contains("set -e"));
        let first: Vec<(usize, String)> = code_lines(t).into_iter().take(10).collect();
        !first.iter().any(|(_, l)| {
            let l = l.trim();
            let strict = l.starts_with("set -") && l.contains('e') && l.contains('u');
            let u_only = l.starts_with("set -") && l.contains('u') && documented;
            strict || u_only
        })
    });
    ratchet("RS-004", f, B_SET_EU);
}

#[test]
fn rs_005_no_eval() {
    ratchet("RS-005", any_code(|l| word_in(l, "eval")), B_EVAL);
}

#[test]
fn rs_006_no_shell_c_of_a_variable() {
    let f = any_code(|l| {
        ["sh -c \"$", "sh -c $", "bash -c \"$", "bash -c $"]
            .iter()
            .any(|p| l.contains(p))
    });
    ratchet("RS-006", f, B_SH_C);
}

#[test]
fn rs_007_rm_rf_variable_paths_are_guarded() {
    let f = any_code(|l| {
        let Some(p) = l.find("rm -rf").or_else(|| l.find("rm -fr")) else {
            return false;
        };
        let rest = &l[p + 6..];
        let rest = rest.split([';', '|', ')']).next().unwrap_or(rest);
        let rest = rest.split("&&").next().unwrap_or(rest);
        rest.split_whitespace()
            .any(|a| a.contains('$') && !a.contains(":?"))
    });
    ratchet("RS-007", f, B_RM_RF);
}

#[test]
fn rs_008_no_quiet_failure_on_disk_key_boot_db_writes() {
    const WRITERS: &[&str] = &[
        "mkfs",
        "mkswap",
        "sgdisk",
        "sfdisk",
        "parted",
        "wipefs",
        "systemd-repart",
        "luksFormat",
        "luksAddKey",
        "luksKillSlot",
        "openssl",
        "efibootmgr",
        "sbctl",
        "bootctl",
        "pacman-key",
        "tpm2_",
        "dd ",
        "btrfs subvolume",
        "mkinitcpio",
        "-keyout",
        "shred",
    ];
    let f = any_code(|l| l.contains("2>/dev/null") && WRITERS.iter().any(|w| l.contains(w)));
    ratchet("RS-008", f, B_QUIET_WRITES);
}

#[test]
fn rs_009_no_pipe_to_shell() {
    let f = any_code(|l| {
        (l.contains("curl") || l.contains("wget"))
            && ["| sh", "|sh", "| bash", "|bash", "| sudo sh"]
                .iter()
                .any(|p| l.contains(p))
    });
    ratchet("RS-009", f, &[]);
}

#[test]
fn rs_010_sudo_only_where_allowed() {
    // wol-install is the one place a user-level program asks for root (sudoers rule, ADR 0023)
    const ALLOWED: &[&str] = &["files/usr/lib/wolos/wol-install"];
    let f = any_code(|l| word_in(l, "sudo") || word_in(l, "pkexec") || word_in(l, "doas"));
    let bad: BTreeSet<String> = f
        .into_iter()
        .filter(|s| !ALLOWED.contains(&s.as_str()))
        .collect();
    ratchet("RS-010", bad, &[]);
}

/// Unquoted `$name` / `${...}` occurrences in a shell script (heuristic; see docs).
fn unquoted(text: &str) -> usize {
    #[derive(Clone, Copy, PartialEq)]
    enum S {
        Top,
        Dq,
        Sq,
        Cmd,
        Arith,
    }
    let mut st = vec![S::Top];
    let mut n = 0;
    let mut heredoc: Option<String> = None;
    for line in text.lines() {
        if let Some(e) = &heredoc {
            if line.trim() == e {
                heredoc = None;
            }
            continue;
        }
        if st.len() == 1 && is_comment(line) {
            continue;
        }
        let c: Vec<char> = line.chars().collect();
        let trimmed = line.trim_start();
        let case_end = if trimmed.starts_with("case ") {
            line.find(" in").unwrap_or(0)
        } else {
            0
        };
        let mut i = 0;
        while i < c.len() {
            let top = *st.last().unwrap();
            let ch = c[i];
            match top {
                S::Sq => {
                    if ch == '\'' {
                        st.pop();
                    }
                }
                S::Dq => {
                    if ch == '\\' {
                        i += 1;
                    } else if ch == '"' {
                        st.pop();
                    } else if ch == '$' && c.get(i + 1) == Some(&'(') {
                        if c.get(i + 2) == Some(&'(') {
                            st.push(S::Arith);
                            i += 2;
                        } else {
                            st.push(S::Cmd);
                            i += 1;
                        }
                    }
                }
                S::Arith => {
                    if ch == ')' && c.get(i + 1) == Some(&')') {
                        st.pop();
                        i += 1;
                    }
                }
                S::Top | S::Cmd => {
                    let word_start = i == 0 || c[i - 1].is_whitespace() || c[i - 1] == ';';
                    if ch == '#' && word_start && st.len() == 1 {
                        break;
                    }
                    if ch == '\\' {
                        i += 1;
                    } else if ch == '\'' {
                        st.push(S::Sq);
                    } else if ch == '"' {
                        st.push(S::Dq);
                    } else if ch == ')' && top == S::Cmd {
                        st.pop();
                    } else if ch == '<' && c.get(i + 1) == Some(&'<') && c.get(i + 2) != Some(&'<')
                    {
                        let rest: String = c[i + 2..].iter().collect();
                        let w: String = rest
                            .trim_start_matches('-')
                            .trim_start()
                            .trim_start_matches(['\'', '"'])
                            .chars()
                            .take_while(|x| x.is_alphanumeric() || *x == '_')
                            .collect();
                        if !w.is_empty() {
                            heredoc = Some(w);
                        }
                    } else if ch == '$' {
                        let nx = c.get(i + 1).copied().unwrap_or(' ');
                        if nx == '(' {
                            if c.get(i + 2) == Some(&'(') {
                                st.push(S::Arith);
                                i += 2;
                            } else {
                                st.push(S::Cmd);
                                i += 1;
                            }
                        } else if (nx.is_alphanumeric()
                            || nx == '_'
                            || nx == '{'
                            || nx == '@'
                            || nx == '*')
                            && !(i > 0 && c[i - 1] == '=')
                            && !(case_end > 0 && i < case_end)
                        {
                            n += 1;
                        }
                    }
                }
            }
            i += 1;
        }
    }
    n
}

#[test]
fn rs_011_expansions_are_quoted() {
    let mut m = BTreeMap::new();
    for s in scripts() {
        let n = unquoted(&read(s));
        if n > 0 {
            m.insert(s.clone(), n);
        }
    }
    ratchet_count("RS-011", m, B_UNQUOTED);
}

#[test]
fn rs_012_scripts_parse_with_sh_n() {
    let mut bad = BTreeSet::new();
    for s in scripts() {
        match Command::new("sh").arg("-n").arg(root().join(s)).output() {
            Ok(o) if !o.status.success() => {
                bad.insert(format!(
                    "{s}: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                ));
            }
            Ok(_) => {}
            Err(_) => return, // no sh on this box
        }
    }
    ratchet("RS-012", bad, &[]);
}

#[test]
fn rs_013_shellcheck_has_no_errors() {
    if Command::new("shellcheck")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("RS-013: shellcheck not installed; skipped");
        return;
    }
    let mut bad = BTreeSet::new();
    for s in scripts() {
        let o = Command::new("shellcheck")
            .args(["-S", "error", "-s", "sh"])
            .arg(root().join(s))
            .output()
            .unwrap();
        if !o.status.success() {
            bad.insert(s.clone());
        }
    }
    ratchet("RS-013", bad, &[]);
}

#[test]
fn rs_014_no_bashisms() {
    let f = any_code(|l| {
        let t = l.trim_start();
        l.contains("[[ ")
            || l.contains(" <(")
            || l.contains("$RANDOM")
            || l.contains("echo -e ")
            || l.contains("${#!")
            || l.contains("&>")
            || t.starts_with("source ")
            || t.starts_with("function ")
    });
    ratchet("RS-014", f, B_BASHISM);
}

#[test]
fn rs_015_no_backticks() {
    ratchet("RS-015", any_code(|l| l.contains('`')), B_BACKTICK);
}

#[test]
fn rs_016_no_fixed_tmp_paths() {
    ratchet("RS-016", any_code(|l| l.contains("/tmp/")), B_TMP);
}

#[test]
fn rs_017_nothing_is_made_world_writable() {
    let f = any_code(|l| {
        l.contains("chmod 777")
            || l.contains("chmod -R 777")
            || l.contains("a+w")
            || l.contains("o+w")
    });
    ratchet("RS-017", f, &[]);
}

#[test]
fn rs_018_no_home_directory_literals() {
    ratchet("RS-018", any_code(|l| l.contains("/home/")), B_HOME);
}

#[test]
fn rs_019_no_trace_mode() {
    let f = any_code(|l| {
        let t = l.trim();
        t.starts_with("set -x") || t.starts_with("set -ex") || t.starts_with("set -eux")
    });
    ratchet("RS-019", f, &[]);
}

#[test]
fn rs_020_curl_fails_on_http_errors_and_has_a_time_limit() {
    let f = on_scripts(|_, t| {
        let mut bad = false;
        let lines = code_lines(t);
        for (i, (_, l)) in lines.iter().enumerate() {
            if word_in(l, "curl") && !l.trim_start().starts_with("echo") {
                // the invocation may continue on the next lines (trailing backslash)
                let mut call = l.clone();
                let mut j = i;
                while call.trim_end().ends_with('\\') && j + 1 < lines.len() {
                    j += 1;
                    call.push_str(&lines[j].1);
                }
                let fail = call.contains(" -f") || call.contains("--fail");
                let limit = call.contains(" -m ")
                    || call.contains("--max-time")
                    || call.contains("--connect-timeout");
                bad |= !(fail && limit);
            }
        }
        bad
    });
    ratchet("RS-020", f, B_CURL);
}

#[test]
fn rs_021_read_uses_dash_r() {
    let f = any_code(|l| {
        let t = l
            .trim_start()
            .trim_start_matches("while ")
            .trim_start_matches("if ");
        t.starts_with("read ") && !t.starts_with("read -r")
    });
    ratchet("RS-021", f, B_READ_R);
}

#[test]
fn rs_022_command_v_not_which() {
    ratchet(
        "RS-022",
        any_code(|l| word_in(l, "which") && l.contains("which ")),
        B_WHICH,
    );
}

#[test]
fn rs_023_no_carriage_returns_in_shipped_text() {
    let mut bad = BTreeSet::new();
    for f in files_under("files")
        .into_iter()
        .chain(files_under("desktop/overlay"))
    {
        if read(&f).contains('\r') {
            bad.insert(f);
        }
    }
    ratchet("RS-023", bad, &[]);
}

#[test]
fn rs_024_every_wolos_script_is_referenced() {
    let mut hay = String::new();
    for top in ["files", "desktop", "crates"] {
        for f in files_under(top) {
            if f.ends_with(".rs")
                || f.contains("/systemd/")
                || f.contains("/hooks/")
                || f.contains("/hypr/")
                || f.contains("/lib/wolos/")
                || f.ends_with(".service")
            {
                hay.push_str(&read(&f));
                hay.push('\n');
            }
        }
    }
    let mut dead = BTreeSet::new();
    for s in scripts()
        .iter()
        .filter(|s| s.starts_with("files/usr/lib/wolos/"))
    {
        let n = base(s);
        // one reference outside the script itself
        let own = read(s);
        let mentions = hay.matches(n).count() - own.matches(n).count();
        if mentions == 0 {
            dead.insert(s.clone());
        }
    }
    ratchet("RS-024", dead, B_DEAD_SCRIPT);
}

#[test]
fn rs_025_no_literal_credentials() {
    let f = any_code(|l| {
        ["password", "passwd", "secret", "token", "apikey"]
            .iter()
            .any(|k| {
                let low = l.to_ascii_lowercase();
                low.find(&format!("{k}=")).is_some_and(|p| {
                    let v = &l[p + k.len() + 1..];
                    let v = v.trim_start_matches(['\'', '"']);
                    !v.is_empty()
                        && !v.starts_with(['$', '\'', '"', ' ', ';'])
                        && v.chars().take(6).all(|c| c.is_alphanumeric())
                })
            })
    });
    ratchet("RS-025", f, &[]);
}

#[test]
fn rs_026_scripts_do_not_pipe_secrets_through_echo_args() {
    // a key or password on a command line shows in /proc/<pid>/cmdline
    let f = any_code(|l| {
        [
            "--passphrase ",
            "--password ",
            "--key-file=/dev/stdin ",
            "-pass pass:",
        ]
        .iter()
        .any(|p| l.contains(p) && l.contains('$'))
    });
    ratchet("RS-026", f, &[]);
}

#[test]
fn rs_027_mktemp_has_a_trap() {
    let f = on_scripts(|_, t| {
        let c = code_lines(t);
        c.iter().any(|(_, l)| word_in(l, "mktemp"))
            && !c.iter().any(|(_, l)| l.trim_start().starts_with("trap "))
    });
    ratchet("RS-027", f, B_MKTEMP_TRAP);
}

#[test]
fn rs_028_scripts_end_with_a_newline() {
    let f = on_scripts(|_, t| !t.ends_with('\n'));
    ratchet("RS-028", f, &[]);
}

// ---------------------------------------------------------------- units RS-030..RS-055

/// Root services deliberately without a sandbox (crates/controller/tests/units.rs EXEMPT).
const UNIT_EXEMPT: &[&str] = &["wold.service"];

#[test]
fn rs_029_units_found() {
    assert!(system_services().len() >= 15, "{:?}", system_services());
    assert!(unit_files(SYS_UNITS, &[".timer"]).len() >= 5);
}

#[test]
fn rs_030_root_services_carry_the_baseline_hardening_keys() {
    let mut bad = BTreeSet::new();
    for u in system_services().iter().filter(|u| is_root_service(u)) {
        if UNIT_EXEMPT.contains(&base(u)) {
            continue;
        }
        for (k, v) in [
            ("NoNewPrivileges", "yes"),
            ("LockPersonality", "yes"),
            ("RestrictRealtime", "yes"),
            ("SystemCallArchitectures", "native"),
        ] {
            if vals(u, "Service", k) != [v] {
                bad.insert(format!("{u}: {k}={v}"));
            }
        }
    }
    ratchet("RS-030", bad, &[]);
}

#[test]
fn rs_031_root_services_protect_the_system_tree() {
    let f = system_services()
        .into_iter()
        .filter(|u| is_root_service(u) && !UNIT_EXEMPT.contains(&base(u)))
        .filter(|u| !has(u, "Service", "ProtectSystem"))
        .collect();
    ratchet("RS-031", f, B_PROTECT_SYSTEM);
}

#[test]
fn rs_032_unprivileged_services_cannot_gain_privileges() {
    let f = system_services()
        .into_iter()
        .filter(|u| !is_root_service(u) && vals(u, "Service", "NoNewPrivileges") != ["yes"])
        .collect();
    ratchet("RS-032", f, &[]);
}

fn exec_values(u: &str) -> Vec<String> {
    ini(u)
        .into_iter()
        .filter(|(s, k, _)| s == "Service" && k.starts_with("Exec"))
        .map(|t| t.2)
        .collect()
}

fn exec_cmd(v: &str) -> &str {
    v.trim_start_matches(['-', '@', '+', '!', ':', '|'])
        .split_whitespace()
        .next()
        .unwrap_or("")
}

#[test]
fn rs_033_exec_paths_are_absolute() {
    let mut bad = BTreeSet::new();
    for u in all_units() {
        for v in exec_values(&u) {
            if !exec_cmd(&v).starts_with('/') {
                bad.insert(format!("{u}: {v}"));
            }
        }
    }
    ratchet("RS-033", bad, &[]);
}

#[test]
fn rs_034_exec_programs_exist() {
    let mut bad = BTreeSet::new();
    for u in all_units() {
        for v in exec_values(&u) {
            let c = exec_cmd(&v);
            if c.starts_with('/') && !command_exists(c) {
                bad.insert(format!("{u}: {c}"));
            }
        }
    }
    ratchet("RS-034", bad, &[]);
}

#[test]
fn rs_035_no_explicit_user_root() {
    let f = all_units()
        .into_iter()
        .filter(|u| {
            vals(u, "Service", "User")
                .iter()
                .any(|v| v == "root" || v == "0")
        })
        .collect();
    ratchet("RS-035", f, &[]);
}

#[test]
fn rs_036_long_running_services_state_a_restart_policy() {
    let f = system_services()
        .into_iter()
        .filter(|u| {
            vals(u, "Service", "Type")
                .first()
                .is_none_or(|t| t != "oneshot")
        })
        .filter(|u| !has(u, "Service", "Restart"))
        .collect();
    ratchet("RS-036", f, B_RESTART);
}

#[test]
fn rs_037_calendar_timers_are_persistent() {
    let f = unit_files(SYS_UNITS, &[".timer"])
        .into_iter()
        .chain(unit_files(USER_UNITS, &[".timer"]))
        .filter(|u| has(u, "Timer", "OnCalendar") && vals(u, "Timer", "Persistent") != ["true"])
        .collect();
    ratchet("RS-037", f, &[]);
}

#[test]
fn rs_038_network_ordering_comes_with_wants() {
    let mut bad = BTreeSet::new();
    let mut nowants = BTreeSet::new();
    for u in all_units() {
        let after: Vec<String> = ini(&u)
            .into_iter()
            .filter(|(s, k, _)| s == "Unit" && k == "After")
            .flat_map(|t| t.2.split_whitespace().map(String::from).collect::<Vec<_>>())
            .collect();
        let wants: Vec<String> = ini(&u)
            .into_iter()
            .filter(|(s, k, _)| s == "Unit" && (k == "Wants" || k == "Requires"))
            .flat_map(|t| t.2.split_whitespace().map(String::from).collect::<Vec<_>>())
            .collect();
        if after.iter().any(|a| a == "network.target") {
            bad.insert(format!(
                "{u}: After=network.target does not wait for a network"
            ));
        }
        if after.iter().any(|a| a == "network-online.target")
            && !wants.iter().any(|a| a == "network-online.target")
        {
            nowants.insert(u);
        }
    }
    ratchet("RS-038", bad, &[]);
    ratchet("RS-038", nowants, B_NET_WANTS);
}

#[test]
fn rs_039_timers_and_sockets_have_their_service() {
    let mut bad = BTreeSet::new();
    for u in all_units() {
        for ext in [".timer", ".socket"] {
            if let Some(stem) = u.strip_suffix(ext) {
                let unit = vals(&u, "Timer", "Unit").first().cloned();
                let svc = unit.map_or_else(
                    || format!("{stem}.service"),
                    |n| format!("{}/{n}", u.rsplit_once('/').unwrap().0),
                );
                if !exists(&svc) {
                    bad.insert(format!("{u}: {svc} missing"));
                }
            }
        }
    }
    ratchet("RS-039", bad, &[]);
}

#[test]
fn rs_040_units_have_a_description() {
    let f = all_units()
        .into_iter()
        .filter(|u| !has(u, "Unit", "Description"))
        .collect();
    ratchet("RS-040", f, &[]);
}

#[test]
fn rs_041_timers_and_sockets_are_wanted_by_their_target() {
    let mut bad = BTreeSet::new();
    for u in all_units() {
        let want = if u.ends_with(".timer") {
            "timers.target"
        } else if u.ends_with(".socket") {
            "sockets.target"
        } else {
            continue;
        };
        if !vals(&u, "Install", "WantedBy").iter().any(|v| v == want) {
            bad.insert(format!("{u}: WantedBy={want}"));
        }
    }
    ratchet("RS-041", bad, &[]);
}

#[test]
fn rs_042_no_deprecated_unit_keys() {
    const OLD: &[&str] = &[
        "PermissionsStartOnly",
        "MemoryLimit",
        "CPUShares",
        "BlockIOWeight",
        "StartLimitInterval",
        "StartLimitBurst",
        "SendSIGKILL_",
        "RemoveIPC_",
    ];
    let mut bad = BTreeSet::new();
    for u in all_units() {
        for (s, k, _) in ini(&u) {
            // StartLimit* belong in [Unit] since systemd 229
            if OLD.contains(&k.as_str()) && !(s == "Unit" && k.starts_with("StartLimit")) {
                bad.insert(format!("{u}: [{s}] {k}"));
            }
        }
    }
    ratchet("RS-042", bad, &[]);
}

#[test]
fn rs_043_units_do_not_start_shells() {
    let mut bad = BTreeSet::new();
    for u in all_units() {
        for v in exec_values(&u) {
            let c = exec_cmd(&v);
            if matches!(c, "/bin/sh" | "/usr/bin/sh" | "/bin/bash" | "/usr/bin/bash") {
                bad.insert(u.clone());
            }
        }
    }
    ratchet("RS-043", bad, B_UNIT_SHELL);
}

#[test]
fn rs_044_dropin_dirs_hold_only_conf_files() {
    let mut bad = BTreeSet::new();
    for f in files_under(SYS_UNITS) {
        let rest = &f[SYS_UNITS.len() + 1..];
        if let Some((dir, file)) = rest.split_once('/')
            && (!dir.ends_with(".d") || !file.ends_with(".conf") || file.contains('/'))
        {
            bad.insert(f.clone());
        }
    }
    ratchet("RS-044", bad, &[]);
}

#[test]
fn rs_045_unit_names_carry_the_wolos_prefix() {
    let mut bad = BTreeSet::new();
    for u in all_units() {
        let n = base(&u);
        if !(n.starts_with("wolos-") || n.starts_with("wold.")) {
            bad.insert(u);
        }
    }
    ratchet("RS-045", bad, &[]);
}

#[test]
fn rs_046_read_write_paths_are_absolute() {
    let mut bad = BTreeSet::new();
    for u in all_units() {
        for k in ["ReadWritePaths", "ReadOnlyPaths", "InaccessiblePaths"] {
            for v in vals(&u, "Service", k) {
                for p in v.split_whitespace() {
                    if !p.trim_start_matches(['-', '+', '!', '~']).starts_with('/') {
                        bad.insert(format!("{u}: {k}={p}"));
                    }
                }
            }
        }
    }
    ratchet("RS-046", bad, &[]);
}

#[test]
fn rs_047_no_secrets_in_unit_environment() {
    let mut bad = BTreeSet::new();
    for u in all_units() {
        for v in vals(&u, "Service", "Environment") {
            let low = v.to_ascii_lowercase();
            if ["password", "secret", "token", "apikey", "key="]
                .iter()
                .any(|k| low.contains(k))
            {
                bad.insert(format!("{u}: {v}"));
            }
        }
    }
    ratchet("RS-047", bad, &[]);
}

#[test]
fn rs_048_oneshot_units_do_not_restart_always() {
    let f = all_units()
        .into_iter()
        .filter(|u| {
            vals(u, "Service", "Type") == ["oneshot"]
                && vals(u, "Service", "Restart").iter().any(|r| r == "always")
        })
        .collect();
    ratchet("RS-048", f, &[]);
}

#[test]
fn rs_049_ordering_names_a_unit_that_exists() {
    let mut bad = BTreeSet::new();
    for u in all_units() {
        for (s, k, v) in ini(&u) {
            if s == "Unit"
                && matches!(
                    k.as_str(),
                    "After" | "Before" | "Wants" | "Requires" | "BindsTo" | "PartOf"
                )
            {
                for n in v
                    .split_whitespace()
                    .filter(|n| n.starts_with("wolos-") || n.starts_with("wold."))
                {
                    if !exists(&format!("{SYS_UNITS}/{n}")) && !exists(&format!("{USER_UNITS}/{n}"))
                    {
                        bad.insert(format!("{u}: {k}={n}"));
                    }
                }
            }
        }
    }
    ratchet("RS-049", bad, &[]);
}

#[test]
fn rs_050_hardening_is_explained() {
    let f = system_services()
        .into_iter()
        .filter(|u| {
            is_root_service(u)
                && !UNIT_EXEMPT.contains(&base(u))
                && has(u, "Service", "ProtectSystem")
        })
        .filter(|u| !read(u).contains("# Hardening"))
        .collect();
    ratchet("RS-050", f, B_HARDENING_COMMENT);
}

#[test]
fn rs_051_system_slices_are_dropins_only() {
    // resource limits for slices are drop-ins (70-wolos-limits.conf), never replacement units
    let f = unit_files(SYS_UNITS, &[".slice", ".mount", ".scope"]);
    ratchet("RS-051", f.into_iter().collect(), &[]);
}

// ---------------------------------------------------------------- sysctl RS-060..RS-068

fn security_doc() -> String {
    read("docs/modules/security.md")
}

fn documented_key(doc: &str, k: &str) -> bool {
    let short = k.strip_prefix("kernel.").unwrap_or(k);
    // the doc writes `fs.protected_hardlinks/symlinks` for two keys
    let tail = k.rsplit('_').next().unwrap_or(k);
    doc.contains(k)
        || doc.contains(short)
        || (k.starts_with("fs.protected_") && doc.contains(&format!("/{tail}")))
}

#[test]
fn rs_060_every_sysctl_key_is_documented() {
    let doc = security_doc();
    assert!(!doc.is_empty(), "docs/modules/security.md");
    let f = sysctl_pairs()
        .into_iter()
        .filter(|(_, k, _)| !documented_key(&doc, k))
        .map(|(_, k, _)| k)
        .collect();
    ratchet("RS-060", f, B_SYSCTL_DOC);
}

#[test]
fn rs_061_wol_security_reports_documented_keys() {
    let doc = security_doc();
    for (k, _, _) in wol_core::security::HARDENING {
        assert!(
            documented_key(&doc, k),
            "RS-061: {k} not in docs/modules/security.md"
        );
    }
}

#[test]
fn rs_062_sysctl_files_are_ordered_wolos_confs() {
    let mut bad = BTreeSet::new();
    for f in files_under("files/usr/lib/sysctl.d") {
        let n = base(&f);
        let ok = n.ends_with(".conf")
            && n.len() > 3
            && n[..2].chars().all(|c| c.is_ascii_digit())
            && n[2..].starts_with("-wolos");
        if !ok {
            bad.insert(f);
        }
    }
    ratchet("RS-062", bad, &[]);
}

#[test]
fn rs_063_sysctl_lines_are_key_equals_value() {
    let mut bad = BTreeSet::new();
    for (f, k, v) in sysctl_pairs() {
        let key_ok = !k.is_empty()
            && k.chars().all(|c| {
                c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_' || c == '-'
            });
        let val_ok = !v.is_empty() && !v.contains(['$', '`', ';', '"']);
        if !key_ok || !val_ok {
            bad.insert(format!("{f}: {k} = {v}"));
        }
    }
    ratchet("RS-063", bad, &[]);
}

#[test]
fn rs_064_no_key_gets_two_different_values() {
    let mut seen: BTreeMap<String, (String, String)> = BTreeMap::new();
    let mut bad = BTreeSet::new();
    for (f, k, v) in sysctl_pairs() {
        if let Some((pf, pv)) = seen.get(&k)
            && *pv != v
        {
            bad.insert(format!("{k}: {pf}={pv} vs {f}={v}"));
        }
        seen.insert(k, (f, v));
    }
    ratchet("RS-064", bad, &[]);
}

#[test]
fn rs_065_no_setting_weakens_hardening() {
    const WEAK: &[(&str, &str)] = &[
        ("kernel.yama.ptrace_scope", "0"),
        ("kernel.randomize_va_space", "0"),
        ("kernel.kptr_restrict", "0"),
        ("kernel.dmesg_restrict", "0"),
        ("kernel.unprivileged_bpf_disabled", "0"),
        ("kernel.kexec_load_disabled", "0"),
        ("fs.suid_dumpable", "1"),
        ("fs.suid_dumpable", "2"),
        ("fs.protected_symlinks", "0"),
        ("fs.protected_hardlinks", "0"),
        ("vm.mmap_min_addr", "0"),
        ("net.ipv4.tcp_syncookies", "0"),
        ("net.ipv4.conf.all.accept_source_route", "1"),
        ("net.ipv4.conf.all.accept_redirects", "1"),
    ];
    let mut bad = BTreeSet::new();
    for (f, k, v) in sysctl_pairs() {
        if WEAK.contains(&(k.as_str(), v.as_str())) {
            bad.insert(format!("{f}: {k} = {v}"));
        }
    }
    ratchet("RS-065", bad, &[]);
}

#[test]
fn rs_066_sysctl_files_open_with_a_comment() {
    let f = files_under("files/usr/lib/sysctl.d")
        .into_iter()
        .filter(|f| !read(f).lines().next().is_some_and(is_comment))
        .collect();
    ratchet("RS-066", f, &[]);
}

#[test]
fn rs_067_sysctl_keys_have_a_reason_nearby() {
    let mut bad = BTreeSet::new();
    for f in files_under("files/usr/lib/sysctl.d") {
        let t = read(&f);
        let lines: Vec<&str> = t.lines().collect();
        for (i, l) in lines.iter().enumerate() {
            if l.trim().is_empty() || is_comment(l) || !l.contains('=') {
                continue;
            }
            let near = l.contains(" #") || (i.saturating_sub(6)..i).any(|j| is_comment(lines[j]));
            if !near {
                bad.insert(l.split('=').next().unwrap().trim().to_string());
            }
        }
    }
    ratchet("RS-067", bad, B_SYSCTL_COMMENT);
}

// ---------------------------------------------------------------- polkit RS-070..RS-075

fn polkit_files() -> Vec<String> {
    let mut v = files_under("files/usr/share/polkit-1");
    v.extend(files_under("desktop/overlay/usr/share/polkit-1"));
    v
}

fn js_code(t: &str) -> String {
    t.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn rs_070_polkit_dir_holds_only_rules_files() {
    let all = polkit_files();
    assert!(!all.is_empty(), "10-wolos.rules");
    let f = all
        .into_iter()
        .filter(|f| !(f.contains("/rules.d/") && f.ends_with(".rules")))
        .collect();
    ratchet("RS-070", f, &[]);
}

#[test]
fn rs_071_polkit_never_says_yes() {
    let mut bad = BTreeSet::new();
    for f in polkit_files() {
        let c = js_code(&read(&f));
        for w in [
            "Result.YES",
            "AUTH_SELF_KEEP",
            "AUTH_ADMIN_KEEP",
            "polkit.spawn",
            "addAdminRule",
        ] {
            if c.contains(w) {
                bad.insert(format!("{f}: {w}"));
            }
        }
    }
    ratchet("RS-071", bad, &[]);
}

#[test]
fn rs_072_polkit_rules_sort_before_the_distribution_defaults() {
    let mut bad = BTreeSet::new();
    for f in polkit_files() {
        let n = base(&f);
        let num: Option<u32> = n.get(..2).and_then(|x| x.parse().ok());
        if !(num.is_some_and(|x| x < 50) && n[2..].starts_with("-wolos")) {
            bad.insert(f);
        }
    }
    ratchet("RS-072", bad, &[]);
}

#[test]
fn rs_073_polkit_rules_use_add_rule_and_refuse_wolos_actions() {
    let mut bad = BTreeSet::new();
    for f in polkit_files() {
        let c = js_code(&read(&f));
        if !c.contains("polkit.addRule(") || !c.contains("org.wolos.") {
            bad.insert(f);
        }
    }
    ratchet("RS-073", bad, &[]);
}

#[test]
fn rs_074_no_polkit_action_definitions_are_shipped() {
    let mut bad = BTreeSet::new();
    for top in ["files", "desktop"] {
        for f in files_under(top) {
            if f.ends_with(".policy") {
                bad.insert(f);
            }
        }
    }
    ratchet("RS-074", bad, &[]);
}

#[test]
fn rs_075_polkit_rules_carry_spdx_and_a_header() {
    let f = polkit_files()
        .into_iter()
        .filter(|f| {
            let t = read(f);
            !(t.lines()
                .next()
                .is_some_and(|l| l.contains("SPDX-License-Identifier"))
                && t.lines().nth(1).is_some_and(|l| l.starts_with("//")))
        })
        .collect();
    ratchet("RS-075", f, &[]);
}

// ---------------------------------------------------------------- hyprland RS-080..RS-093

#[test]
fn rs_080_every_bind_has_a_shortcut_sheet_row() {
    let mut bad = BTreeSet::new();
    for conf in [wol_core::shortcuts::SHIPPED, wol_core::shortcuts::TILING] {
        for b in wol_core::shortcuts::parse(conf) {
            if wol_core::shortcuts::name_of(&b.dispatcher, &b.args).is_none() {
                bad.insert(format!("{} {}", b.dispatcher, b.args));
            }
        }
    }
    ratchet("RS-080", bad, B_BIND_ROWS);
}

/// First program of an `exec`-style value (`sh -c '...'` is the shell itself).
fn hypr_exec_values(rel: &str) -> Vec<String> {
    let mut v = Vec::new();
    for (_, l) in hypr_code(rel) {
        let l = strip_trailing_comment(&l);
        let Some((lhs, rhs)) = l.split_once('=') else {
            continue;
        };
        let lhs = lhs.trim();
        if matches!(lhs, "exec" | "exec-once" | "exec-shutdown") {
            v.push(rhs.trim().to_string());
        } else if lhs.starts_with("bind") {
            let parts: Vec<&str> = rhs.splitn(4, ',').collect();
            if parts.len() == 4 && parts[2].trim() == "exec" {
                v.push(parts[3].trim().to_string());
            }
        }
    }
    v
}

#[test]
fn rs_081_exec_targets_exist() {
    let mut bad = BTreeSet::new();
    for conf in hypr_confs() {
        for v in hypr_exec_values(&conf) {
            let c = v
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_end_matches(';');
            if !c.is_empty() && !command_exists(c) {
                bad.insert(format!("{conf}: {c}"));
            }
        }
    }
    ratchet("RS-081", bad, &[]);
}

fn colour_literals(l: &str) -> bool {
    if l.contains("rgb(") || l.contains("rgba(") || l.contains("0x") {
        return true;
    }
    let c: Vec<char> = l.chars().collect();
    (0..c.len()).any(|i| {
        c[i] == '#' && c.len() >= i + 7 && c[i + 1..i + 7].iter().all(|x| x.is_ascii_hexdigit())
    })
}

#[test]
fn rs_082_colours_do_not_grow() {
    let n = hypr_code(&format!("{HYPR_DIR}/hyprland.conf"))
        .iter()
        .filter(|(_, l)| colour_literals(strip_trailing_comment(l)))
        .count();
    assert!(
        n <= B_HYPR_COLOURS,
        "RS-082: {n} colour literals in hyprland.conf (baseline {B_HYPR_COLOURS}); theme values belong to the theme"
    );
    if n < B_HYPR_COLOURS {
        eprintln!("RS-082: down to {n}; lower B_HYPR_COLOURS");
    }
}

#[test]
fn rs_083_every_windowrule_is_commented() {
    let mut bad = BTreeSet::new();
    for conf in hypr_confs() {
        let t = read(&conf);
        let lines: Vec<&str> = t.lines().collect();
        for (i, l) in lines.iter().enumerate() {
            let tl = l.trim();
            if !(tl.starts_with("windowrule") || tl.starts_with("layerrule")) || tl.contains(" #") {
                continue;
            }
            let prev = lines[..i]
                .iter()
                .rev()
                .map(|x| x.trim())
                .find(|x| !x.is_empty())
                .unwrap_or("");
            let ok = prev.starts_with('#')
                || prev.starts_with("windowrule")
                || prev.starts_with("layerrule")
                || prev == "}";
            if !ok {
                bad.insert(format!("{conf}:{}: {tl}", i + 1));
            }
        }
    }
    ratchet("RS-083", bad, &[]);
}

#[test]
fn rs_084_no_home_directory_literals_in_hypr_confs() {
    let mut bad = BTreeSet::new();
    for conf in hypr_confs() {
        for (n, l) in hypr_code(&conf) {
            if l.contains("/home/") || l.contains("/Users/") {
                bad.insert(format!("{conf}:{n}"));
            }
        }
    }
    ratchet("RS-084", bad, &[]);
}

#[test]
fn rs_085_session_confs_never_exec_with_privilege_or_network_fetch() {
    let mut bad = BTreeSet::new();
    for conf in hypr_confs() {
        for v in hypr_exec_values(&conf) {
            if ["sudo", "pkexec", "doas", "curl ", "wget ", "su "]
                .iter()
                .any(|w| word_in(&v, w.trim()))
            {
                bad.insert(format!("{conf}: {v}"));
            }
        }
    }
    ratchet("RS-085", bad, &[]);
}

#[test]
fn rs_086_sourced_files_exist() {
    let mut bad = BTreeSet::new();
    for conf in hypr_confs() {
        for (_, l) in hypr_code(&conf) {
            if let Some(p) = l
                .strip_prefix("source")
                .and_then(|r| r.trim_start().strip_prefix('='))
            {
                let p = p.trim();
                if let Some(rel) = p.strip_prefix("/usr/share/wolos/hypr/")
                    && !exists(&format!("{HYPR_DIR}/{rel}"))
                {
                    bad.insert(format!("{conf}: {p}"));
                }
            }
        }
    }
    ratchet("RS-086", bad, &[]);
}

#[test]
fn rs_087_permissions_end_in_deny_all_per_kind() {
    let rel = format!("{HYPR_DIR}/permissions.conf");
    let lines = hypr_code(&rel);
    assert!(
        lines.iter().any(|(_, l)| l == "enforce_permissions = true"),
        "RS-087: enforce_permissions"
    );
    let perms: Vec<(String, String, String)> = lines
        .iter()
        .filter_map(|(_, l)| l.strip_prefix("permission = "))
        .map(|r| {
            let p: Vec<&str> = r.split(',').map(str::trim).collect();
            (p[0].to_string(), p[1].to_string(), p[2].to_string())
        })
        .collect();
    for kind in ["screencopy", "cursorpos", "input-capture", "plugin"] {
        let of: Vec<&(String, String, String)> = perms.iter().filter(|p| p.1 == kind).collect();
        let last = of
            .last()
            .unwrap_or_else(|| panic!("RS-087: no {kind} rule"));
        assert!(
            last.0 == ".*" && last.2 == "deny",
            "RS-087: {kind} must end with `.*, deny`"
        );
        assert!(
            !of.iter().any(|p| p.0 == ".*" && p.2 == "allow"),
            "RS-087: {kind} allow-all"
        );
    }
}

#[test]
fn rs_088_desktop_conf_sources_the_permissions() {
    let t = read(&format!("{HYPR_DIR}/hyprland.conf"));
    assert!(
        t.lines()
            .any(|l| l.trim() == "source = /usr/share/wolos/hypr/permissions.conf"),
        "RS-088"
    );
}

#[test]
fn rs_089_hypr_confs_open_with_a_comment() {
    let f = hypr_confs()
        .into_iter()
        .filter(|c| !read(c).lines().next().is_some_and(is_comment))
        .collect();
    ratchet("RS-089", f, B_HYPR_HEADER);
}

#[test]
fn rs_090_no_windowrulev2_or_legacy_layerrule_syntax() {
    let mut bad = BTreeSet::new();
    for conf in hypr_confs() {
        for (n, l) in hypr_code(&conf) {
            let legacy_layer = l.starts_with("layerrule") && !l.contains("match:");
            if l.starts_with("windowrulev2") || legacy_layer {
                bad.insert(format!("{conf}:{n}: {l}"));
            }
        }
    }
    ratchet("RS-090", bad, &[]);
}

#[test]
fn rs_091_no_two_binds_share_a_key_combination() {
    let mut seen = BTreeSet::new();
    let mut bad = BTreeSet::new();
    for b in wol_core::shortcuts::parse(wol_core::shortcuts::SHIPPED) {
        let k = b.combo.label();
        if !seen.insert(k.clone()) && !b.flags.contains('r') {
            bad.insert(k);
        }
    }
    ratchet("RS-091", bad, B_HYPR_DUP);
}

#[test]
fn rs_092_session_confs_name_the_login_and_installer_programs() {
    for (conf, prog) in [
        ("login.conf", "wol-loginwindow"),
        ("setup.conf", "wol-setup"),
        ("installer.conf", "wol-installer"),
    ] {
        let t = read(&format!("{HYPR_DIR}/{conf}"));
        assert!(t.contains(prog), "RS-092: {conf} starts {prog}");
        assert!(known_bins().contains(prog), "RS-092: {prog} is built here");
    }
}

#[test]
fn rs_093_hypridle_commands_exist() {
    let mut bad = BTreeSet::new();
    for (_, l) in hypr_code(&format!("{HYPR_DIR}/hypridle.conf")) {
        if let Some((k, v)) = l.split_once('=')
            && (k.trim().ends_with("_cmd") || k.trim() == "on-timeout" || k.trim() == "on-resume")
        {
            for part in v.split("||").flat_map(|p| p.split("&&")) {
                let c = part.split_whitespace().next().unwrap_or("");
                if !c.is_empty()
                    && !command_exists(c)
                    && !["pgrep", "loginctl", "hyprctl", "systemctl"].contains(&c)
                {
                    bad.insert(c.to_string());
                }
            }
        }
    }
    ratchet("RS-093", bad, &[]);
}

// ---------------------------------------------------------------- .desktop RS-100..RS-107

fn desktop_files() -> Vec<String> {
    files_under("files")
        .into_iter()
        .chain(files_under("desktop"))
        .filter(|f| f.ends_with(".desktop"))
        .collect()
}

fn de(rel: &str, key: &str) -> Option<String> {
    vals(rel, "Desktop Entry", key).into_iter().next()
}

#[test]
fn rs_100_desktop_exec_exists() {
    assert!(desktop_files().len() >= 3);
    let mut bad = BTreeSet::new();
    for f in desktop_files() {
        let e = de(&f, "Exec").unwrap_or_default();
        let c = e
            .split_whitespace()
            .find(|w| !w.contains('='))
            .unwrap_or("");
        if !command_exists(c) {
            bad.insert(format!("{f}: {c}"));
        }
    }
    ratchet("RS-100", bad, &[]);
}

#[test]
fn rs_101_desktop_icon_is_set_and_absolute_icons_exist() {
    let mut bad = BTreeSet::new();
    for f in desktop_files() {
        match de(&f, "Icon") {
            None => {
                bad.insert(format!("{f}: no Icon"));
            }
            Some(i) if i.starts_with('/') => {
                let shipped =
                    exists(&format!("files{i}")) || exists(&format!("desktop/overlay{i}"));
                if !shipped {
                    bad.insert(f.clone());
                }
            }
            Some(_) => {}
        }
    }
    ratchet("RS-101", bad, B_ICON);
}

#[test]
fn rs_102_desktop_categories_are_set_and_terminated() {
    let f = desktop_files()
        .into_iter()
        .filter(|f| !de(f, "Categories").is_some_and(|c| c.ends_with(';') && c.len() > 1))
        .collect();
    ratchet("RS-102", f, &[]);
}

#[test]
fn rs_103_desktop_files_name_no_user_paths() {
    let f = desktop_files()
        .into_iter()
        .filter(|f| {
            let t = read(f);
            t.contains("/home/") || t.contains("/Users/") || t.contains("C:\\")
        })
        .collect();
    ratchet("RS-103", f, &[]);
}

#[test]
fn rs_104_desktop_entries_have_type_name_and_header() {
    let f = desktop_files()
        .into_iter()
        .filter(|f| {
            let first = read(f)
                .lines()
                .find(|l| !is_comment(l) && !l.trim().is_empty())
                .unwrap_or("")
                .to_string();
            first != "[Desktop Entry]"
                || de(f, "Type").as_deref() != Some("Application")
                || de(f, "Name").is_none()
        })
        .collect();
    ratchet("RS-104", f, &[]);
}

#[test]
fn rs_105_mime_types_end_with_a_semicolon() {
    let f = desktop_files()
        .into_iter()
        .filter(|f| de(f, "MimeType").is_some_and(|m| !m.ends_with(';')))
        .collect();
    ratchet("RS-105", f, &[]);
}

#[test]
fn rs_106_exec_field_codes_are_valid() {
    let mut bad = BTreeSet::new();
    for f in desktop_files() {
        let e = de(&f, "Exec").unwrap_or_default();
        let mut it = e.chars().peekable();
        while let Some(c) = it.next() {
            if c == '%' {
                let n = it.next().unwrap_or(' ');
                if !"fFuUdDnNickvm%".contains(n) {
                    bad.insert(format!("{f}: %{n}"));
                }
            }
        }
    }
    ratchet("RS-106", bad, &[]);
}

#[test]
fn rs_107_desktop_exec_has_no_privilege_escalation() {
    let f = desktop_files()
        .into_iter()
        .filter(|f| {
            let e = de(f, "Exec").unwrap_or_default();
            ["sudo", "pkexec", "su", "doas"]
                .iter()
                .any(|w| word_in(&e, w))
        })
        .collect();
    ratchet("RS-107", f, &[]);
}

// ---------------------------------------------------------------- pacman hooks RS-110..RS-114

fn hooks() -> Vec<String> {
    files_under("files/usr/share/libalpm/hooks")
}

#[test]
fn rs_110_hooks_have_trigger_and_action_keys() {
    assert!(hooks().len() >= 2);
    let mut bad = BTreeSet::new();
    for h in hooks() {
        let kv = ini(&h);
        let a = |k: &str| kv.iter().any(|(s, key, _)| s == "Action" && key == k);
        let t = |k: &str| kv.iter().any(|(s, key, _)| s == "Trigger" && key == k);
        if !(a("Description")
            && a("When")
            && a("Exec")
            && t("Operation")
            && t("Type")
            && t("Target"))
        {
            bad.insert(h);
        }
    }
    ratchet("RS-110", bad, &[]);
}

#[test]
fn rs_111_hook_programs_exist() {
    let mut bad = BTreeSet::new();
    for h in hooks() {
        for e in vals(&h, "Action", "Exec") {
            let c = e.split_whitespace().next().unwrap_or("");
            if !(c.starts_with('/') && command_exists(c)) {
                bad.insert(format!("{h}: {c}"));
            }
        }
    }
    ratchet("RS-111", bad, &[]);
}

#[test]
fn rs_112_hook_files_are_named_for_wolos() {
    let f = hooks()
        .into_iter()
        .filter(|h| {
            let n = base(h);
            let stem = n.trim_start_matches(|c: char| c.is_ascii_digit() || c == '-');
            !n.ends_with(".hook") || !stem.starts_with("wolos")
        })
        .collect();
    ratchet("RS-112", f, &[]);
}

#[test]
fn rs_113_hook_operations_and_timing_are_valid() {
    let mut bad = BTreeSet::new();
    for h in hooks() {
        for o in vals(&h, "Trigger", "Operation") {
            if !["Install", "Upgrade", "Remove"].contains(&o.as_str()) {
                bad.insert(format!("{h}: Operation={o}"));
            }
        }
        for w in vals(&h, "Action", "When") {
            if !["PreTransaction", "PostTransaction"].contains(&w.as_str()) {
                bad.insert(format!("{h}: When={w}"));
            }
        }
    }
    ratchet("RS-113", bad, &[]);
}

#[test]
fn rs_114_hook_shell_commands_are_guarded() {
    // an inline `sh -c` hook must not fail the transaction when its input is absent
    let mut bad = BTreeSet::new();
    for h in hooks() {
        for e in vals(&h, "Action", "Exec") {
            if e.starts_with("/bin/sh -c")
                && !(e.contains("||") || e.contains("&&") || e.contains("[ "))
            {
                bad.insert(h.clone());
            }
        }
    }
    ratchet("RS-114", bad, &[]);
}

// ---------------------------------------------------------------- coverage RS-120..RS-125

fn test_corpus() -> &'static String {
    static S: OnceLock<String> = OnceLock::new();
    S.get_or_init(|| {
        let mut s = String::new();
        for f in files_under("crates") {
            if f.contains("/tests/") && f.ends_with(".rs") {
                s.push_str(&read(&f));
                s.push('\n');
            }
        }
        s
    })
}

#[test]
fn rs_120_every_shipped_file_is_named_by_a_test() {
    let corpus = test_corpus();
    let f = files_under("files")
        .into_iter()
        .filter(|f| !corpus.contains(base(f)))
        .collect();
    ratchet("RS-120", f, B_UNTESTED);
}

#[test]
fn rs_121_doc_and_tests_agree() {
    let doc = read("docs/RULES-system.md");
    assert!(!doc.is_empty(), "docs/RULES-system.md");
    let src = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/rules_system.rs"),
    )
    .unwrap();
    let mut tests: BTreeMap<String, String> = BTreeMap::new();
    for l in src.lines() {
        if let Some(r) = l.strip_prefix("fn rs_") {
            let name = r.split('(').next().unwrap();
            tests.insert(format!("RS-{}", &name[..3]), format!("rs_{name}"));
        }
    }
    let mut documented = BTreeMap::new();
    let mut manual = BTreeSet::new();
    for l in doc.lines().filter(|l| l.starts_with("| RS-")) {
        let c: Vec<&str> = l.split('|').map(str::trim).collect();
        // | id | area | rule | test | status |
        if c.len() >= 6 {
            if c[5] == "manual" {
                manual.insert(c[1].to_string());
            } else {
                documented.insert(c[1].to_string(), c[4].trim_matches('`').to_string());
            }
        }
    }
    for (id, fnname) in &documented {
        assert_eq!(
            tests.get(id),
            Some(fnname),
            "{id}: doc names test `{fnname}`"
        );
    }
    for id in tests.keys() {
        assert!(
            documented.contains_key(id) || id == "RS-000",
            "{id}: test without a row in docs/RULES-system.md"
        );
    }
    assert!(
        documented.len() >= 60,
        "{} rules documented",
        documented.len()
    );
    assert!(!manual.is_empty(), "the manual rules are listed");
}

#[test]
fn rs_122_baselines_are_sorted_and_unique() {
    for (n, b) in [
        ("B_SET_EU", B_SET_EU),
        ("B_EVAL", B_EVAL),
        ("B_RM_RF", B_RM_RF),
        ("B_QUIET_WRITES", B_QUIET_WRITES),
        ("B_BASHISM", B_BASHISM),
        ("B_BACKTICK", B_BACKTICK),
        ("B_TMP", B_TMP),
        ("B_CURL", B_CURL),
        ("B_READ_R", B_READ_R),
        ("B_DEAD_SCRIPT", B_DEAD_SCRIPT),
        ("B_MKTEMP_TRAP", B_MKTEMP_TRAP),
        ("B_RESTART", B_RESTART),
        ("B_NET_WANTS", B_NET_WANTS),
        ("B_UNIT_SHELL", B_UNIT_SHELL),
        ("B_SYSCTL_DOC", B_SYSCTL_DOC),
        ("B_SYSCTL_COMMENT", B_SYSCTL_COMMENT),
        ("B_ICON", B_ICON),
        ("B_UNTESTED", B_UNTESTED),
        ("B_HEADER", B_HEADER),
        ("B_BIND_ROWS", B_BIND_ROWS),
        ("B_SH_C", B_SH_C),
        ("B_HOME", B_HOME),
        ("B_WHICH", B_WHICH),
        ("B_PROTECT_SYSTEM", B_PROTECT_SYSTEM),
        ("B_HYPR_DUP", B_HYPR_DUP),
        ("B_HARDENING_COMMENT", B_HARDENING_COMMENT),
        ("B_HYPR_HEADER", B_HYPR_HEADER),
        ("B_SHEBANG", B_SHEBANG),
    ] {
        let mut s = b.to_vec();
        s.sort_unstable();
        s.dedup();
        assert_eq!(s, b, "{n}: keep the baseline sorted, without duplicates");
    }
}

// ---------------------------------------------------------------- registry icons, brightness keys

/// Every module record's `icon` (wol_core::modules::Module::icon) is drawn in the WolOS icon set:
/// a `*-symbolic` name in design/icons/symbolic, else an app icon or one of its aliases. The
/// installer Modules step and the Store show it; a missing name is the missing-icon glyph.
#[test]
fn module_registry_icons_exist_in_the_icon_set() {
    let aliases: BTreeSet<String> = [
        "design/icons/aliases.tsv",
        "design/icons/places-aliases.tsv",
    ]
    .iter()
    .flat_map(|f| {
        read(f)
            .lines()
            .filter(|l| !l.starts_with('#'))
            .flat_map(|l| l.split_whitespace().map(String::from).collect::<Vec<_>>())
            .collect::<Vec<_>>()
    })
    .collect();
    let records: Vec<String> = files_under("files/usr/share/wolos/modules")
        .into_iter()
        .filter(|f| f.ends_with(".json"))
        .collect();
    assert!(records.len() >= 7, "{records:?}");
    let mut bad = BTreeSet::new();
    for f in &records {
        let t = read(f);
        let icon = t
            .lines()
            .find_map(|l| l.trim().strip_prefix("\"icon\":"))
            .and_then(|v| v.split('"').nth(1))
            .unwrap_or("")
            .to_string();
        let drawn = if icon.ends_with("-symbolic") {
            exists(&format!("design/icons/symbolic/{icon}.svg"))
        } else {
            exists(&format!("design/icons/apps/{icon}.svg")) || aliases.contains(&icon)
        };
        if icon.is_empty() || !drawn {
            bad.insert(format!("{f}: icon {icon:?}"));
        }
    }
    assert!(
        bad.is_empty(),
        "registry icons not in design/icons:\n{}",
        bad.into_iter().collect::<Vec<_>>().join("\n")
    );
}

/// The brightness keys run the shipped wol-backlight (not a program nothing installs), which steps
/// the level by 5 through `wol display brightness` and clamps it to 1..100.
#[test]
fn brightness_keys_run_the_shipped_wol_backlight() {
    let conf = read(&format!("{HYPR_DIR}/hyprland.conf"));
    for (key, word) in [
        ("XF86MonBrightnessUp", "up"),
        ("XF86MonBrightnessDown", "down"),
    ] {
        let l = conf
            .lines()
            .find(|l| l.contains(key))
            .unwrap_or_else(|| panic!("{key} is bound"));
        assert!(
            l.trim_end()
                .ends_with(&format!("exec, wol-backlight {word}")),
            "{l}"
        );
    }
    assert!(exists("desktop/overlay/usr/bin/wol-backlight"));
    let d = std::env::temp_dir().join(format!("wolos-backlight-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("bin")).unwrap();
    let fake = d.join("bin/wol");
    std::fs::write(
        &fake,
        "#!/bin/sh\nif [ \"$1 $2\" = 'display --json' ]; then printf '%s\n' \"$BL_JSON\"; exit 0; fi\necho \"$*\" >>\"$LOG\"\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let run = |arg: &str, json: &str| -> Option<(i32, String)> {
        let log = d.join("log");
        let _ = std::fs::remove_file(&log);
        let o = Command::new("sh")
            .arg(root().join("desktop/overlay/usr/bin/wol-backlight"))
            .arg(arg)
            .env("PATH", format!("{}:/usr/bin:/bin", d.join("bin").display()))
            .env("BL_JSON", json)
            .env("LOG", &log)
            .output()
            .ok()?;
        Some((
            o.status.code().unwrap_or(-1),
            std::fs::read_to_string(&log).unwrap_or_default(),
        ))
    };
    let pretty = "{\n  \"brightness\": 50,\n  \"monitors\": [{\"name\": \"eDP-1\"}]\n}";
    let Some(first) = run("up", pretty) else {
        eprintln!("no sh on this box; wol-backlight not run");
        return;
    };
    assert_eq!(first, (0, "display brightness 55\n".into()));
    assert_eq!(
        run("down", r#"{"brightness":50,"monitors":[]}"#),
        Some((0, "display brightness 45\n".into()))
    );
    assert_eq!(
        run("up", r#"{"brightness":98}"#),
        Some((0, "display brightness 100\n".into()))
    );
    assert_eq!(
        run("down", r#"{"brightness":3}"#),
        Some((0, "display brightness 1\n".into()))
    );
    // No backlight (desktop monitor, VM): nothing to set, no error.
    assert_eq!(
        run("up", r#"{"brightness":null,"monitors":[]}"#),
        Some((0, String::new()))
    );
    assert_eq!(
        run("sideways", r#"{"brightness":50}"#).map(|r| r.0),
        Some(64)
    );
    let _ = std::fs::remove_dir_all(&d);
}

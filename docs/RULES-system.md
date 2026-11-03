<!-- SPDX-License-Identifier: GPL-2.0-or-later -->
# System-file rules (RS)

Consistency rules for what WolOS ships under `files/` and `desktop/`: shell scripts, systemd units, sysctl, polkit, compositor config, `.desktop` files and pacman hooks. Each automated rule is one test in `crates/controller/tests/rules_system.rs` (function name in the Test column). The tests read the shipped files and execute nothing except `sh -n` and `shellcheck` (when installed). Run: `cargo test -p controller --test rules_system`.

## How baselines work

A file that does not meet a rule today is listed in a `B_*` constant at the top of the test file (status `baseline`). The list only shrinks: a new violation fails the test; a fixed one is reported on stderr so its entry is removed. `B_HYPR_COLOURS` and `B_UNQUOTED` are ceilings (counts), not lists. A new baseline entry needs a reason in the commit that adds it.

Narrower contract tests stay the authority for their units: `units.rs` (hardening values), `polkit.rs` (the 10-wolos.rules logic), `sysctl_docs.rs` (documented values), `keys.rs`, `image_install.rs`, `image_systemd.rs`.

## Rules

| ID | Area | Rule | Test | Status |
|---|---|---|---|---|
| RS-001 | script | Shebang is exactly `#!/bin/sh` (POSIX sh; no bash-only scripts). | `rs_001_shebang_is_bin_sh` | enforced |
| RS-002 | script | Line 2 is the `SPDX-License-Identifier:` line. | `rs_002_spdx_on_line_two` | enforced |
| RS-003 | script | Header: at least two comment lines (60+ characters) after SPDX say what the script does, why, and how it is run. | `rs_003_header_says_what_why_how` | baseline |
| RS-004 | script | `set -eu` within the first code lines, or `set -u` with a comment that names `set -e` and why it is off. | `rs_004_strict_mode` | baseline |
| RS-005 | script | No `eval`. | `rs_005_no_eval` | baseline |
| RS-006 | script | No `sh -c` / `bash -c` on a variable. | `rs_006_no_shell_c_of_a_variable` | baseline |
| RS-007 | script | Every `rm -rf` argument with a variable is guarded with `${var:?}`. | `rs_007_rm_rf_variable_paths_are_guarded` | baseline |
| RS-008 | script | No `2>/dev/null` on a disk, key, boot, TPM or database write (mkfs, sgdisk, cryptsetup, openssl, efibootmgr, sbctl, bootctl, systemd-repart, pacman-key, dd, ...). | `rs_008_no_quiet_failure_on_disk_key_boot_db_writes` | baseline |
| RS-009 | script | No `curl`/`wget` piped to a shell. | `rs_009_no_pipe_to_shell` | enforced |
| RS-010 | script | `sudo`, `pkexec`, `doas` only in the allowed list (wol-install, ADR 0023). | `rs_010_sudo_only_where_allowed` | enforced |
| RS-011 | script | Every variable expansion is quoted (heuristic scanner; a per-script count that may only fall). | `rs_011_expansions_are_quoted` | baseline |
| RS-012 | script | Every script parses with `sh -n`. | `rs_012_scripts_parse_with_sh_n` | enforced |
| RS-013 | script | `shellcheck -S error -s sh` is clean when shellcheck is installed. | `rs_013_shellcheck_has_no_errors` | enforced |
| RS-014 | script | No bashisms (`[[`, `<(`, `$RANDOM`, `echo -e`, `&>`, `source`, `function`). | `rs_014_no_bashisms` | enforced |
| RS-015 | script | No backtick command substitution; use `$(...)`. | `rs_015_no_backticks` | enforced |
| RS-016 | script | No fixed `/tmp/` paths; use `mktemp`. | `rs_016_no_fixed_tmp_paths` | enforced |
| RS-017 | script | Nothing is made world-writable (`chmod 777`, `a+w`, `o+w`). | `rs_017_nothing_is_made_world_writable` | enforced |
| RS-018 | script | No `/home/` literals in code. | `rs_018_no_home_directory_literals` | baseline |
| RS-019 | script | No `set -x` (traces leak keys and passwords). | `rs_019_no_trace_mode` | enforced |
| RS-020 | script | Every `curl` has `-f`/`--fail` and a time limit (`-m`, `--max-time`, `--connect-timeout`). | `rs_020_curl_fails_on_http_errors_and_has_a_time_limit` | baseline |
| RS-021 | script | `read` is `read -r`. | `rs_021_read_uses_dash_r` | enforced |
| RS-022 | script | `command -v`, not `which`. | `rs_022_command_v_not_which` | baseline |
| RS-023 | script | No carriage returns in shipped text under files/ and desktop/overlay/ (a CR breaks a shebang). | `rs_023_no_carriage_returns_in_shipped_text` | enforced |
| RS-024 | script | Every script in /usr/lib/wolos is referenced by a unit, hook, config, Rust source or other script. | `rs_024_every_wolos_script_is_referenced` | baseline |
| RS-025 | script | No literal credentials assigned in code. | `rs_025_no_literal_credentials` | enforced |
| RS-026 | script | No passphrase or key passed as a command-line argument built from a variable (visible in /proc). | `rs_026_scripts_do_not_pipe_secrets_through_echo_args` | enforced |
| RS-027 | script | A script that calls `mktemp` also has a `trap` to clean up. | `rs_027_mktemp_has_a_trap` | baseline |
| RS-028 | script | Scripts end with a newline. | `rs_028_scripts_end_with_a_newline` | enforced |
| RS-029 | unit | At least 15 system services and 5 timers are found (guards the discovery). | `rs_029_units_found` | enforced |
| RS-030 | unit | Every root service (no `User=`) has `NoNewPrivileges=yes`, `LockPersonality=yes`, `RestrictRealtime=yes`, `SystemCallArchitectures=native`, or is in the documented exemption list (wold.service). | `rs_030_root_services_carry_the_baseline_hardening_keys` | enforced |
| RS-031 | unit | Every root service sets `ProtectSystem=`. | `rs_031_root_services_protect_the_system_tree` | baseline |
| RS-032 | unit | Every service that runs as a non-root `User=` has `NoNewPrivileges=yes`. | `rs_032_unprivileged_services_cannot_gain_privileges` | enforced |
| RS-033 | unit | Every `Exec*=` program path is absolute. | `rs_033_exec_paths_are_absolute` | enforced |
| RS-034 | unit | Every `Exec*=` program exists in files/, is a crate binary, or is a known packaged program. | `rs_034_exec_programs_exist` | enforced |
| RS-035 | unit | No explicit `User=root`. | `rs_035_no_explicit_user_root` | enforced |
| RS-036 | unit | Every non-oneshot service states a `Restart=` policy. | `rs_036_long_running_services_state_a_restart_policy` | baseline |
| RS-037 | unit | Every timer with `OnCalendar=` has `Persistent=true`. | `rs_037_calendar_timers_are_persistent` | enforced |
| RS-038 | unit | No `After=network.target`; `After=network-online.target` comes with `Wants=network-online.target`. | `rs_038_network_ordering_comes_with_wants` | baseline |
| RS-039 | unit | Every timer and socket has the service it starts. | `rs_039_timers_and_sockets_have_their_service` | enforced |
| RS-040 | unit | Every unit has `Description=`. | `rs_040_units_have_a_description` | enforced |
| RS-041 | unit | Timers are `WantedBy=timers.target`, sockets `WantedBy=sockets.target`. | `rs_041_timers_and_sockets_are_wanted_by_their_target` | enforced |
| RS-042 | unit | No deprecated keys (`MemoryLimit=`, `CPUShares=`, `BlockIOWeight=`, `PermissionsStartOnly=`, `StartLimit*` outside [Unit]). | `rs_042_no_deprecated_unit_keys` | enforced |
| RS-043 | unit | No unit starts a shell as its program. | `rs_043_units_do_not_start_shells` | enforced |
| RS-044 | unit | Drop-in directories end in `.d` and hold only `.conf` files. | `rs_044_dropin_dirs_hold_only_conf_files` | enforced |
| RS-045 | unit | Unit names start with `wolos-` (or `wold.`). | `rs_045_unit_names_carry_the_wolos_prefix` | enforced |
| RS-046 | unit | `ReadWritePaths=`, `ReadOnlyPaths=`, `InaccessiblePaths=` are absolute. | `rs_046_read_write_paths_are_absolute` | enforced |
| RS-047 | unit | No secret-looking `Environment=` values. | `rs_047_no_secrets_in_unit_environment` | enforced |
| RS-048 | unit | A oneshot unit never has `Restart=always`. | `rs_048_oneshot_units_do_not_restart_always` | enforced |
| RS-049 | unit | Ordering and dependency keys that name a wolos unit name one that exists. | `rs_049_ordering_names_a_unit_that_exists` | enforced |
| RS-050 | unit | Hardened root units carry the `# Hardening` comment that says what the daemon needs. | `rs_050_hardening_is_explained` | enforced |
| RS-051 | unit | No `.slice`/`.mount`/`.scope` units are shipped; limits are drop-ins. | `rs_051_system_slices_are_dropins_only` | enforced |
| RS-060 | sysctl | Every sysctl key is named in docs/modules/security.md. | `rs_060_every_sysctl_key_is_documented` | baseline |
| RS-061 | sysctl | Every key `wol security` reports (`HARDENING`) is documented. | `rs_061_wol_security_reports_documented_keys` | enforced |
| RS-062 | sysctl | Files are named `NN-wolos*.conf`. | `rs_062_sysctl_files_are_ordered_wolos_confs` | enforced |
| RS-063 | sysctl | Every line is `key = value` with a plain key and a value free of shell characters. | `rs_063_sysctl_lines_are_key_equals_value` | enforced |
| RS-064 | sysctl | No key is set to two different values across files. | `rs_064_no_key_gets_two_different_values` | enforced |
| RS-065 | sysctl | No setting weakens hardening (ptrace_scope 0, randomize_va_space 0, kptr_restrict 0, dmesg_restrict 0, unprivileged BPF, suid_dumpable, protected_*, mmap_min_addr 0, ...). | `rs_065_no_setting_weakens_hardening` | enforced |
| RS-066 | sysctl | Every file opens with a comment. | `rs_066_sysctl_files_open_with_a_comment` | enforced |
| RS-067 | sysctl | Every key has a comment on its line or in the six lines above. | `rs_067_sysctl_keys_have_a_reason_nearby` | enforced |
| RS-070 | polkit | The polkit directories hold only `rules.d/*.rules` files. | `rs_070_polkit_dir_holds_only_rules_files` | enforced |
| RS-071 | polkit | No `Result.YES`, `AUTH_*_KEEP`, `polkit.spawn` or `addAdminRule` in code. | `rs_071_polkit_never_says_yes` | enforced |
| RS-072 | polkit | Rules sort before the distribution defaults: `NN-wolos*.rules` with NN below 50. | `rs_072_polkit_rules_sort_before_the_distribution_defaults` | enforced |
| RS-073 | polkit | Each rules file uses `polkit.addRule(` and refuses `org.wolos.*` actions. | `rs_073_polkit_rules_use_add_rule_and_refuse_wolos_actions` | enforced |
| RS-074 | polkit | No `.policy` action files are shipped (WolOS actions go through wol-admind and wold). | `rs_074_no_polkit_action_definitions_are_shipped` | enforced |
| RS-075 | polkit | Rules files start with SPDX and a `//` header. | `rs_075_polkit_rules_carry_spdx_and_a_header` | enforced |
| RS-080 | hyprland | Every `bind` in hyprland.conf and tiling.conf has a named row in the shortcut sheet (`wol_core::shortcuts::name_of`). | `rs_080_every_bind_has_a_shortcut_sheet_row` | baseline |
| RS-081 | hyprland | Every `exec`/`exec-once`/bind-exec program exists (crate binary, shipped file or known package). | `rs_081_exec_targets_exist` | enforced |
| RS-082 | hyprland | Colour literals (`rgb(`, `rgba(`, `#hex`) in hyprland.conf do not grow; theme values come from the theme. | `rs_082_colours_do_not_grow` | baseline |
| RS-083 | hyprland | Every `windowrule`/`layerrule` follows a comment (or a rule that does) or has an inline comment. | `rs_083_every_windowrule_is_commented` | enforced |
| RS-084 | hyprland | No `/home/` or `/Users/` literals in any compositor conf. | `rs_084_no_home_directory_literals_in_hypr_confs` | enforced |
| RS-085 | hyprland | No exec of `sudo`, `pkexec`, `doas`, `su`, `curl`, `wget` in a compositor conf. | `rs_085_session_confs_never_exec_with_privilege_or_network_fetch` | enforced |
| RS-086 | hyprland | Every `source =` under /usr/share/wolos/hypr exists. | `rs_086_sourced_files_exist` | enforced |
| RS-087 | hyprland | permissions.conf enforces permissions and each kind (screencopy, cursorpos, input-capture, plugin) ends with `.*, deny`. | `rs_087_permissions_end_in_deny_all_per_kind` | enforced |
| RS-088 | hyprland | hyprland.conf sources permissions.conf. | `rs_088_desktop_conf_sources_the_permissions` | enforced |
| RS-089 | hyprland | Every conf opens with a comment. | `rs_089_hypr_confs_open_with_a_comment` | baseline |
| RS-090 | hyprland | No `windowrulev2`; `layerrule` uses `match:`. | `rs_090_no_windowrulev2_or_legacy_layerrule_syntax` | enforced |
| RS-091 | hyprland | No two binds share a key combination. | `rs_091_no_two_binds_share_a_key_combination` | enforced |
| RS-092 | hyprland | login.conf, setup.conf and installer.conf start the program this repo builds for them. | `rs_092_session_confs_name_the_login_and_installer_programs` | enforced |
| RS-093 | hyprland | hypridle commands exist (crate binary or known tool). | `rs_093_hypridle_commands_exist` | enforced |
| RS-100 | desktop | Every `.desktop` `Exec` program exists. | `rs_100_desktop_exec_exists` | enforced |
| RS-101 | desktop | `Icon` is set; an absolute Icon path exists in the repo. | `rs_101_desktop_icon_is_set_and_absolute_icons_exist` | baseline |
| RS-102 | desktop | `Categories` is set and ends with `;`. | `rs_102_desktop_categories_are_set_and_terminated` | enforced |
| RS-103 | desktop | No user paths (`/home/`, `/Users/`, `C:\`). | `rs_103_desktop_files_name_no_user_paths` | enforced |
| RS-104 | desktop | Starts with `[Desktop Entry]`, `Type=Application`, `Name` set. | `rs_104_desktop_entries_have_type_name_and_header` | enforced |
| RS-105 | desktop | `MimeType` ends with `;`. | `rs_105_mime_types_end_with_a_semicolon` | enforced |
| RS-106 | desktop | `Exec` field codes are valid. | `rs_106_exec_field_codes_are_valid` | enforced |
| RS-107 | desktop | `Exec` does not use `sudo`, `pkexec`, `su`, `doas`. | `rs_107_desktop_exec_has_no_privilege_escalation` | enforced |
| RS-110 | hook | A pacman hook has Trigger (Operation, Type, Target) and Action (Description, When, Exec). | `rs_110_hooks_have_trigger_and_action_keys` | enforced |
| RS-111 | hook | The hook `Exec` program is an absolute path that exists. | `rs_111_hook_programs_exist` | enforced |
| RS-112 | hook | Hook files end in `.hook` and are named `[NN-]wolos*`. | `rs_112_hook_files_are_named_for_wolos` | enforced |
| RS-113 | hook | `Operation` is Install, Upgrade or Remove; `When` is PreTransaction or PostTransaction. | `rs_113_hook_operations_and_timing_are_valid` | enforced |
| RS-114 | hook | An inline `sh -c` hook is guarded (an or-list, an and-list or a test) so an absent input does not fail the transaction. | `rs_114_hook_shell_commands_are_guarded` | enforced |
| RS-120 | coverage | Every file under files/ is named by a contract test. | `rs_120_every_shipped_file_is_named_by_a_test` | baseline |
| RS-121 | meta | This document and the `rs_nnn_*` tests list the same rules, and each documented test exists. | `rs_121_doc_and_tests_agree` | enforced |
| RS-122 | meta | Baselines are sorted and free of duplicates. | `rs_122_baselines_are_sorted_and_unique` | enforced |

## Not automated

| ID | Area | Rule | Test | Status |
|---|---|---|---|---|
| RS-190 | script | Scripts are installed mode 0755 (set by the package recipe; checkouts do not carry modes everywhere). | none | manual |
| RS-191 | unit | `systemd-analyze security` exposure of each root unit stays at or below 50 (CI step, STANDARDS.md). | none | manual |
| RS-192 | hyprland | `Hyprland --verify-config` accepts the shipped confs on the pinned compositor version. | none | manual |
| RS-193 | desktop | `desktop-file-validate` accepts each `.desktop` file (tool not on the build box). | none | manual |
| RS-194 | script | Behaviour of each script is covered by a runtime harness (these tests read the files, they do not run them). | none | manual |
| RS-195 | sysctl | A documented value is accepted by the kernel of the target image (needs a booted image). | none | manual |
| RS-196 | script | Whether a flagged expansion is really unsafe (the RS-011 scanner is a heuristic, not a parser). | none | manual |

## Baselines today

- RS-003: `B_HEADER`
- RS-004: `B_SET_EU`
- RS-005: `B_EVAL`
- RS-006: `B_SH_C`
- RS-007: `B_RM_RF`
- RS-008: `B_QUIET_WRITES`
- RS-011: `B_UNQUOTED`
- RS-018: `B_HOME`
- RS-020: `B_CURL`
- RS-022: `B_WHICH`
- RS-024: `B_DEAD_SCRIPT`
- RS-027: `B_MKTEMP_TRAP`
- RS-031: `B_PROTECT_SYSTEM`
- RS-036: `B_RESTART`
- RS-038: `B_NET_WANTS`
- RS-060: `B_SYSCTL_DOC`
- RS-080: `B_BIND_ROWS`
- RS-082: `B_HYPR_COLOURS`
- RS-089: `B_HYPR_HEADER`
- RS-101: `B_ICON`
- RS-120: `B_UNTESTED`

## Adding a rule

1. Add `fn rs_nnn_name()` to `rules_system.rs` (next free id in the area).
2. Add its row above (ID, Area, Rule, Test = the function name, Status).
3. If shipped files fail it today, list them in a new `B_*` constant, add it to `rs_122`, and mark the row `baseline`.

`rs_121_doc_and_tests_agree` fails when the two drift apart.

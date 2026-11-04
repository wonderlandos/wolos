# WolOS documentation

**Concept.** The index of everything under `docs/`: the rules every change follows (Doctrine), the
decisions (ADRs), one document per module, and research notes. **Why.** One place to find a rule or a
decision before writing code; a document not listed here is a review finding (`STANDARDS.md` §5).
**How.** Read Doctrine first, then the module you touch and the ADRs it names.

## Architecture map

Base: Arch Linux snapshot (ADR 0016). Compositor: a tuned fork of an open source Wayland compositor (ADR 0001).
Layers: kernel (fork `wonderlandos/linux`, package in wolos-pkgbuilds) -> packages (wolos-pkgbuilds: this code at a pinned commit) -> image (wolos-iso: installs the published packages; ADR 0029).
Rust userland: one Cargo workspace in `crates/` (ADR 0002). Rules enforced by `crates/shared/tests/architecture.rs`.
Rule: prefer upstream ports + config over new code. The whole build picture: [`../BUILD.md`](../BUILD.md).

## Doctrine

| Document | What it is |
|---|---|
| [STANDARDS.md](STANDARDS.md) | coding standards for Rust, shell, Crystal, C/C++, configs, docs, commits and security (§7.1: the v0.12 hardening rules, one helper per rule); lines marked **CI** are checked |
| [REVIEW.md](REVIEW.md) | the post-change audit (second pass): how to get a box, the general list, per-component briefs, the report format |
| [DOCTRINE.md](DOCTRINE.md) | the code of conduct ("The Wonderland Doctrine"): who you are here, what is and is not welcome in project spaces, upstreams, and how reports are handled (conduct@wolos.org); `CODE_OF_CONDUCT.md` at the repository root points to it |
| [RULES.md](RULES.md) | the rule catalogue: small concrete rules, each with an id, a reason and the check that enforces it |
| [RULES-system.md](RULES-system.md) | system-file rules (RS-nnn) for scripts, units, sysctl, polkit, compositor config and pacman hooks; checked by `crates/controller/tests/rules_system.rs` |
| [RULES-ui.md](RULES-ui.md) | UI consistency rules (RU-nnn) for copy, widgets, CSS, goldens and docs; checked by `crates/shell/tests/rules_ui.rs` |
| [RULES-ruby.md](RULES-ruby.md) | Ruby rules (RB-nnn) for Ruby scripts: header, argv-only commands, no eval or object-building decoders, default gems only, `ruby -wc` and one explicit RuboCop list, no Ruby root helpers; checked by `crates/controller/tests/rules_ruby.rs` |
| [RULES-crystal.md](RULES-crystal.md) | Crystal rules (RCR-nnn) for `.cr` code, the ameba and formatter setup, FFI limits, known-vulnerable patterns and the shard supply chain; checked by `crates/controller/tests/rules_crystal.rs` |
| [THIRD-PARTY.md](THIRD-PARTY.md) | third-party notices: the only place where projects we translated code from, and their copyright holders, are named; shipped as `/usr/share/licenses/<package>/THIRD-PARTY.md` |
| ADR process | a decision is a file `adr/NNNN-title.md` with Status, Context, Decision, Consequences (**CI** `tools/check-adrs.sh`). A decision changes only by a new ADR that names the old one; the old one's Status says `superseded by NNNN`. `mem adr` is the maintainers' tooling that drafts them; the files in `adr/` are the record. |

## Decisions (ADRs)

| ADR | Decision | Status |
|---|---|---|
| [0001](adr/0001-freebsd-ports-not-from-scratch.md) | Fork and tune mature software, not a new stack | accepted; its base is superseded by 0016, the fork-and-tune decision stands |
| [0002](adr/0002-rust-port-of-iaraos.md) | Rust port of IaraOS userland on the forked compositor | accepted; its syscall line is superseded by 0016 |
| [0003](adr/0003-login-socket-power.md) | login socket: power requests | accepted |
| [0004](adr/0004-setup-assistant.md) | first-boot setup assistant (wol-setup) | accepted |
| [0005](adr/0005-overview-surface.md) | Overview is a wol-shell layer surface | accepted |
| [0006](adr/0006-vendored-blocking-idle-exit.md) | vendored `blocking` with idle thread exit | accepted |
| [0007](adr/0007-gpu-tuning-interface.md) | GPU tuning interface | accepted; its counter sources follow 0016 |
| [0008](adr/0008-i915-frame-deadline-boost.md) | i915kms: pinned drm-kmod build | superseded by 0016 |
| [0009](adr/0009-live-iso.md) | live ISO boots the desktop | superseded by 0016 |
| [0010](adr/0010-desktop-widgets.md) | desktop widgets: one layer surface, data packages, no web view | accepted |
| [0011](adr/0011-wol-packages.md) | .wol packages: one signed file around a pkg(8) package | superseded by 0021 |
| [0012](adr/0012-seamless-boot.md) | one picture from power-on to the desktop | accepted; its console mechanics follow 0016 |
| [0013](adr/0013-first-run.md) | first run: Install WolOS app and Setup Assistant v2 | accepted; its install backend is replaced by 0018 and 0023 |
| [0014](adr/0014-power-policy.md) | the power source picks the profile | accepted; its triggers follow 0016 |
| [0015](adr/0015-security-defaults.md) | secure by default, honest about the rest | superseded by 0040 |
| [0016](adr/0016-arch-linux-base.md) | Arch Linux base, released in snapshots | accepted; its build parts are replaced by 0029 |
| [0017](adr/0017-wolos-base.md) | One base system: WolKernel and userland released together | accepted; its build parts are replaced by 0029 |
| [0018](adr/0018-installer-systemd-repart.md) | Installer disk layout from systemd-repart | accepted |
| [0019](adr/0019-wolos-mirror.md) | WolOS mirror: [wolos], [wolos-cachy], Arch | superseded by 0030 |
| [0020](adr/0020-libwol-stable-abi.md) | libwol: one app API behind a stable C ABI | accepted |
| [0021](adr/0021-wopkg.md) | wopkg: one package tool for every source, libwopkg behind a stable C ABI | accepted |
| [0022](adr/0022-aur-laws.md) | Community-collection packages judged by laws over graphs | accepted |
| [0023](adr/0023-installer-plans.md) | The installer writes a plan, and a plan names what it may touch | accepted |
| [0024](adr/0024-first-use-malware-check.md) | A first-use malware check, not a file crawler | accepted |
| [0025](adr/0025-signed-boot.md) | Signed boot with this computer's own keys | accepted |
| [0026](adr/0026-memory-saver.md) | Memory Saver | accepted |
| [0027](adr/0027-one-app-cannot-take-the-desktop.md) | One app cannot take the desktop with it | accepted |
| [0028](adr/0028-clamav-in-its-own-process.md) | The full signature engine in its own process, with data the person fetches | accepted |
| [0029](adr/0029-repository-split-and-single-build-path.md) | Repository split and a single build path | accepted |
| [0030](adr/0030-repo-cdn-cachy-redirect.md) | The package repo on CloudFront; CachyOS repos through a redirect, not a copy | accepted |
| [0031](adr/0031-shell-plugins-themes-layouts.md) | Shell plugins, themes and layouts | accepted |
| [0032](adr/0032-optional-modules.md) | Optional modules: AI, developer, creative and console stacks | accepted |
| [0033](adr/0033-identity-handshake.md) | wold greets first: identity handshake on the control socket | accepted |
| [0034](adr/0034-root-staged-installs.md) | Locally built packages: who installs the staged copy | accepted |
| [0035](adr/0035-layer-hold-on-slot-switch.md) | Live layers on a slot switch: held, not merged, until the new slot is good | proposed |
| [0036](adr/0036-crystal-userland.md) | Crystal is the userland language: bindings, wolkit, GPU surface, shard index | accepted |
| [0037](adr/0037-shower-identity.md) | The shower is the shell wold's session started, not any process running its file | proposed |
| [0038](adr/0038-sound-service.md) | One sound service, and 100% is 0 dB | accepted |
| [0039](adr/0039-libwolkit-and-wolkit.md) | libWolkit (the engine) and wolkit (the shard) | accepted |
| [0040](adr/0040-security-defaults-on-linux.md) | Security defaults on Linux | accepted |
| [0041](adr/0041-ruby-scripts.md) | Ruby scripts run like shell scripts (`wol script`) | accepted |
| [0042](adr/0042-wolos-web-runtime.md) | A WolOS web-app runtime on the Rust browser engine: two tracks, one engine library | accepted |
| [0044](adr/0044-etc-overlay-across-slot-switches.md) | The /etc overlay across slot switches | accepted |
| [0048](adr/0048-system-web-apps.md) | System web apps: a folder per app, one bridge to `wol` verbs by permission, one kit | proposed |
| [0051](adr/0051-house-in-order.md) | 0.13: put the house in order before adding anything (proof rule, root-cause standards, wave gates) | proposed |

## Modules

Each module document names its ADR and its code by crate and path.

| Document | Content | ADR |
|---|---|---|
| [modules/abi.md](modules/abi.md) | libwol: the app API (`wol.h`, `libwol.so.1`), ABI rules, planned families, Lua widgets | [0020](adr/0020-libwol-stable-abi.md) |
| [modules/app-theming.md](modules/app-theming.md) | app theming (`wol theme set <id>`): toolkit files for GTK, Qt/Kvantum, Firefox, Helium, editors | [0031](adr/0031-shell-plugins-themes-layouts.md) |
| [modules/audio.md](modules/audio.md) | sound: PipeWire, `wol-volume`, the volume OSD, devices | [0038](adr/0038-sound-service.md) |
| [modules/doctor.md](modules/doctor.md) | Doctor: every check of the system in one list (`wol doctor`, the Doctor app), each with its fix | [0048](adr/0048-system-web-apps.md) |
| [modules/cryswol.md](modules/cryswol.md) | Cryswol: the Crystal shard over libwol (`require "cryswol"`, namespace `Wol`): layers, ownership and callback rules, errors, threads, build, how to add a verb | [0036](adr/0036-crystal-userland.md) |
| [modules/libwolkit.md](modules/libwolkit.md) | libWolkit: the UI engine (`wolkit.h`, `libwolkit.so.1`): scene and patch stream, style, layout, text, paint, CPU and GPU renderers, window backends, limits | [0039](adr/0039-libwolkit-and-wolkit.md) |
| [modules/wolkit.md](modules/wolkit.md) | Wolkit: the Crystal UI kit over libWolkit (`require "wolkit"`): DSL, signals, components, test harness, developer tools | [0039](adr/0039-libwolkit-and-wolkit.md) |
| [modules/wolkit-style.md](modules/wolkit-style.md) | the `.wolcss` style language: properties, tokens, selectors, lints `WK-nnn` (generated by `wolkit-stylec --doc`) | [0039](adr/0039-libwolkit-and-wolkit.md) |
| [modules/wolkit-tutorial.md](modules/wolkit-tutorial.md) | a first Wolkit app in fifteen minutes: a to-do list in eight steps | [0039](adr/0039-libwolkit-and-wolkit.md) |
| [modules/wolkit-bench.md](modules/wolkit-bench.md) | Wolkit benchmarks: method, the budgets and what was measured | [0039](adr/0039-libwolkit-and-wolkit.md) |
| [modules/apps.md](modules/apps.md) | the bundled apps rewritten in Crystal on Wolkit: the list, what each one has, lessons | [0036](adr/0036-crystal-userland.md), [0039](adr/0039-libwolkit-and-wolkit.md) |
| [modules/gallery.md](modules/gallery.md) | Gallery (`org.wolos.gallery`, `wol-gallery`): picture library by date and place, viewer, Trash with Undo; how it is built and tested | [0039](adr/0039-libwolkit-and-wolkit.md) |
| [modules/aurlaws.md](modules/aurlaws.md) | aurlaws: community-collection packages judged by laws over dependency graphs | [0022](adr/0022-aur-laws.md) |
| [modules/automation.md](modules/automation.md) | helpers (`wol mcp`, scripts): policy, approval sheet wire, audit log | [0020](adr/0020-libwol-stable-abi.md) |
| [modules/console.md](modules/console.md) | the console: `su` through sudo in every shell, command hints (`wol hint`, `hints.tsv`) for what is missing or sealed | [0032](adr/0032-optional-modules.md) |
| [modules/boot.md](modules/boot.md) | quiet boot, ESP layout, systemd-boot A/B slots, Secure Boot, install safety checks | [0016](adr/0016-arch-linux-base.md), [0023](adr/0023-installer-plans.md), [0025](adr/0025-signed-boot.md), [0044](adr/0044-etc-overlay-across-slot-switches.md) |
| [modules/games.md](modules/games.md) | native Steam and Proton, `wol games`, `.exe` double-click on wine or umu | [0016](adr/0016-arch-linux-base.md) |
| [modules/hardware.md](modules/hardware.md) | hardware catalogue, `wol hw`, `wolos-hw.service`, shipped firmware, gaps | [0016](adr/0016-arch-linux-base.md) |
| [modules/hig.md](modules/hig.md) | WolOS Human Interface Guidelines (binding for every app and shell surface) | [0016](adr/0016-arch-linux-base.md) |
| [modules/hue.md](modules/hue.md) | wol-hue: themes from a picture, 12 sliders, Base16 and colors.toml import, saved looks | [0031](adr/0031-shell-plugins-themes-layouts.md) |
| [modules/hyprland-fork.md](modules/hyprland-fork.md) | the compositor fork: base version, patches in `wolos-pkgbuilds/hyprland-wolos/`, config, QoS plugin | [0012](adr/0012-seamless-boot.md) |
| [modules/intel-graphics.md](modules/intel-graphics.md) | i915 firmware, tunables, VA-API/Vulkan stack (historical: FreeBSD base) | [0008](adr/0008-i915-frame-deadline-boost.md) |
| [modules/memory-bandwidth.md](modules/memory-bandwidth.md) | iGPU bandwidth paradigms, top 5, measurement | [0007](adr/0007-gpu-tuning-interface.md) |
| [modules/modules.md](modules/modules.md) | optional modules (AI, developer, creative, console stacks): what each installs | [0032](adr/0032-optional-modules.md) |
| [modules/packages.md](modules/packages.md) | `.wol` format, keys and trust, `wol install/remove/list/search`, index, factory | [0011](adr/0011-wol-packages.md) |
| [modules/performance.md](modules/performance.md) | measured desktop latency and frame targets, the probes and how to reproduce them | [0006](adr/0006-vendored-blocking-idle-exit.md) |
| [modules/monitor.md](modules/monitor.md) | wol-monitor: System Monitor (live graphs of processor, memory, disks, network, graphics, temperatures; apps from their scopes with Memory Saver state; process tree; disks; startup services) | |
| [modules/plugin-maker.md](modules/plugin-maker.md) | wol-plugin-maker: make a plugin or a theme and see it live (editor with the CLI check, sandboxed preview on four surfaces, Theme Studio, pack and share) | [0031](adr/0031-shell-plugins-themes-layouts.md) |
| [modules/plugins.md](modules/plugins.md) | shell plugins: manifest, Lua runtime, sandbox, `wol plugin check|run` | [0031](adr/0031-shell-plugins-themes-layouts.md) |
| [modules/power.md](modules/power.md) | power policy per power source and mode, sleep and lock, battery alerts, `wol power` | [0014](adr/0014-power-policy.md) |
| [modules/scripting.md](modules/scripting.md) | scripts in sh, bash and Ruby: `wol script check|run|on`, the clean environment, the helper gate, ssh | [0041](adr/0041-ruby-scripts.md) |
| [modules/security.md](modules/security.md) | firewall, hardening, update check, malware check, encrypted homes, app permissions, what runs as root | [0040](adr/0040-security-defaults-on-linux.md), [0024](adr/0024-first-use-malware-check.md) |
| [modules/shards.md](modules/shards.md) | Crystal shards: the signed curated list, `wol shard list|add|update|check`, pins by commit and `shard.lock` (libwopkg 1.1) | [0036](adr/0036-crystal-userland.md) |
| [modules/system-api.md](modules/system-api.md) | System API topics (`wol_core::system`) and their invariants | [0016](adr/0016-arch-linux-base.md) |
| [modules/userland.md](modules/userland.md) | Dock apps and app-ids, default apps, zsh, sudo, keyring, printing, USB sticks, size | [0016](adr/0016-arch-linux-base.md) |
| [modules/ux-journeys.md](modules/ux-journeys.md) | error copy rules (`shell::copy`, lint), where the journeys run now (`wolvm`), and the 2026-10-27 journey audit (historical) | [0013](adr/0013-first-run.md) |
| [modules/vm-e2e.md](modules/vm-e2e.md) | VM end to end: `wolvm` boots the ISO, drives the installer and the installed desktop step by step, snapshots and collects the journal | [0029](adr/0029-repository-split-and-single-build-path.md) |
| [modules/vm-graphics.md](modules/vm-graphics.md) | the desktop in a VM: what runs and why (historical record from the FreeBSD base) | none yet |
| [modules/webapps.md](modules/webapps.md) | web apps, both tracks: per-app policies (`webpolicy/<app>.json`, the patch hook), runtime profiles and flag rows | [0026](adr/0026-memory-saver.md) |
| [modules/webengine.md](modules/webengine.md) | the web-app runtime's engine: `crates/webengine`, `libwolwebengine.so.1`, the origin gate, process model, input bridge, gaps | [0042](adr/0042-wolos-web-runtime.md) |
| [modules/widgets.md](modules/widgets.md) | desktop widgets, Edit Widgets, building a widget package, `wol widgets` | [0010](adr/0010-desktop-widgets.md) |
| [modules/wifi.md](modules/wifi.md) | Wi-Fi: one interface, NetworkManager and iwd, join and its errors | [0016](adr/0016-arch-linux-base.md) |
| [modules/wopkg.md](modules/wopkg.md) | wopkg: one package tool for every source, libwopkg behind a stable C ABI | [0021](adr/0021-wopkg.md) |

## Other

| Document | Content |
|---|---|
| [creators-audio.md](creators-audio.md) | sound for creators: recording, streaming, producing |
| [plugins/plugin.schema.json](plugins/plugin.schema.json) | JSON schema of a shell plugin manifest (ADR 0031, [modules/plugins.md](modules/plugins.md)) |
| [bench/webapps-baseline.md](bench/webapps-baseline.md) | web-app baseline: what each web-based app costs in memory, start-up, idle and frames, Memory Saver on and off, the gap against native apps and the ranked wins; raw data in `bench/webapps-baseline.json` and `bench/webapps-roles.json`, harness in `tools/bench/webapps/` |
| [research/attested-gaming-mode.md](research/attested-gaming-mode.md) | research: attested gaming mode and kernel anti-cheat on WolOS (builds on ADR 0025; not a decision) |

# 0002 - Rust port of IaraOS userland on the forked compositor
Status: accepted (amends 0001: the compositor stays the fork chosen there). The syscall line is superseded by 0016: the system is Linux, and the last `Host::is_freebsd` paths go as their slices are removed.
Context: IaraOS (C#/.NET + Rust) solved jobs, QoS, admin auth, login, theme contract, bundles, System API, fork/join. It failed by rebuilding everything, incl. its own compositor.
Decision:
- One Cargo workspace. `shared` (contracts: QoS ABI, jobs, admin wire, theme tokens, bundle Info.json, events, error codes, `sys`, `par`) <- `core` (pure logic) <- `api` (`wold`, `wol-admind`), `controller` (`wol`, `wol-login`, `wol-authcheck`), `proxy` (`wol-run`), `realtime` (compositor IPC, event bus, QoS boost, GPU env, pure compositor policies + C ABI for `plugins/wol-hypr`).
- IaraOS compositor not ported; only its pure, tested modules (anim, titlebar, placement, policy, session lock, glass, gpu select) kept as library.
- Names: `iara` -> `wol`; config `/usr/local/etc/wolos/`; env `WOL_*`; sockets `/var/run/wold.sock`, `wol-qos.sock`, `wol-admin.sock`.
- State is files + sysctl; no database.
- FreeBSD syscalls behind `cfg(target_os = "freebsd")` with fakes; dev and CI on Linux.
- Architecture test: `crates/shared/tests/architecture.rs` (cargo metadata).
Consequences: IaraOS invariants and tests carried over as Rust table tests. C# UI (IaraKit, React.Iara) not ported; shell UI is `crates/shell` (GTK).

Amended 2026-11-04 (QA-ADR-15, QA-ADR-17): the config root is `/etc/wolos/` (ADR 0016, e.g. `shared::jobs::JOBS_DIR` = `/etc/wolos/jobs`), not `/usr/local/etc/wolos/`. The sockets keep their names under `/var/run` (`shared::jobs::SOCKET`). No FreeBSD command is left in a privileged path: the login window's power argv are `systemctl` verbs (0003, amended the same day), and `crates/controller/tests/image_arch.rs` refuses `acpiconf` and `/sbin` programs.

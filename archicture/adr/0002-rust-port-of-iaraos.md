# 0002 - Rust port of IaraOS userland on Hyprland
Status: accepted (amends 0001: compositor stays Hyprland)
Context: IaraOS (C#/.NET + Rust) solved jobs, QoS, admin auth, login, theme contract, bundles, System API, fork/join. It failed by rebuilding everything, incl. its own compositor.
Decision:
- One Cargo workspace. `shared` (contracts: QoS ABI, jobs, admin wire, theme tokens, bundle Info.json, events, error codes, `sys`, `par`) <- `core` (pure logic) <- `api` (`wold`, `wol-admind`), `controller` (`wol`, `wol-login`, `wol-authcheck`), `proxy` (`wol-run`), `realtime` (Hyprland IPC, event bus, QoS boost, GPU env, pure compositor policies + C ABI for `plugins/wol-hypr`).
- IaraOS compositor not ported; only its pure, tested modules (anim, titlebar, placement, policy, session lock, glass, gpu select) kept as library.
- Names: `iara` -> `wol`; config `/usr/local/etc/wolos/`; env `WOL_*`; sockets `/var/run/wold.sock`, `wol-qos.sock`, `wol-admin.sock`.
- State is files + sysctl; no database.
- FreeBSD syscalls behind `cfg(target_os = "freebsd")` with fakes; dev and CI on Linux.
- Architecture test: `crates/shared/tests/architecture.rs` (cargo metadata).
Consequences: IaraOS invariants and tests carried over as Rust table tests. C# UI (IaraKit, React.Iara) not ported; shell UI is `crates/shell` (GTK).

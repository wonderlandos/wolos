# 0003 - login socket: power requests
Status: accepted (amends IaraOS ADR 0024)
Context: the login window runs as `_wollogin` and cannot power off or suspend. ADR 0024 defines only `user\n` + password on `/var/run/wol-login.sock`.
Decision:
- Commands on the same socket: `!verb\n` (+ body). `!` never starts an account name, so auth framing is unchanged.
- Verbs `sleep`, `restart`, `shutdown`. Reply `ok` before running; `denied` (2 s delay) otherwise.
- Allowed only from the `_wollogin` peer (LOCAL_PEERCRED, as auth) and only while the greeter shows (no session, no login in flight).
- wol-login (root) runs fixed argv: `acpiconf -s 3`, `shutdown -r now`, `shutdown -p now`.
- Wire in `shared::login`; argv and policy in `controller::login`.
Consequences: no new privileged process and no doas rule. Any process running as `_wollogin` can power off the machine at the login screen (same as a physical button).

Amended 2026-11-04 (QA-ADR-16, QA-FILES-1: the system is Linux since 0016, and `acpiconf` and `shutdown -p` do not exist in the image): the fixed argv are the session menu's `systemctl suspend`, `systemctl reboot` and `systemctl poweroff`, one list for both (`wol_core::admin::session_power_argv`, read by `controller::login::power_argv`). logind locks and saves state. The rest of this decision stands. `crates/controller/tests/image_arch.rs` fails when a power argv names a program the Arch image does not have.

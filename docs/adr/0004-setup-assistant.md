# 0004 - first-boot setup assistant (wol-setup)
Status: accepted (amends IaraOS ADR 0024; replaces the bsddialog user prompt in firstboot)
Context: first boot asked for the first account in bsddialog on the console. WolOS wants a macOS-style setup assistant in the same visual language as the login window, without giving its UI root.
Decision:
- wol-login: when no login user exists (uid >= 1000, shell in /etc/shells), the greeter runs `wol-greeter-session setup` -> Hyprland `setup.conf` -> `wol-setup` (as `_wollogin`).
- Steps: Welcome, Language & Region, Keyboard, Wi-Fi (skippable), Create Your Account, Appearance, Done.
- Socket requests (ADR 0003 framing), only from `_wollogin`, only while no login user exists and the greeter shows:
  - `!setup-create-user\n` name, full name, hint, locale, keymap, appearance (`light|dark|auto`), one per line, then the password (rest). Rules: `wol_core::settings::users::validate_account`.
  - `!setup-wifi\n` ssid `\n` password (empty = open). Same steps as wol-admind Wi-Fi join (`wol_core::admin::plan`).
- wol-login runs `pw useradd -n <name> -c <full> -m -s /bin/sh -G wheel,video,operator -h 0` (password on stdin), writes `~/.login_conf` (lang), `~/.config/wol/setup.json` (locale, keymap, appearance), `/var/db/wolos/hints/<name>`, and `/usr/local/etc/wolos/appearance` unless `auto`; then starts the new user's session directly (no second login).
Consequences: account creation is reachable from `_wollogin` only on a machine with no login user. Applying `setup.json` keymap/auto appearance in the session is the session's job.

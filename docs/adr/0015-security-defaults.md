# 0015 - secure by default, honest about the rest
Status: accepted
Context: a desktop is safe only if it is safe without settings: nothing listening, updates found without asking, and privacy switches that do what they say.
Decision:
- On from the first boot: pf (inbound blocked except replies and local discovery), no remote login, system log off the network, kernel hardening with one reason per line (`wol_core::security::HARDENING`). `wol security` reports each item from the running system.
- Updates are found daily (cron, at most every 20 h) and only downloaded; installing is asked for and always starts with a restore point. freebsd-update never touches a kernel that is not GENERIC.
- App permissions say how they are enforced. Screen copy, cursor position, input capture and plugins are refused by the compositor (Hyprland permissions, first match wins). Camera, microphone, location and files outside home are advisory: FreeBSD 15.1 cannot stop an app that ignores them, and Settings says so.
- Encrypted homes are per-account ZFS datasets opened by wol-login with the login password. Off by default until proven on a real install.
Consequences: no per-app sandbox yet (no Capsicum or jails for apps). Root daemons stay small: fixed argv lists, allow-listed actions, peer uid from the kernel. Raising an advisory permission to enforced needs a device-level mechanism first (e.g. per-app camera access through a broker).

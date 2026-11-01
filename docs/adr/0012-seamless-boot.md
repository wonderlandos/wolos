# 0012 - one picture from power-on to the desktop
Status: accepted (amends ADR 0003 and ADR 0004)
Context: the first real-GPU boot showed rc text, a greeter hop and black gaps between compositors. A finished product shows its logo, then the login window or the desktop, and nothing else.
Decision:
- Screen sequence: loader menu -> WolOS kernel splash (`/boot/images/wolos.png`, `boot_mute`) -> black -> first WolOS surface fades in. `rc.d/wolos_quiet` (before `dumpon`) turns a video console black on black; serial consoles stay readable. Every compositor starts black (`misc:background_color`, `cursor:invisible` until the first window), and every first surface fades in from black and out to black (`kit::reveal`).
- One login account, no auto-login: wol-login starts that session directly with `WOL_SESSION_LOCKED=1`; `wol-session` runs Hyprland with `--locked-cmd "wol-lock --startup"`. No greeter hop. Several accounts: the login window, which fades to black before wol-login starts the session (2 s; 10 min on the Setup Done page).
- Nothing of the session is visible before authentication: the Hyprland fork (patch 0005) marks a lock started at boot or restored after a crash as a takeover: workspaces are hidden from its first frame, so only black shows until the lock screen draws.
- A lock crash never unlocks: `misc:allow_session_lock_restore`; `wol-lock` is a supervisor (one per session, `flock` on `$XDG_RUNTIME_DIR/wol-lock.lock`) that respawns the lock UI until it prints `unlocked`, and stops only when the compositor's Wayland socket is gone. A Hyprland crash restarts it locked (`WOLOS_CRASH_LOCK_CMD`). A locked start that dies within seconds falls back to the login window, never to an open session.
- VT switching is off while a WolOS session or greeter owns the screen (`VT_LOCKSWITCH` on ttyv0): Ctrl+Alt+Fn cannot show a console.
- Between compositors wol-login writes black-on-black with no cursor to ttyv0; text appears only for a reason (console notice) and is readable then.
- The start-up lock unlocks the login keyring with the typed password (`UnlockWithMasterPassword`); nobody typed one at boot.
- Boot timeline: wol-login and the first surfaces log `wolos-boot: <ms since boot> ms <stage>` (`grep wolos-boot /var/log/messages`): wol-login start, compositor spawned/exited, login window / lock / setup / desktop shown. A gap shows up as a hop.
Consequences: one more process per session (the lock supervisor). Console debugging needs a serial console or `kern.console` with a serial first. The fork owns the takeover policy; its tests live with the patch.

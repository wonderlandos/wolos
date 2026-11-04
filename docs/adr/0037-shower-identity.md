# 0037 - the shower is the shell wold's session started, not any process running its file
Status: proposed. Builds on 0020 (identity from the kernel) and 0033 (greet first). Audit P6-1 (S6).

## Context
wold answers "who may show the account's asks and answer them" (`subscribe account`, `session.answer`,
`Need::Shower`) by the executable of the peer: `/usr/bin/wol-shell` (`portal::SHOWERS`). Any process
of the account can run that file. With `LD_PRELOAD=evil.so /usr/bin/wol-shell`, or by tracing its own
child before the exec, it passes the check, receives every `SessionAsk` of the account and answers
"Allow once" for a sheet such as `SystemUpdate`, which wold then runs as root with nobody at the
computer. The update is signed and newer, so integrity holds; consent does not.

A path is not an identity. Identity by inheritance is: the shower is the process wold's session
started, and it holds something only that process holds.

## Landed first (defence in depth, no new moving part)
`PeerIdentity::injected` (`crates/api/src/peer.rs`), checked in `caller()` for the executables in
`SHOWERS` only, refuses a peer whose pinned `/proc/<pid>`:
- has `LD_*`, `GTK_MODULES`, `GTK3_MODULES`, `GTK_PATH`, `GIO_EXTRA_MODULES`, `GDK_PIXBUF_MODULE_FILE`,
  `GST_PLUGIN_*`, `QT_PLUGIN_PATH` or `GSETTINGS_SCHEMA_DIR` in its environment (or an empty one);
- maps executable code from outside `/usr`, `/lib`, `/lib64` (a library of the account's, a memfd:
  this also catches a preload that wiped its own `environ`);
- has a tracer (`TracerPid`), which survives an exec;
- lives in another mount namespace than wold (a private `/usr` overlay);
- or cannot be read at all (unknown is refused).
Not done: a parent check. The compositor starts `exec-once` programs by a double fork, so the legitimate shell
is an orphan whose parent is init, exactly like one an attacker double-forks.
This raises the price; it is not the identity. It does not stop a process that gets code into the
real shell after it started (ptrace needs the shell to be a descendant under `ptrace_scope=1`; making
the shell non-dumpable would close that; it is not landed, see the note below). A user
whose profile exports `LD_LIBRARY_PATH` gets no notifications until the shell is started clean.

## Decision (proposed: the token)
1. `wol-login` (root; it starts the session) draws 256 random bits per session, sends them to wold
   (`session.token {uid, token}`, accepted from root only) and keeps the descriptor of a memfd that
   holds them, sealed.
2. The memfd is inherited down to `wol-session` (clear close-on-exec across the setuid/exec only for
   this chain). `wol-session` forks the shell itself, before it execs `start-hyprland`, passes it the
   memfd (close-on-exec cleared in that child only) and sets close-on-exec in its own image, so the
   compositor and every app started under it never hold it. The shell waits for the compositor
   socket (it already retries), and wol-session restarts it if it dies.
3. The shell presents the token as the argument of `subscribe account` and of `session.answer`
   (never in an environment or argv); wold compares in constant time, per uid, and forgets it
   when the session ends. A mismatch is refused like a wrong executable. The path check stays too.
4. The shell sets `PR_SET_DUMPABLE=0` first thing, so no process of the account can ptrace it or
   read its memory or descriptors (`pidfd_getfd`). Not landed (no `prctl` in `crates/shell`, checked
   2026-11-04, QA-ADR-11): it is one call, but it also stops `systemd-coredump` from keeping the
   shell's stack trace, the crash evidence QA-CRASH-4 restored (`coredump.conf.d/wolos.conf`). Decide it
   together with the token, not before.

## Options not taken
- A systemd user unit and a cgroup check: the account owns `~/.config/systemd/user`, whose units
  override the vendor's; it proves nothing against the account.
- A different uid for the shell: it is a GTK client of the account's compositor and files; too big.
- `ptrace_scope=2` or 3: a lead call (breaks debuggers and Wine tools), and not sufficient alone.

## Consequences if accepted
- The compositor `exec-once` line for `wol-shell` goes; `wol-session` owns the shell's lifetime
  (restart, crash log). Tests (`desktop/overlay`, golden journeys) that start the shell from the compositor change.
- A second login of the same account (fast user switching) has its own token per session.
- Residual, accepted: code the real shell loads by its own plug-in path (themes, plugins) runs with
  its token. Plugins are `[[hypr]]`/Lua with allow-lists (`plugins.md`), not libraries.

# Assistants & Automation (ADR 0020)

Decision record: ADR 0020 (capabilities, approval, identity from the kernel).

One door for people, scripts and AI helpers: the `wol` command tree. Helpers never get more than the user could approve on screen.

## Who is a helper

| Caller | Name (policy, audit log) | Gate |
|---|---|---|
| `wol mcp` client (MCP over stdio, JSON-RPC 2.0) | its `clientInfo.name` | yes |
| script with `WOL_HELPER=<name>` | `script:<name>` | yes |
| script started with `wol script run <file>` or `wol script on <host> <file>` (sh, bash or Ruby, docs/modules/scripting.md) | `script:<file name>` (set by the runner) | yes; plus one `script run` / `script on` record per run |
| a person running `wol` | `cli` | no (admin verbs ask for the password on stdin) |
| an assistant's own shell (Claude Code's Bash tool running `wol ...`) | `script:claude-code` (`CLAUDECODE=1`, set by Claude Code), or `script:<name>` when `WOL_HELPER=<name>` is set | yes (advisory: a process can unset its own environment) |

Helper verbs: `catalog::HELPER_VERBS` (`wol api --json` lists access and capability). Anything else stays for people: under a helper name (`WOL_HELPER`, or an assistant's shell) it is refused and logged, except `help`, `api` and `mcp`.

## Gate (`controller::approve::decide`, same for MCP and scripts)

| Step | Rule |
|---|---|
| policy | `$XDG_CONFIG_HOME/wolos/helpers.json`: off switch, deny per capability; malformed = off |
| read | runs |
| change (`files tidy` without a plan is a dry run = read) | asks on screen unless Always Allow for that helper + capability |
| administrator | asks for the password every time; runs in wol-shell through wol-admind, never in the helper |
| no session (no `XDG_RUNTIME_DIR` / no socket) | refused, "No WolOS session is showing" |

## Wire (`shared::shell`)

Requester -> `$XDG_RUNTIME_DIR/wol-shell.sock`: `approve {"helper","tool","request"}\n`, half-close, wait up to `APPROVAL_WAIT_SECS` (60).
Shell -> one line `{"answer": ...}`, then close:

| Answer | Meaning | Requester |
|---|---|---|
| `allow-once` | change allowed | runs it |
| `allow-always` | change allowed; shell saved the grant (journaled) | runs it |
| `done` | admin change ran after the password | reports success |
| `failed` + `message` | admin change allowed but failed | error |
| `deny` | user said no | nothing changes |
| `timed-out` | no answer within `APPROVAL_TIMEOUT_SECS` (50, from arrival; MCP clients time out at 60) | nothing changes |
| `busy` | 1 on screen + 4 waiting already | nothing changes |
| empty / other | old shell or a wrong answer for the request | refused; an admin request never runs in the helper |

## Approval sheet (`crates/shell/src/ui/approval.rs`)

- Kit AuthSheet on one glass panel: "“<helper>” wants to <what>." in plain words (`catalog::describe`).
- Change: Deny, Always Allow, Allow Once; focus starts on Deny (a stray Enter or Space never says yes).
- Admin: password field, Deny, Allow; a wrong password asks again.
- Esc or closing = Deny. Queue: one on screen, 4 waiting, more get `busy`.

## `wol mcp` server limits

One server per client process, stdio, one JSON-RPC message per line. A client starts with `initialize` (its `clientInfo.name` is fixed for the session; a second `initialize` is refused, and listing or calling a tool before it answers -32002). A JSON array is a batch and gets one array of answers. A line over 1 MiB is dropped unread and answered as too large; a tool answers at most 256 KiB of text. The server ends when its client has gone, and a sheet it was waiting on is dropped, not approved. It does not serve root: run as your own account. Always Allow is bound to the client's name and its program, with release numbers in the program's path read as `*` (`.../claude/versions/2.1.3` is `.../claude/versions/*`), so a self-update keeps the grant.

## Errors and JSON, today (audit8 standards #11)

`wol` has no shared JSON error envelope yet; this table is what each surface does now, so a script
or an assistant knows which key to read. A change to one row changes this table in the same commit.

| Surface | Where | Failure shape | Success shape |
|---|---|---|---|
| `wol <verb> [--json]` | `crates/controller/src/run.rs` (`fail`) | no JSON: one line `wol: <message>` on stderr and the exit status of the code (`shared::errors::ErrorCode::exit`: 0 ok, 1 failed, 64 usage, 65 invalid, 66 not found, 69 unavailable or unsupported, 77 denied) | the verb's own object or array on stdout |
| `wol plugin run` | `crates/controller/src/plugin.rs` | the object so far plus `"error": "<sentence>"` on stdout, exit 65 | `plugin`, `kind`, `entry`, `refresh`, then `node`/`popover` or `hits` |
| `wol plugin check` | same | `"valid": false` and `"problems": [..]` (one sentence each) | `"valid": true` |
| `wol audio ...`, `wol sessions restore`, `wol modules remove` | `audio_doctor.rs`, `sessions.rs`, `modules.rs` | a `"failed"` key (count or list) inside the normal object | the same object with `"failed"` empty or 0 |
| MCP tool call (`wol mcp`) | `crates/controller/src/mcp.rs` | `{"content":[{"type":"text","text":..}],"isError":true}`; a protocol error is JSON-RPC `{"error":{"code","message"}}` | `isError: false`, the verb's JSON in `structuredContent` (an array as `{"items":[..]}`), or `{"status":"done","request":..}` after an approval |
| wold control socket | `shared::jobs::Reply` (`crates/shared/src/jobs.rs`), `crates/api/src/control.rs` | `{"ok":false,"error":"<sentence>"}` | `{"ok":true,"error":null}` |
| libwol service answers | `shared::portal::Answer` (`crates/shared/src/portal.rs`) | `{"ok":false,"code":<Code>,"message":..}` | `{"ok":true,"id"?,..}` |
| `wol-login` greeter lines | `crates/controller/src/bin/wol-login.rs` | the word `denied` (wrong password) or `error` as the reply line | the word `ok` |

The keys `kind` (widget and plugin entries), `status` (the MCP approval result) and `error` (plugin
run, control socket) name different things; do not read one as the other. The plan is one envelope
`{"ok", "code", "message"}` for the `--json` output of `wol`, with the exit status kept.

## Desktop toggles (`wol toggle <word> [on|off]`, `wol_core::toggles`)

What a toggle is: one switch the menu bar, Control Center tiles, hotkeys and helpers share. Why one
verb: a toggle can touch the display, idle and notifications at once, which no single capability
covers. How: `wol toggle <word>` flips it, `wol toggle <word> on|off` sets it, `wol toggle` (or
`wol toggle list [--json]`) lists every toggle with its state. Helpers may only list them
(`toggle list`, Read); flipping stays for people, hotkeys and tiles. State is one flag file per toggle
under `$XDG_STATE_HOME/wolos/toggles/org.wolos.<id>` (Do Not Disturb: the `wol/dnd` file in the
config home, which the notification code and `wol focus` read).

| Word (aliases) | Toggle | What it flips |
|---|---|---|
| `nightlight` (`night-light`, `nightshift`, `night-shift`) | Night Light | warmer colours: `hyprctl hyprsunset temperature` at `NIGHT_LIGHT_KELVIN`, off = `identity`. hyprsunset starts at login (warm when Night Light was left on); not running, on starts it (`hyprctl dispatch exec hyprsunset -t 4500`); not installed, on stops with a sentence saying so; off always succeeds |
| `stay-awake` (`idle`, `awake`, `caffeine`) | Stay Awake | the screen does not dim, lock or sleep by itself: a `systemd-inhibit --what=idle` held by the user unit `wol-stay-awake.service`, which off stops |
| `silence` (`dnd`, `focus`) | Do Not Disturb | banners wait in the Notification Center: only the flag file changes |

## Invariants

- One audit record per helper call (`$XDG_STATE_HOME/wolos/actions.jsonl`), written by the requester; the shell writes one only for an admin change it ran after the requester left. Passwords are never logged.
- Always Allow is per helper and capability. An MCP helper is its declared name plus the program that started `wol mcp`, read from the kernel through a pinned `/proc` directory (`Assistant (/usr/bin/assistant)`), so another program that copies the name is asked again (audit C10-6). Changes that leave something behind (`webapp add|remove`, `modules enable`, `sessions save|restore`, a `doctor fix` of the person's own services) are asked every time and offer no Always Allow (C10-1); `wol helpers deny <cap>` and `wol helpers forget <helper>` take it back; it is a journaled `helpers` change a helper cannot undo.
- The gate does not care what language a script is written in: it sees `WOL_HELPER`, so a Ruby script and a shell script meet the same policy, sheets and log (tests `crates/controller/tests/script.rs`). Over ssh (`wol script on`, or any ssh session) there is no desktop showing, so changes are refused with "No WolOS session is showing"; reads run.
- Helper names are what the helper says it is and helpers run as the user: the gate keeps well-behaved helpers inside what the user approved; it is not a sandbox.
- Tests: `crates/controller/tests/{mcp,helpers,approval_socket}.rs` (fake shell on a real socket), goldens `approval`, `approval-admin`.

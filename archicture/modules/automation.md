# Assistants & Automation

One door for people, scripts and AI helpers: the `wol` command tree. Helpers never get more than the user could approve on screen.

## Who is a helper

| Caller | Name (policy, audit log) | Gate |
|---|---|---|
| `wol mcp` client (MCP over stdio, JSON-RPC 2.0) | its `clientInfo.name` | yes |
| script with `WOL_HELPER=<name>` | `script:<name>` | yes |
| a person running `wol` | `cli` | no (admin verbs ask for the password on stdin) |

Helper verbs: `catalog::HELPER_VERBS` (`wol api --json` lists access and capability). Anything else stays for people.

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

## Invariants

- One audit record per helper call (`$XDG_STATE_HOME/wolos/actions.jsonl`), written by the requester; the shell writes one only for an admin change it ran after the requester left. Passwords are never logged.
- Always Allow is per helper name and capability; `wol helpers deny <cap>` and `wol helpers forget <helper>` take it back; it is a journaled `helpers` change a helper cannot undo.
- Helper names are what the helper says it is and helpers run as the user: the gate keeps well-behaved helpers inside what the user approved; it is not a sandbox.
- Tests: `crates/controller/tests/{mcp,helpers,approval_socket}.rs` (fake shell on a real socket), goldens `approval`, `approval-admin`.

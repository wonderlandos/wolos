# Atlas Drive (ADR 0063)

Decision record: ADR 0063 (accepted). Crate: `crates/atlas-drive` (binary `atlas-drive`, library `atlas_drive`). Numbers: [../bench/atlas-drive.md](../bench/atlas-drive.md). Design of record: `.claude/v012/atlas/automation-design.md`.

## What it is, and why

Atlas Drive drives Atlas the way Playwright drives Chromium, but its output is made for a model: a page is a few hundred tokens instead of tens of thousands, an action answers with what changed, many verbs go in one request, and the page's own text is treated as data. It is also the shared piece under the wolvm journeys, the Atlas lane's benchmark harness and the hub tests: one daemon, one socket, one frozen subset of the DevTools Protocol.

It speaks CDP to the browser over `--remote-debugging-pipe` (the browser's file descriptors 3 and 4). **There is no debugging port**: the crate refuses `--remote-debugging-port`, `-address` and `--remote-allow-origins` (`browser::forbid_debug_port`), and the product launcher never passes one.

## How it runs

**Installed by default.** The image ships `/usr/bin/atlas-drive` (in the `wolos-system` package: `tools/build-rust.sh`, the same release build as the other binaries; `crates/controller/tests/image_build_rust.rs` fails when it is missing). There is no systemd unit and no socket activation: the daemon is per user and on demand, started by the first `atlas-drive <verb>` (or `atlas-drive serve --launch` to warm the browser up front), and it creates `$XDG_RUNTIME_DIR/atlas` itself or uses the one `atlas-bridge.socket` made (both 0700), and refuses to serve from it when it is a symlink, another account's or open to group or others (`server::private_dir`). Registering it as an MCP server or a skill for the bundled helpers by default is not done yet (after the ISO); `atlas-drive mcp` works when a helper is pointed at it.

```
atlas-drive serve [--browser PATH] [--headed] [--profile DIR] [--no-sandbox] [--disable-gpu] [--launch]   the daemon
atlas-drive <verb> [args]      any verb as a command (the first one starts the daemon if none runs)
atlas-drive mcp                MCP server on stdio (tools drive_<verb>, page_<webmcp tool>)
atlas-drive mcp-manifest       the tool manifest as JSON
atlas-drive --help-agent       the one-page manual for a model's context (60 lines at most)
atlas-drive permissions list | allow <exe> [--scopes read,act] | revoke <exe> | auto on [--for 2h] [--site S] | auto off
atlas-drive stop
```

The first command starts the daemon when none runs (detached, in its own process group; `ATLAS_DRIVE_SERVE_ARGS` passes it options such as `--browser PATH --no-sandbox`; `--no-spawn` turns that off). The browser is `/opt/atlas/atlas` unless `--browser` says otherwise.

The socket is `$XDG_RUNTIME_DIR/atlas/drive.sock` (directory 0700, socket 0600, a live socket is never replaced). Requests are one JSON object per line; the same socket takes raw CDP.

```
{"id":1,"method":"Drive.click","params":{"target":"e12","then":"diff"}}       a verb
{"id":2,"method":"Page.navigate","params":{"url":"https://example.org"}}      raw CDP, the client's own ids
```

A verb answers `{"id":1,"result":{...,"tokens":41}}` or `{"id":1,"error":{"code":-32000,"message":"unknown ref e99; run snapshot (or find) first; refs are numbered per page","data":{"code":"stale_ref"}}}`. `batch` also streams `{"id":N,"event":{"event":"step","step":{...}}}` lines before its answer. A connection that sends raw CDP gets the browser's replies with its own ids; `cdp_events` streams matching events.

The browser starts headless (`--headless=new`) with a temporary profile unless `launch` says otherwise; headed needs a display. Threads, no async runtime: the daemon at rest has no timer and does not wake (measured, bench).

## The surface for a model

**The loop.** `snapshot` → act on a ref → read the short result → `snapshot --diff` (or put `then: diff` on the action).

```
@ http://127.0.0.1:8091/app.html "Profile settings"
[e1] banner
 [e2] navigation "Main"
  [e3] link "Home" -> /app.html
[e7] main
 [e8] h1 "Profile"
 [e9] textbox "Email" value="" required
 [e10] combobox "Plan" value="Free" collapsed options=[Free*|Pro]
 [e12] button "Save" disabled
```

- One line per interactive or textual node; indentation is landmark depth; refs only where a verb can act; text runs merged; screen-reader text kept; hidden nodes are not in the accessibility tree and so not here.
- `--interactive-only`, `--depth N`, `--region eN`, `--viewport-only`, `--full` (do not collapse a navigation or footer already shown on an earlier page of this tab), `--diff` (`+` new, `~` changed, `- e5` gone, same refs), `--max-tokens N` (priority: in the viewport or focused, then the focus region, then the rest, in document order within a class; ends with `… K more nodes`). Same page, same options, same bytes (tested).
- Refs are numbered per page and stay valid until the document navigates; the next verb after a navigation says so in one line.
- Locators work wherever a ref does and wait for the element (default 3 s): `"Save"`, `role=button name~=Save`, `name=Save`, `label~=Email`, `value~=x`, `css=.row`, `testid=save`, `nth=1`. More than one match lists the candidate refs.
- Every result has `tokens` (bytes / 4). `budget: N` (`--budget N`) on any call cuts its longest text at a line boundary and says `… K more lines cut to fit`; a batch keeps whole steps.
- `quiet: true` (`--quiet`, optionally `--fields a,b`) on any call returns only the answer: the `value` or `text` of a verb (or the fields named), the answers of a batch's steps (one answer bare), and the failure if there is one. A script that already knows its locators pays for nothing else.
- **Page text is data.** A snapshot withholds text that only a trick makes invisible (white on white, a 1 px font, zero opacity) or that reads like an instruction, and says so in a warning line, `N hidden or instruction-like text block(s) withheld`. `traps` lists everything it found, by kind, apart from the content. Screen-reader-only text stays in the snapshot unless it reads like an instruction. The scan is heuristic; it narrows the attack, it does not close it.
- `batch` is sequential by default (stop at the first failure). `after: [ids]` or `parallel: true` makes fork/join steps that run at the same time, stream as they finish and join before a dependent step; `continue: true` runs on after a failure.

## Permissions

No prompt per verb. The first connection of an agent (its executable path, from `SO_PEERCRED` and a pinned `/proc/<pid>`) is checked against `grants.json`:

| Scope | Verbs |
|---|---|
| `read` | snapshot, find, screenshot, wait, links, forms, extract, traps, outline, css, listeners, frameworks, memory, tabs, status, codegen |
| `act` | goto, click, fill, type, press, scroll, select, check, hover, tab, dialog, eval, emulate, cursor, js, webmcp, launch, close, batch, cdp |
| `network` | network, route, cdp_events (events carry request headers) |
| `storage` | cookies, storage |
| `files` | upload, pdf, trace, record, downloads |

A scope that was not granted is refused with one line that names the command to allow it. Raw CDP (the socket's CDP methods and the `cdp` verb) needs every scope. Any call that writes where the caller says (`out`, `dir`) also needs `files`, so a read-only agent cannot overwrite a file. `eval`, `js` and `webmcp call` run page script as `act`: they can read whatever the page's own script can, including its localStorage and cookies that are not HttpOnly. **AUTO** (`permissions auto on`) stops the questions, globally or for named sites, for a time or until `auto off`; `permissions list` shows it and `revoke` takes an agent's grant away. The person at a terminal is not an agent: the CLI on a tty is allowed; the same CLI started by a program without a terminal is that program. The consent sheet is the shell's: `ATLAS_DRIVE_CONSENT_CMD` is run once for a new agent (`cmd <exe>`, exit 0 grants every scope). While an agent is attached `$XDG_RUNTIME_DIR/atlas/drive.attached` lists it, for the shell's indicator. `ATLAS_DRIVE_CI=1` (forge, wolvm guest) skips consent.

## Status of every verb

Done = built and run against a real Atlas (headless, the component build of 2026-11-05 on the forge) by this record, with the unit and protocol tests behind it. What is not "done" says so in the cell.

| Verb | Status |
|---|---|
| launch, close, status, tabs, tab | done (new, select, close); headed: not run |
| goto, back, forward, reload | done (`wait` load, domcontentloaded, commit, networkidle; a cancelled navigation, `net::ERR_ABORTED`, is retried once) |
| snapshot, find | done: every option; the tree comes from the in-page mirror (`src/mirror.rs`, Drive's isolated world; `--ax` reads the browser's accessibility tree instead); `--viewport-only` and `--max-tokens` use the boxes the walk measures; `--hrefs` shows link addresses, `--full` turns off folding; a bare text locator finds the text run when no control matches |
| click, hover, fill, type, press, scroll, select, check, upload | done: by ref and by locator, double click, `type --slow`, chords (`Control+a`), `upload`; `click --dom` and `--button right` not run |
| wait | done: selector, text, gone, url, network_idle, stable; load and ref not run |
| screenshot, pdf | done: PNG, JPEG, full page, element, clip, inline base64; `--scale` not run; `pdf` is headless only |
| eval | done: errors carry the top frames; `--map` resolves them through a real source map (`src/app.ts:2:3`) |
| cookies, storage | done: cookies list and set (`clear` not run), storage dump, get, set, keys, clear |
| emulate | done: viewport, devices, colour scheme, locale (`navigator.language` follows), timezone, geolocation, offline; `--grant`, `--user-agent`, `--motion` not run |
| trace, memory, downloads, dialog | done (trace writes a 2.8 MB Chrome trace; downloads waits for the file; dialog shows and answers) |
| batch | done: sequential, fork/join, streamed steps, stop at the first failure, `continue` |
| cdp, cdp_events | done |
| links, forms, extract, traps, outline, frameworks | done (traps: eight kinds caught on a fixture page; `frameworks` found none on the test pages, so its signatures are not exercised) |
| css, listeners | done: computed, matched rules, variables, `--why <property>`; listeners of a ref |
| network, route | list, failures, console, body, replay (against a server that answered 501), har done; `ws` not run (no WebSocket server on the forge); route mock and delay done, block not run |
| js | done: scripts, search, source with pretty-print, break on exception, paused, resume; break on an event or an XHR not run |
| webmcp | done: list and call on a declarative form (`toolname`), watch; imperative tools through `navigator.modelContext` not run (the build has no such API) |
| cursor, record | done: the pointer is drawn into screenshots; record writes numbered JPEG frames and an index (video: `ffmpeg`) |
| codegen | done: the action verbs of the session as a batch array |
| permissions | the CLI is done (allow, revoke, auto, list, per-scope refusal, files for `--out`); the verb is read-only |
| mcp, mcp-manifest, --help-agent | done: initialize, tools/list, tools/call over stdio; 50 tools; 31 lines of help |

## Parity with Playwright

| Playwright | Atlas Drive |
|---|---|
| launch headless / headed, contexts | headless (done), headed (flag and the engine's automation bar, not run); one browser, many tabs; no incognito contexts yet (pending) |
| locators by role, text, label, placeholder, testid, css, nth | done (`placeholder` is matched through the accessible name; no XPath on purpose) |
| auto-wait | locators wait; a click does not wait for the element to stop moving or for a hit-target check (pending: `--check`) |
| actions: click, dblclick (`count: 2`), right click (`button`), hover, fill, type, press, select, check, upload | done except drag and drop (pending) |
| frames, iframes | pending: the main frame only (out-of-process frames need their own session) |
| dialogs | done (an open dialog fails the next verb with its text and the fix) |
| downloads | done (`downloads`, `--wait`) |
| geolocation, permissions, locale, timezone, colour scheme, reduced motion, offline | done except permissions and reduced motion (not run) |
| network interception, mocking, HAR | `route` (mock, block, delay) and `network har`; HAR without timings |
| tracing, viewer | `trace` writes a Chrome/Perfetto trace; open it in ui.perfetto.dev (no Playwright-style viewer) |
| codegen | `codegen` replays the verbs of a session; it is not an interactive recorder |
| screenshots, PDF, video | screenshot, pdf; frames from `record`; no mp4/webm muxer |
| accessibility snapshot | `snapshot` (also refs and diff, which `ariaSnapshot` does not have) |
| cross-browser | Atlas only (an adapter for the other page engine is later) |

## Frozen CDP subset

`src/subset.rs`: 79 methods and 30 events. `tests/subset.rs` fails when a source names a method that is not listed and when a listed row is not used, so a diff of the list is a diff of what Atlas must keep. Verbs are the stable surface; raw CDP is for the person who needs more.

## Not built (and where it goes)

Attach to a running Atlas (the launcher holds the pipe: ADR 0063 "Not decided"), the consent sheet and indicator widget, the Crystal shard, wolvm `drive.*` steps, bindings generated from `browser_protocol.pdl`, an incremental (dirty-subtree) mirror in the page, frames, drag and drop, incognito contexts, adapter for the page engine.

## Where things are

`src/agent.rs` (snapshot, refs, diff, truncation, folding; pure), `mirror.rs` (the in-page accessibility mirror: rows to AX-shaped nodes, the isolated world, refs to backend ids), `verbs.rs` (verbs), `investigate.rs` (investigation verbs), `pagejs.rs` (scripts that run in the page), `browser.rs` (pipe, multiplexing, tabs), `page.rs` (tab state from events), `server.rs` (socket, identity, budget), `perms.rs`, `spec.rs` (verb table, help, MCP manifest), `subset.rs`, `cli.rs`, `mcp.rs`, `batch.rs`, `route.rs`, `record.rs`, `cursor.rs`, `sourcemap.rs`. Tests: unit tests beside the code, `tests/protocol.rs` (the socket against a test double of the pipe), `tests/subset.rs`, `tests/fuzz_seeds.rs`. Benchmark: `tools/bench/atlas-drive/` and `crates/atlas-drive/examples/bench.rs`.

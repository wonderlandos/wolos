# atlas: the bridge between Atlas and WolOS (ADR 0043 D4)

The program `atlas-bridge` lets the Atlas browser Share a file, Open With another app and Move to Trash the way every WolOS app does: the desktop shows the share sheet, the app chooser or moves the item, and the browser only learns what the person did.

**Concept.** Atlas runs no WolOS code. Its host component (`atlas_host`, in the browser) is a thin client of the **Atlas Host Protocol**, a versioned line-delimited JSON wire spoken over `$XDG_RUNTIME_DIR/atlas/bridge.sock`. On the other end is `atlas-bridge` (`crates/atlas`, GPL-2.0-or-later), the one program of the Atlas stack that asks wold for anything. It checks that the peer really is the browser, then hands each verb to wold as the portal call libwol 1.15 makes (`share.file`, `open.with`, `file.trash`). The wire types live in `crates/shared/src/atlas.rs` (BSD-2-Clause) and the fixtures in `crates/shared/tests/fixtures/atlas/`, shared byte for byte with the browser's `components/atlas_host/testdata/`.

**Why.** The browser and WolOS are separate programs with separate licences and release trains. A documented protocol keeps them apart (aggregation, not linking): no WolOS source is compiled into the browser, an engine upgrade touches only the host component's hook sites, and either side can change as long as the version and the fixtures hold. wold names an app by its executable, so the person's grants are given to one small, checked program instead of to a 300 MB browser that loads web content.

**How.**

```text
browser (atlas_host)                      atlas-bridge                       wold -> desktop
  connect bridge.sock      ─────────▶  SO_PEERCRED + pinned /proc/<pid>
                           ◀─────────  hello <nonce>   (the bridge speaks first, ADR 0033)
  hello <nonce>            ─────────▶  every byte written by the connecting pid (SCM_CREDENTIALS)
  {"op":"hello",...}       ─────────▶  exe == /opt/atlas/atlas, pid matches
                           ◀─────────  {"reply":"hello","version":2}
  {"op":"share","id":1,..} ─────────▶  file there, a file      ─────────▶  share.file  (sheet on screen)
  {"op":"trash","id":2,..} ─────────▶  (own thread)            ─────────▶  file.trash
                           ◀─────────  {"reply":"ok","id":2,"outcome":"trashed"}
                           ◀─────────  {"reply":"ok","id":1,"outcome":"copied"}   (the person chose)
```

## The protocol, version 2

Version 2 (QA-A9-SEC-1) adds the greeting. As soon as the bridge accepts a connection it writes `hello <nonce>` (128 random bits, hex) and reads nothing else until the browser has written the same line back (`shared::jobs::answer_hello`). The bridge turns on SO_PASSCRED before it sends the greeting and requires every byte of the echo to carry the pid the kernel recorded at connect (`shared::jobs::greet_by`), so a hello written before the bridge accepted, or answered by another process that holds a copy of the descriptor, is refused as `not-browser`. The greeting and the hello line share the one 5-second deadline.

One JSON object per line, UTF-8, at most 32 KiB including the newline. Field order is canonical (the fixtures are compared byte for byte); a line that is not an object, has an unknown field or an unknown `op` is refused.

The browser speaks first:

| Browser sends | Bridge answers |
|---|---|
| `{"op":"hello","version":1,"pid":4242}` | `{"reply":"hello","version":1}` |

`version` is the newest version the browser speaks; the bridge answers the version both speak (the smaller of the two), and a browser that cannot speak it closes. `pid` is the browser's own pid: it must be the pid the kernel reports for the socket. The hello must be complete within 5 seconds of the connection being accepted (a peer that sends a byte now and then does not extend it). At most 16 connections are open at once; the 17th gets `busy` (id 0) and is closed. If the bridge reads the end of the stream before any hello (it chose to leave for idleness in the very instant the connection arrived), the browser connects again: the socket starts a new bridge. Anything else first, a wrong pid, or a peer that is not the browser gets `not-browser` and the connection closes.

Then any number of verbs, each with an `id` from 1 to 4294967295 that the browser does not reuse while it waits:

| Request | Outcomes in `ok` |
|---|---|
| `{"op":"share","id":1,"path":"/home/ana/Downloads/report.pdf"}` | `copied`, `saved`, `opened`, `cancelled` |
| `{"op":"open_with","id":2,"path":"/home/ana/Downloads/report.pdf"}` | `opened`, `cancelled` |
| `{"op":"trash","id":3,"path":"/home/ana/Downloads/report.pdf"}` | `trashed` |

`path` is a full path as libwol 1.15 takes it: from `/`, no empty, `.` or `..` part, no trailing `/`, no control characters, at most 4096 bytes. Share and Open With take a file; Trash takes a file or a folder.

Every verb gets exactly one reply with its id, **in the order the answers are ready**: a share sheet that waits for the person does not hold back a Trash. At most 4 verbs wait at once per connection.

| Reply | Meaning |
|---|---|
| `{"reply":"ok","id":1,"outcome":"copied"}` | The verb ran; the outcome is what the person did. |
| `{"reply":"error","id":1,"code":"…","text":"…"}` | The verb did not run. Show `text` as it is. |

Error codes, with the text the person reads (texts never name a path; the browser knows which file it asked about):

| `code` | When | `text` |
|---|---|---|
| `invalid-request` | Not a request of this version (bad JSON, unknown op, id 0, a path that is not a full path, an id still waiting, a second hello, a line over 32 KiB which also closes) | Atlas sent a request WolOS does not understand. |
| `version` | Reserved: a hello older than the bridge still speaks; closes | This Atlas and this WolOS do not match. Update both to share files and use the Trash. |
| `not-browser` | The peer is not the browser, said no hello, named another pid, or exec()ed since the hello; closes | Only Atlas can use this service. |
| `permission-denied` | The person has not allowed it (share, open with) | You have not allowed Atlas to share files or open them in other apps. You can allow it in System Settings, Privacy & Security. |
| `permission-denied` | The person has not allowed it (trash) | You have not allowed Atlas to move files to the Trash. You can allow it in System Settings, Privacy & Security. |
| `unavailable` | wold or the desktop cannot be reached, or did not answer | WolOS could not reach the desktop. Try again in a moment. |
| `unavailable` | Share on a WolOS older than libwol 1.15 | Sharing a file from Atlas needs a newer WolOS. |
| `unavailable` | Trash on a WolOS older than libwol 1.15 | Moving to the Trash from Atlas needs a newer WolOS. |
| `not-found` | The path is not there (or a link to nothing, for Share and Open With) | The file is not there any more. It may have been moved or deleted. |
| `busy` | Another sheet is on screen, or 4 verbs already wait | Something else is waiting for you to choose. Finish it, then try again. |
| `busy` | 16 connections are already open (id 0, closes) | Atlas has too many connections open to WolOS. Close some and try again. |
| `failed` | Share of a folder, or the desktop refused | This item cannot be shared. Only a file can. |
| `failed` | Open With of a folder, or no app opens the kind | No app on this computer opens this kind of file. |
| `failed` | The desktop refused the Trash (the home folder, a folder holding it, the Trash itself) | This item cannot be moved to the Trash. |

Errors that belong to no request carry id 0. There is no cancel request in version 1: cancelling is the person's, in the sheet, and arrives as `cancelled`. A reply the browser does not read within 5 seconds (or cannot be written) closes the connection, so a half-written line is never followed by another. A browser that closes its connection abandons what waits; a sheet already on screen stays until the person answers it.

### Older WolOS

A wold older than libwol 1.15 answers the three calls "not a known call". The bridge then answers Share and Trash `unavailable` (texts above) and runs Open With as `xdg-open <path>`: the file opens in its default app (`opened`) instead of a chooser, or `failed` if xdg-open fails. (xdg-open refuses a `--` argument; the path always starts with `/`, so it never reads as an option.)

### Fixtures

`crates/shared/tests/fixtures/atlas/`:

- `requests.valid.jsonl`, `replies.valid.jsonl`: each line decodes and encodes back to the same bytes.
- `requests.invalid.jsonl`, `replies.invalid.jsonl`: each line is refused.
- `sessions/*.txt`: transcripts. `> ` a line the browser sends, `< ` the line the bridge must answer next, `closed` the bridge closes. `@PID` is the browser's pid and `@DIR` a scratch directory. The other lines set up and check the fake wold side (`file`, `dir`, `wold <call> <answer>`, `wold down`, `opener ok|fail`, `asked <call>`, `opened <path>`) and a fake bridge ignores them.

`crates/atlas/tests/fixtures.rs` and `crates/atlas/tests/sessions.rs` play them on the Rust side; the browser's host tests read the same files.

## Identity and permissions

- The socket is 0600 in a 0700 directory (`atlas-bridge.socket`), so only the account's processes connect.
- At accept the bridge reads the peer's uid and pid (SO_PEERCRED), opens `/proc/<pid>` as a directory and reads the executable through that descriptor (STANDARDS 7.1 "Identity", audit C10-6): the peer must be this account's process running `/opt/atlas/atlas` (canonical path), it must answer the greeting itself (every byte of the echo is stamped with the writing process by the kernel), its hello must name that pid, and before each verb it must still run the executable it ran at accept.
- **What "the browser" means.** Any process of this account that runs the browser executable counts as the browser, renderers and other child processes included (they all run `/opt/atlas/atlas`). The bridge cannot tell the browser's main process from a compromised renderer; keeping web content out of the sockets of the account is the engine's sandbox, which this check relies on. What a stolen connection can do is limited to the three verbs on files that exist, and Share and Open With wait for the person to choose in the desktop's own sheet, and Move to Trash is only as safe as the person's grant `trash`.
- During an update of the package the running browser's executable reads as `/opt/atlas/atlas (deleted)` (`read_link` appends it); such a peer is refused as `not-browser` until the browser is restarted.
- The bridge's own listening socket is not inherited by the programs it starts (it is a close-on-exec copy of the descriptor systemd passes), so the opener the fallback runs, and the app that opener starts, cannot accept connections.
- wold sees the bridge, not the browser: the calls need the grants `share` (Share, Open With) and `trash` (Trash) for `/usr/lib/atlas-bridge`. System Settings, Privacy & Security gives them, written to `/etc/wolos/consent.json` like every grant; the `atlas-browser` package may ship them as defaults.
- The bridge looks at the path before asking (it is there; for Share and Open With it is a file) and never opens, reads or moves it: the desktop does that as the person.

## Running

| Piece | Where |
|---|---|
| Binary | `/usr/lib/atlas-bridge` (`cargo build -p atlas --bin atlas-bridge`) |
| Units | `desktop/overlay/usr/lib/systemd/user/atlas-bridge.{socket,service}` |
| Socket | `%t/atlas/bridge.sock` |

The socket starts the service at the first connection. Every wait in the bridge is a blocking read or accept, so it never wakes while the browser holds an idle connection; with no connection and nothing waiting for the person it exits after 30 seconds, and the socket starts it again next time. A development run binds its own socket: `atlas-bridge --socket /run/user/1000/atlas/bridge.sock`; `WOL_SOCKET` points it at another wold, as for libwol.

Budget: idle RSS at most 6 MB, no wakeups while idle. Measured on 2026-11-04 (release build, 910 KB binary): 2.3 MB RSS waiting for the first connection (`/proc/<pid>/status`, 2 threads, 0 context switches in 20 s, then the 30 s exit); 2.4 MB RSS with a browser connected and idle (4 threads, the harness's own reader included), and 0 voluntary context switches in the bridge's threads over 60 s, read from `/proc/self/task/*/status`. `MemoryMax=32M` in the unit ends a leak, never the session.

## Tests

- `cargo test -p atlas`: fixtures, every session against a real bridge and a fake wold on a socket, a waiting sheet that does not hold back a Trash, the 4-verb limit, another executable refused, a silent connection and a trickling one closed at the hello deadline, the 17th connection refused, the listener close-on-exec, idle exit only with nothing connected and nothing waiting.
- `cargo test -p shared`: the wire types' own tests (refusals keep the id, no text names a path).
- `fuzz/fuzz_targets/atlas_wire.rs`: request and reply decoders never panic and an accepted line re-encodes to itself.

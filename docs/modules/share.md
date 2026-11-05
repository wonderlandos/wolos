# Share: one sheet, the apps that can take the thing (ADR 0020, ADR 0059, E2E-SHARE, surface S04)

Share is one sheet that opens from everywhere a person can send something: an app built on libwol (`share.file`, `share.text`), Files ("Share…" in the right-click menu), Atlas (through the bridge, the Atlas bridge page) and the command `wol-share`. The sheet shows a tile for every app on the computer that can take the thing, then Open With…, Save a Copy… and Copy. It never lists an app that cannot take it, and it does not need a list kept by anyone: the desktop entries are the list.

**Why.** Before, Share was an alert with three fixed buttons. A person who wanted to mail a report or put a page into Notes had to copy it, switch app and paste. The apps already say what they take (`MimeType`, `x-scheme-handler/mailto`), so the system can offer them without the app changing (ADR 0059 rule 1, D4).

## What the sheet offers

| Tile | The entry | What the app gets |
|---|---|---|
| **Mail** | handles `x-scheme-handler/mailto` | started with a `mailto:` link: `subject`, `body` (text or link, at most 4000 bytes) and `attach` (the file's full path) |
| **Declared** | has `X-WolOS-Share=text;url;file` (the kinds it takes; add `share-file` to ask for long text in a file) | a file as a plain file argument; text as `--share-text TEXT`; a link as `--share-url URL` (an app that declared `text` but not `url` gets the address as `--share-text`); `--share-title TITLE` when there is a title; text above 4 KiB as `--share-file PATH` when the app declared `share-file` |
| **Opens** | `MimeType` holds the file's kind (any `text/*` kind matches `text/plain`; `type/*` matches), or `x-scheme-handler/<scheme>` holds the link's scheme, and its `Exec` has a file or link code (`%f %F %u %U`) | the file or the link, the way Open With starts it |

The first matching rule wins: an entry that takes mail links is Mail for everything. Where the entries come from: the same folders and the same filter as the app list, so an entry the person put under `~/.local/share/applications` is a target with the same trust as any app in the list: it can be started with the file, text or link the person is sharing, and nothing more. Entries the app list hides (`NoDisplay`, `Hidden`, `OnlyShowIn` for another desktop, a missing `TryExec`), terminal-only entries (`Terminal=true`) and the shell's own hidden list never show. At most 8 tiles show; the rest stays behind Open With….

A text that is one web address on its own (`http://` or `https://`, no spaces) is a link: Mail gets it in the body, the browser opens it, an app that declares `url` (or `text`) gets `--share-url`.

**Order.** What the person shared most often comes first (`share_targets::Usage`, counted in `$XDG_STATE_HOME/wolos/share-usage` or `~/.local/state/wolos/share-usage`, one `id<TAB>count` per line, written beside the file and renamed). At most 64 KiB of the file is read and at most 200 apps are kept, trimmed on every write; without a state folder or a home nothing is counted; ties go to Mail, then declared apps, then the rest, then by name.

**No apps.** The sheet says "No app here takes this. You can still copy it or save it." and keeps Cancel, Open With… (files), Save and Copy.

## Keys

Tab and the arrow keys move between the tiles and the buttons, Return picks the one with the keyboard ring, Escape cancels. Cancel has the focus when the sheet opens, so a stray Return sends nothing anywhere; the arrows and Tab reach the tiles. The window is modal; when it closes the focus goes back to the window the person came from.

## `wol-share`

```text
wol-share FILE        the sheet for a file (a relative path is made full)
wol-share -           the sheet for the text on standard input (at most 24 KiB)
wol-share https://…   the sheet for a link
```

Standard input is checked the way libwol checks shared text: at most 24 KiB (`MAX_MESSAGE`, `crates/shared/src/portal.rs`), valid UTF-8, line breaks as `\n` (a CRLF becomes `\n`, a tab one space) and no other control character; more than 24 KiB or other bytes is refused whole with a sentence, never cut, so nothing is shared half.

It prints what the person did, one word: `copied`, `saved`, `opened` or `cancelled` (the same words libwol and the Atlas bridge answer), and exits 0; a failure prints one friendly sentence on standard error and exits 1. Files' right-click "Share…" (`desktop/nautilus/wolos-share.c`, one local file only) starts `wol-share` with the path.

## For app authors

To be a target for text and links, add to your launcher entry `X-WolOS-Share=text;url;file` (leave out what you do not take) and read the arguments above. To be a target for files, the usual `MimeType` and an `Exec` with `%F` or `%U` is enough. Notes is the first app that declares `text;url;file` (E2E-NOTES).

## Code and tests

| Piece | Where |
|---|---|
| Discovery, ranking, usage, the command that starts the app | `crates/shell/src/share_targets.rs` (pure on entry text; `read_entries` and `find` read the disk and are for worker threads only) |
| The sheet and `wol-share` | `crates/shell/src/ui/share_sheet.rs`, `crates/shell/src/bin/wol-share.rs` |
| The libwol side | `crates/shell/src/app_session.rs` (`share_file`, `share`, `Session::deliver`) |
| Files item | `desktop/nautilus/wolos-share.c` |
| Goldens | `crates/shell/tests/golden/share-{file-4,file-6,file-none,text-2}-{light,dark}.png` |
| Journey | `e2e-share-file` (wolvm, forge) |

Unit tests: discovery on a fake applications folder of 10 entries (hidden, `Hidden`, terminal, another desktop and an `Exec` without a file code skipped), each kind of thing, ranking by usage, the 8-tile cap, `Exec` field codes, the `mailto:` link, the command per role, and in `app_session` that a chosen target answers `opened` and a target that does not start answers its sentence.

## The hand-off to the image

`tools/build-rust.sh` builds `wol-share` with the other shell programs (the binary loop) and compiles `desktop/nautilus/wolos-share.c` into `libwolos-share.so` next to `libwolos-analyze.so` in the Files extension folder; the `wolos-system` package ships whatever the script stages.

## Who can see what

- A mail link carries the subject and the body (at most 4000 bytes of text) in the arguments of the mail app, and so does `--share-text`; the process list shows arguments to every account on the computer for as long as that app runs. Text above 4 KiB is therefore passed in a file, `--share-file PATH`, when the app declared `share-file`: a new file, mode 0600, in `$XDG_RUNTIME_DIR/wolos/share-texts/` (mode 0700), removed when the app's process ends and swept after ten minutes if a single-instance app handed over and left earlier than it read it. The app must read the file before it returns from its start-up handler, or copy it. Without `share-file` (and for every mail body) the text stays in the arguments: a limit of this design, and the reason the sheet never sends a text without the person choosing the app.
- Control characters never reach a mail subject or body (line breaks stay in the body as CRLF).

## Limits

- Mail attaches with the `attach` parameter of the `mailto:` link. Whether the installed mail app honours `attach` is to be verified on the image (the journey checks that the compose window opens; the attachment is read in its screenshot).
- A target that opens a window on its own (a compose window, a new note) is "opened"; the sheet cannot know whether the person then sends it.
- Open With… shows the same tiles (up to 8 apps that open the kind, the ones the person picked before first, remembered in the `[Added Associations]` of `~/.config/mimeapps.list`); it is the stand-in `open_with` module until E2E-OPEN's replaces it.
- Several files at once, and folders, are not shared (libwol's `share.file` takes one file).

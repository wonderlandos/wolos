# Quick Look, org.wolos.quicklook (ADR 0059)

Space on a file in Files shows it in a small window; Space or Esc closes it; the arrow keys go to the next file. The program is `wol-quicklook` (`crates/shell/src/bin/wol-quicklook.rs`, window in `crates/shell/src/ui/quicklook.rs`, readers and the D-Bus service in `crates/shell/src/quicklook.rs`, sentences in `crates/shell/src/copy_quicklook.rs`). It is a system service, so it works for Files with no change to Files. The decision (system services first, stock apps unchanged) is in ADR 0059, still a draft in `.claude/v012/apps-e2e/`. The human interface guide promises it (`docs/modules/hig.md`).

## How Files reaches it

Files (Nautilus 45 and later) has no preview of its own. On Space it calls a D-Bus service that the old Sushi previewer used to provide. WolOS owns that name:

- Bus name `org.gnome.NautilusPreviewer`, object path `/org/gnome/NautilusPreviewer`, **interface `org.gnome.NautilusPreviewer2`** (the bus name has no 2, the interface has).
- `ShowFile(s uri, s windowHandle, b closeIfAlreadyShown, s activationToken)` and `Close()`.
- Properties `Visible` (b) and `ParentHandle` (s). Files reads `Visible` from its cached copy and, while it is true, sends the new file after every selection change, so the property must change before the next arrow key.
- Signals back to Files: `SelectionEvent(u direction)` (a `GtkDirectionType`: up 2, down 3, left 4, right 5; Files moves its own selection and calls `ShowFile` again), `NavigateTo(s uri)` (Return on a folder).

Confirmed on 2026-11-04 from the Nautilus source (`src/nautilus-previewer.c`, constants `PREVIEWER2_DBUS_IFACE`, `PREVIEWER_DBUS_NAME`, `PREVIEWER_DBUS_PATH`, the `(ssbs)` call in `real_call_show_file`), from Sushi's interface file (`data/org.gnome.NautilusPreviewer2.xml`, with the activation file `data/org.gnome.NautilusPreviewer.service.in`), and from the strings of the Nautilus 51.1 binary in the test lane image. `crates/shell/tests/quicklook_bus.rs` pins the signatures on a private bus.

Files pings the name when it starts (that is a D-Bus call with auto start), so the activation file `desktop/overlay/usr/share/dbus-1/services/org.gnome.NautilusPreviewer.service` starts `wol-quicklook` the first time Files opens. The previewer quits after ten minutes with nothing shown (`IDLE_EXIT_SECS`) and starts again on the next Space.

## What it shows

| Kind | Shown |
|---|---|
| Picture | the picture, scaled down to at most 2400 px on its longest side (a small one keeps its pixels), turned by its embedded orientation, in a rounded frame |
| Text and code | the first 256 KiB in a monospace view, with a note when the file is longer; no syntax colours |
| Audio | an icon card with play or pause, a position slider and the time; starts playing |
| Video | the video; starts playing |
| PDF | the first page, drawn by `pdftocairo` when it is installed |
| Folder | the name, the number of items, folders and files, and the size of the files directly inside |
| Anything else | an icon card with the name, kind, size and date and one sentence |

A file with an unknown name is looked into: if its first 4 KiB are text it shows as text. A thing that cannot be read, a picture that cannot be decoded, a PDF that cannot be drawn, media that cannot be played all end on the card with a sentence saying so (`copy::quicklook`).

## Limits, by design

- Every read is bounded: 256 KiB of text, 256 MiB of picture file and 200 million pixels, 20 000 folder entries, 8 seconds, 32 MiB of output and 1 GiB of memory for `pdftocairo`. The reads run off the GTK thread.
- Nothing is executed and nothing touches the network. Only `file://` addresses are read; another kind of address (a phone, a network share) gets the card with the name.
- The window is the size of its content, at most 80% of the screen on each side and never smaller than 480 by 360. Its title is the file name; the compositor draws the strip. It has no toolbar.
- Not done: syntax colours (they need a source-view library the image does not have), the window being tied to Files' window (that needs the Wayland window export, a further library), and Rename and Trash from the preview (the interface has the signals, the window does not offer them).

## Trust, in plain terms

- Any process of the person's session may call `ShowFile` on the session bus; that is the same trust the old previewer had. The caller can make the window show any file the person can read, and nothing else: no write, no program is run.
- `ParentHandle` keeps at most 256 bytes of what the caller sends (`HANDLE_LIMIT`). Quick Look does not use the handle for anything but that property.
- The address is read as given. Symbolic links are followed, as Files does, and the path is not canonicalised.
- Only a folder or a regular file is opened. A named pipe, a socket or a device (a `.txt` that is really `/dev/zero`) gets the card without a single read, and files are opened without blocking and checked again on the open descriptor.
- A picture that declares more than 200 million pixels is refused before it is decoded; the PDF renderer runs with its memory capped at 1 GiB (`prlimit`, part of util-linux) and its time at 8 seconds.
- One file loads at a time. A newer `ShowFile` replaces the one waiting, and Close or Space during a slow load drops it, so a slow file never appears after the person closed.
- The signals Quick Look sends (`SelectionEvent`, `NavigateTo`) cannot be forged by another process for Files: Files subscribes with the bus name as sender, and the bus daemon matches the name's owner.

## Who may call it, and what it trusts

- **Any process on the person's session bus can call `ShowFile`.** The session bus is the person's own; there is no separate permission, as with the old previewer. What a caller can make happen is bounded: it can show a file the person's own account can read, in the limits below, and nothing is written, run or sent. A sandboxed app that cannot reach the session bus cannot call it.
- The address is read as given: the path is **not canonicalised** and **symlinks are followed** the way Files follows them, so a link shows what it points to.
- Only a folder or a **regular file** is ever opened. The path is statted first; a FIFO, a socket, a device or anything else (a `.txt` that is really `/dev/zero`, a named pipe called `.png`) gets the card without a read. The open is non-blocking and the opened descriptor is checked again, so a file swapped for a pipe between the check and the open is refused too. The PDF reaches `pdftocairo` on its standard input from that same descriptor.
- A picture declaring more than 200 million pixels is refused before it is decoded (the loader reports the size first); the file is read through the loader, not by path. `pdftocairo` runs under `prlimit --as` (1 GiB) when `prlimit` is installed, as well as the 8 s and 32 MiB limits. Audio and video are opened by the player from the path after the regular-file check; the player, not this program, then reads them.
- `ParentHandle` keeps at most 256 bytes of what the caller sends (cut on a character boundary). The window handle is otherwise unused: the window is not tied to Files' window.
- Signals (`SelectionEvent`, `NavigateTo`) go out from the name this program owns. Files subscribes with that bus name as the sender, which the bus resolves to the current owner, so another process cannot make Files move its selection by sending a look-alike signal.
- One file loads at a time. A new `ShowFile` replaces the one waiting; `Close`, Space or Esc while a file is still loading drops it, so it never appears after being closed; any call from Files restarts the ten-minute idle quit.

## Keys

Space or Esc closes. Left, Right, Up and Down ask Files for the previous or next file. Return on a folder opens it in Files. Space in Files on the file in view closes it too (`closeIfAlreadyShown`).

## Design check (the guide's list, section 5)

1. The root view is not `AppWindow`: a preview has no toolbar and no sidebar, by the brief. It uses the kit's window surface, art tile, labels, icon buttons and row slider, and no style of its own.
2. Goldens at 900 by 600 and 480 by 360 pass the layout check, light and dark.
3. Loading: the window appears when the file is read (reads are bounded, so this is short). Empty and error states are the card with a sentence; the folder with nothing in it says so.
4. Shortcuts: Space and Esc close, the arrows move, Super+W reaches the window as a close and hides it like the close button.
5. The side by side check with Files on a real screen is in the journey.

## Standalone

`wol-quicklook FILE` shows one file and quits when it closes, without owning the bus name. `WOL_SNAPSHOT=<dir>` writes a picture of the window.

## Tests

- `crates/shell/src/quicklook.rs` unit tests: 31 names, the text reader, the folder counter, the bounded process runner, window sizing, the show and close rule, the loader for each kind.
- `crates/shell/tests/quicklook_bus.rs`: the interface on a private bus, and that the activation file names the binary.
- Goldens `quicklook-{image,code,audio,pdf,folder,unknown}-{900x600,480x360}` in light and dark (`crates/shell/tests/golden_cases/quicklook.rs`).
- Journey `e2e-quicklook` (Space on a picture in Files, an arrow, Space closes, a text file, Esc), run on the forge.

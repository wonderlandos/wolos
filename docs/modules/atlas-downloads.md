# Atlas downloads: they survive closing Atlas, and torrents are downloads too (ADR 0064)

**Concept.** A download in Atlas is never lost because Atlas closed, crashed or the computer lost power: the
partial file and its place stay, and Atlas continues it by itself the next time it opens. A magnet link or a
`.torrent` file is a download like any other: the same row in the same list, with the same progress, pause,
resume, cancel and "Show in Files". The torrent work is done by `atlas-torrent`, a small WolOS program beside the
browser that runs confined to the downloads folder; the browser has no torrent code.

**Why.** The engine already keeps a record of every download (its row in the download history: partial file,
offset, ETag, Last-Modified) and already knows how to ask a server for the rest of a file. What it did not do:
continue without a click, and keep the download when Atlas closes (a clean close deleted it). Using that one
record keeps the same row and the same file. Torrents need a peer-to-peer engine and parse untrusted data from
strangers; that runs out of process, in a sandbox, in a memory-safe language (ADR 0043 D4).

**How.**

```text
close / crash / kill -9          next start of Atlas                        the server
  download manager keeps the     DownloadResumer (chrome/browser/atlas/dl)
  partial file and the row  ───▶ sees the row interrupted, not paused  ───▶  GET with Range: bytes=<offset>-
  (patch 0130)                   by the person; resumes it (patch 0131)      If-Range: <ETag>  -> 206, the rest
                                                                            no validator / 200 -> from zero

magnet link / .torrent           TorrentContentProvider (offline items)    atlas-torrent (user service)
  click in a normal window  ───▶ ATT/1 over $XDG_RUNTIME_DIR/atlas/   ───▶  librqbit; checks every name and
  (patch 0132)                   torrent.sock, hello with launch token      path; writes only in Downloads
  private window: a row that says torrents are not available there (nothing is sent anywhere)
```

## Resuming downloads
- Kept at close: in normal windows nothing is lost. The first time Atlas closes with downloads running it says
  so once ("Downloads continue when you open Atlas again"; Close Atlas or Keep Atlas open); after that it closes
  without asking. A private window's downloads still end with it (the prompt says so, as before).
- Resumed without asking: downloads interrupted by a close, a crash, a network or server failure; again when the
  network returns. Not resumed: a download the person paused (remembered across restarts in
  `atlas.downloads.paused_by_person`), cancelled, or one held for a decision (dangerous, insecure, being scanned).
  At most 4 automatic attempts per download and session (waits 0, 5, 30, 120 s).
- A server that does not support resuming: the download starts again from zero.

## Torrents
- Nothing starts without the person: a magnet link (clicked or typed) or a downloaded `.torrent` file the person
  opens from its row shows a sheet first (name, info hash, what is known of the size, "your address becomes
  visible to the other computers sharing this torrent"); Not now is the default. A `.torrent` a page downloads
  by itself stays a file.
- `atlas-torrent launch` (run by the browser) first runs the unit's properties in `atlas-torrent-check`, which
  only checks its view from inside, then starts `atlas-torrent serve` as the transient user service
  `atlas-torrent`: ProtectSystem=strict, ProtectHome=tmpfs with only the downloads folder, the helper's state
  folder (`$XDG_STATE_HOME/atlas/torrent`) and the socket folder bound in, PrivateTmp, PrivateDevices,
  PrivateUsers, NoNewPrivileges, ProtectKernelTunables, ProtectControlGroups, ProtectHostname,
  ProtectProc=invisible, RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6, RestrictNamespaces, RestrictRealtime,
  RestrictSUIDSGID, LockPersonality, MemoryDenyWriteExecute, SystemCallFilter=@system-service (EPERM),
  MemoryMax=512M, TasksMax=64. `serve` checks again (tmpfs home with only the handed folders, `/usr` EROFS,
  NoNewPrivs 1, Seccomp 2, no block devices) and refuses to serve otherwise (exit 3, the row says the service
  manager cannot confine the helper). It never runs unconfined. The downloads folder may not be the home folder.
- Identity: the socket folder is 0700 and the socket 0600; SO_PEERCRED must be this account; the browser's hello
  carries the per-launch token `launch` printed to it, which the service read once from a 0600 file and removed
  (never argv or environment); inside the service another program's `/proc/<pid>/exe` cannot be read.
- Limits: `.torrent` at most 4 MiB, bencode depth 32, 10^6 values, 10,000 files, names of at most 255 bytes, no
  `..`, separators, NUL, control or Unicode format characters, 8 torrents at once, 60 peers each (8 while a
  magnet asks for its files); at most 32 trackers per job, never this machine or the LAN; peers in loopback,
  private, link-local or reserved ranges are blocked. A new job writes into a fresh, visible folder of its own in
  Downloads (a one-file torrent too), never over something that was there, never a name starting with `.`.
- Rows: a finished torrent holding a program, script, launcher or package is shown in Files but not opened from
  the row; a path outside Downloads is neither opened nor shown. Private windows and the Tor instance refuse
  torrents (a row says why) and never start the helper.
- Sharing is off: uploads disabled, no listening socket, finished torrents paused ("done, not shared").
  Trackers and the DHT find peers for the download.
- A torrent moves only while Atlas is open: when the browser goes away every job pauses at once and the service
  ends 20 s later; with nothing to do it ends after 20 s even while Atlas is open; a finished job leaves its
  state, and Atlas starts the helper at its start only when the state lists a job. The jobs continue when Atlas
  starts it again (the pieces on disk are checked first).

## Protocol ATT/1 (one JSON object per line)
Browser: `hello {version, token}`, `add_magnet {magnet}`, `add_torrent {torrent_hex}`, `pause {id}`,
`resume {id}`, `cancel {id, delete}`, `list`. Helper: `hello {version, sharing}`, `added {id}`,
`error {code, text}`, `job {id, name, state, have, total, peers, peers_known, down_bps, path, error, exts}`,
`gone {id}`. States: metadata, checking, downloading, paused, done, seeding, failed. The id is the info hash.

## Tests
- `cargo test -p atlas-torrent`: protocol, bounded parsing, tracker filter and bound, path confinement
  (components, links, fresh and hidden folders), magnet links, the service command line, the confinement check,
  the token file; `tests/cli.rs` (serve outside a unit refuses, launch refuses the home folder and folder names
  systemd misreads, no test tools in the package build); `tests/fuzz_parse.rs` (seeded mutations, 2 s;
  `ATLAS_TORRENT_FUZZ_SECS=1800` for 30 minutes).
- `cargo test -p atlas-torrent --features tools` adds `tests/swarm.rs`: a real swarm on 127.0.0.1 (generated
  20 MB file, a tracker in the test, the helper's seeder): no peer connection after the browser's connection
  drops, SIGKILL in the middle, the same job continues and completes with the same bytes, not shared.
- Browser (fork): `atlas_dl_unittests` (`*AtlasDl*`: keep at exit, the resume policy, the torrent policy and
  sheet, the protocol, the rows); journeys `atlas/tests/dl/e2e_resume.py` (kill -9, clean close, a server
  without resume) and `atlas/tests/dl/e2e_torrent.py` (a `tools` helper: drive-by `.torrent` starts nothing, the
  sheet, the confinement seen from inside, the token, no peers after the browser is killed, resume to
  completion, the helper ends with nothing to do).

## Pending
The "started again from the beginning" row text; the torrent texts in `dl_strings.grdp` with translations; the
ask-once sharing sheet; Share and Move to Trash on torrent rows; one helper per account (all profiles see its
rows); downloads folders with a space, colon, backslash, quote or % in their path; installing `atlas-torrent`
with the browser package; a `cargo fuzz` target in `fuzz/`.

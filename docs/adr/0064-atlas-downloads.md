# 0064 - Atlas downloads: they survive closing Atlas, and torrents are downloads too
Status: proposed (lead order 2026-11-05 04:09Z, in Portuguese: integrated torrent download, native and safe,
like any other download; and resuming a download from the middle, not losing it by closing the app). Builds on
0043 (the Atlas browser: D3 cheapest mechanism first, D4 no WolOS code in the engine) and 0053 (private and Tor
windows). Design notes with the options and premortems: `.claude/v012/atlas/dl-program.md` (internal).

## Context
Read in the engine at 156.0.8078.79 (the slot base tree):
- Every download already has a durable record: its row in the profile's history database (partial file path,
  received bytes, ETag, Last-Modified, the URL chain, parallel slices). After a crash or `kill -9` the history
  turns in-progress rows into interrupted ones (reason CRASH) and keeps the partial file; resuming sends a
  `Range` from the recorded offset with `If-Range`, and a server that refuses gets the whole file again.
- But a crashed download waits for a click, and a **clean close deletes it**: the download manager cancels
  every in-progress download at shutdown (`DownloadManagerImpl::Shutdown`, `Cancel(false)`), which discards the
  partial file, after the window asked "downloads in progress, exit anyway?".
- The download list (the engine's bubble and the Kit card) is built from download items *and* offline items of
  any provider registered with the profile's offline content aggregator; a provider's items get the same row,
  progress, pause, resume, cancel and open.

## Decision
**D1 - The engine's record is the one record.** No second journal. Atlas stops deleting it at exit and resumes
it without asking:
- Patch 0130 `dl/keep-at-exit`: `DownloadManagerDelegate::ShouldKeepPartialAtShutdown` (new, default false);
  when the Atlas delegate says yes (ordinary downloads of normal profiles), the download manager ends the
  download as interrupted (reason CRASH, `DownloadItemImpl::InterruptKeepingPartialForShutdown`) instead of
  cancelling it. Never kept: a dangerous, insecure or unconfirmed download (waiting for Keep/Discard or a scan,
  or with no target yet), a temporary or transient one: the engine deletes those at shutdown on purpose and so
  does Atlas (`atlas_dl::ShouldKeepAtShutdown`, review DL-SEC-11); the close asks when such a download would end.
- Patch 0133 `dl/close-sheet`: when only downloads of normal windows run, the close shows, once, the sheet
  "Downloads continue when you open Atlas again" (Close Atlas is the default, Keep Atlas open the other choice;
  pref `atlas.downloads.close_sheet_seen`); after that closing does not ask. A private window's downloads still
  make the close ask, because they cannot be kept.
- Patch 0131 `dl/core-service`: `atlas_dl::DownloadResumer` (chrome/browser/atlas/dl) continues every download
  interrupted by a close, a crash, or a network or server failure, at the start and again when the network
  returns; never one the person paused (kept across restarts in `atlas.downloads.paused_by_person`) or
  cancelled, never one held for a decision (dangerous, insecure, scanning); at most 4 automatic attempts per
  download and session with waits of 0, 5, 30 and 120 s (the engine adds its own cap of 5).
- A server that does not allow resuming (no validator, or 200 to a Range) gets the whole file again; the row
  starts over from zero.

**D2 - Closing Atlas pauses downloads; they continue when Atlas opens again.** Rejected for now: keeping the
engine running without a window until the downloads end. WolOS has no transfers indicator and no notification
server yet, so a windowless browser using the network would be invisible, which the Doctrine forbids. When the
shell has a transfers indicator this becomes a small keep-alive on top of D1 (follow-up).

**D3 - Torrents run in a WolOS helper, out of process.** `atlas-torrent` (crates/atlas-torrent, BSD-2-Clause,
`forbid(unsafe_code)`) wraps librqbit 9.1.0 (Apache-2.0: GPL-3.0-compatible, so the combined binary is
distributed under GPL-3.0-or-later terms, which GPL-2.0-or-later allows; maintained; DHT, peer exchange, uTP,
magnet). Rejected: cratetorrent (no DHT or magnet, unmaintained since 2021), the bip-rs crates (2018, tokio 0.1),
our own client (weeks of work and our own parser as attack surface).
- The browser starts it on demand with `atlas-torrent launch` (`/usr/bin/systemd-run --user`, never found
  through PATH), as the user service `atlas-torrent`: ProtectSystem=strict, ProtectHome=tmpfs with only the
  downloads folder, its state folder and its socket folder bound in, PrivateTmp, PrivateDevices, PrivateUsers,
  NoNewPrivileges, ProtectKernelTunables, ProtectControlGroups, ProtectHostname, ProtectProc=invisible,
  RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6 (no AF_NETLINK: neither librqbit nor its resolver uses it),
  RestrictNamespaces, RestrictRealtime, RestrictSUIDSGID, LockPersonality, MemoryDenyWriteExecute,
  SystemCallFilter=@system-service with SystemCallErrorNumber=EPERM, SystemCallArchitectures=native,
  MemoryMax=512M, TasksMax=64, UMask=0077. A user manager refuses ProtectKernelModules, ProtectKernelLogs,
  ProtectClock and CapabilityBoundingSet, so they are not asked for; it accepts IPAddressDeny= but does not
  enforce it, so the address fence is the helper's own (below).
- **Configured is not applied** (review DL-SEC-01): a user manager that cannot make user namespaces (Ubuntu's
  `apparmor_restrict_unprivileged_userns=1`, an old systemd) starts the unit with the mount fences silently
  absent. So the helper checks its own view and refuses to serve otherwise: `launch` first runs the same
  properties in a unit `atlas-torrent-check` that only looks from inside (`serve --check-only`: a tmpfs home
  holding only the handed folders, `/usr` read-only with EROFS, `NoNewPrivs: 1`, `Seccomp: 2`, no block device in
  `/dev`), and `serve` checks again before it binds its socket. A unit the manager cannot set up, or a missing
  fence, ends as exit 3 and the row "Torrents are off on this computer: its service manager cannot confine the
  torrent helper". It never runs unconfined, whether systemd fails loudly or quietly.
- Protocol ATT/1 over `$XDG_RUNTIME_DIR/atlas/torrent.sock` (folder 0700, socket 0600): one JSON object per line.
  Who may connect: this account (SO_PEERCRED uid) and the per-launch token. Inside the unit's user namespace the
  browser's `/proc/<pid>/exe` cannot be read, so the program pin applies only outside a unit (tests); the real
  check is **uid + token**. `launch` writes a fresh 32-byte token to a 0600 file in the socket folder, the
  service reads it once and removes it (never argv, never the service environment, which `systemctl show` and
  `/proc/<pid>/environ` show to every program of the account: decision 10315), and prints it to the browser
  that ran it; the hello carries it, compared in constant time; one bad hello closes the connection.
- Checks before the engine sees anything: magnet links (one `btih`, http/https/udp trackers only, `xs`/`as`
  dropped), `.torrent` files and the metadata a magnet fetches (bounded bencode: 4 MiB, depth 32, 10^6 values,
  10,000 files; pieces must cover the length; every path component checked: no `..`, no separator, no NUL, no
  control or Unicode format character such as right-to-left override, at most 255 bytes, depth at most 32), and
  every job writes only under a fresh, visible folder of its own in the downloads folder, a one-file torrent too
  (named after the file); a name starting with `.` is refused, and the downloads folder may not be the home
  folder or hold it (the bind would hand the whole home over): nothing of the person's is overwritten and no
  dotfile is planted (DL-SEC-05).
- Trackers and peers come from strangers (DL-SEC-03): at most 32 trackers per job (the `.torrent` bytes are
  rebuilt with the same info dictionary and only the kept trackers), http/https/udp only, never `localhost`, a
  single-label or `.local`/`.lan`/`.internal` name, or a literal address in a loopback, private, link-local,
  carrier-NAT, multicast, documentation or reserved range (numeric spellings like `127.1` and `0x7f.1` too);
  every peer address goes through the engine's blocklist of the same ranges, for outgoing and incoming peers. A
  tracker NAME that resolves into the LAN is not caught (the engine resolves it); its peers still are.
- Nothing starts without the person (DL-SEC-02): a magnet link the person clicked or typed, or a downloaded
  `.torrent` file the person opens from its row, shows a sheet first (name, info hash for magnets, what is known
  of the size, "your address becomes visible to the other computers sharing this torrent and to the trackers it
  names"; Not now is the default); only Download reaches the helper. A `.torrent` file a page downloads stays a
  file: it is never handed over because it finished. A magnet asks at most 8 peers at a time for its list of
  files (each may make the engine hold up to 32 MiB; DL-SEC-13), the unit's MemoryMax is the last fence.
- A finished torrent with a file type that runs programs or installs something (`.desktop`, `.sh`,
  `.AppImage`, `.run`, `.bin`, packages, executables, scripts) is a dangerous row: shown in Files, never opened
  from the row (DL-SEC-04; the engine's own file-type policy is not built in Atlas, safe_browsing_mode=0). A
  path the helper reports outside the downloads folder is neither opened nor shown (DL-SEC-16).
- Sharing is off: the session is built with uploads disabled and no listening socket; a finished torrent is
  paused ("done, not shared"). Trackers and the DHT are used to find peers for the download. Asking once to
  allow sharing (pref and sheet) is a follow-up.
- Like any other download, a torrent moves only while Atlas is open: when the browser's connection drops the
  helper pauses every job at once (no peer connection stays open or opens, no tracker is asked) and ends after
  20 s; the jobs it paused continue when the browser is back, in time or at the next start (the pieces on disk
  are checked first; fast resume is off because librqbit 9.1.0 trusts a stale bitfield after SIGKILL). With
  nothing to do (no job looking for its files, checking or downloading) the helper ends after 20 s even while
  Atlas is open, so its DHT node lives only while a job needs it; a finished job leaves its state (the files
  stay), and the browser starts the helper at its start only when that state lists a job (DL-SEC-08).
- What leaves the machine while a job runs (DL-SEC-18, defaults): DNS and UDP to the DHT bootstrap hosts
  (`dht.transmissionbt.com:6881`, `dht.libtorrent.org:25401`) when the helper starts; DHT `get_peers` for the
  job's hash (never `announce_peer`: there is no listener); announces to the job's kept trackers, which carry
  `port=4240` although nothing listens, so trackers give the person's address and that port to other peers;
  connections to the peers found, whose extended handshake and peer id say "Atlas" (librqbit's
  `client_name_and_version`), which makes Atlas users recognisable in a swarm. Nothing is uploaded.
- The test tools (`make`, `seed`, `serve --unconfined`, `--local-peers` for a loopback swarm) are in a `tools`
  build only, never in the package (DL-SEC-17).

**D4 - A torrent is a row of the same list.** `atlas_dl::TorrentContentProvider` is an offline content provider
(namespace `atlas_torrent`) of the profile: each helper job is an offline item (title, bytes, progress, peers in
the description, paused, complete, failed), so the engine's list and the Kit card show it with no list change.
Patch 0132 `dl/torrent-handler`: a `magnet:` navigation the person asked for goes to the provider instead of the
desktop's protocol handler (one without a gesture is dropped) and shows the sheet; opening a finished `.torrent`
download shows the sheet instead of handing the file to another program; the provider is made beside the
resumer (so 0130, 0131 and 0133 ship without torrents); "Show in Files" works for torrent rows.

**D5 - Private and Tor windows refuse torrents.** The private profile's provider never starts the helper: a
magnet link or `.torrent` there becomes a failed row that says why (other computers would see the address); it
does not even look at the helper's state, so opening a private window adds no row (DL-SEC-09). The Tor instance
is refused by its own switches too (`--atlas-tor-window`, `--atlas-tor-isolation`), not only because its
profile is private (DL-SEC-10): a torrent would leave the Tor network namespace.

## Consequences
- Closing Atlas, a crash, `kill -9`, a power cut or a logout no longer lose a download; the row is the same row.
- One more binary beside the browser (`atlas-torrent`, to be installed by the browser package) and one more
  user service while torrents run.
- Pending (honest list): the row text "started again from the beginning" for servers without resume; the
  torrent texts in `dl_strings.grdp` with translations (English literals in `torrent_texts.h` and the helper
  today); the ask-once sharing sheet; Share and Move to Trash on torrent rows (HOST's menu model reads download
  items only); one helper per account (rows of all profiles appear in each); a downloads folder whose path has
  a space, a colon, a backslash, a quote or `%` cannot be handed to the service yet (the row says the helper
  could not start; DL-SEC-14); the sheet and refusal texts are English literals in `torrent_texts.h`.
- Fuzzing: `crates/atlas-torrent/tests/fuzz_parse.rs` mutates `.torrent` files, bare info dictionaries and
  magnet links from seeds (2 s per test run, `ATLAS_TORRENT_FUZZ_SECS=1800` for 30 minutes) and checks no panic
  and the parser's promises; a `cargo fuzz` target in `fuzz/` is a follow-up (DL-SEC-19).

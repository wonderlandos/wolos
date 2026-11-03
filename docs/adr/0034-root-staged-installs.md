# 0034 - Locally built packages: who installs the staged copy
Status: Accepted (SF1 proposal 2026-11-02; implemented in v0.12 after audit5 C7-1). Builds on 0021 (wopkg asks, wold decides) and 0033 (identity handshake).

## Context
`package.install-file` installs a package built on this computer (the community package collection, a recipe): it is not from
the signed repos and WolOS has not reviewed it. Since audit2, every pacman transaction from the
repos (install, remove, update with or without names, update with holds) is an administrator sheet
that the shell runs through wol-admind after the password (`crates/api/src/packages.rs`, `request`).
`install-file` is the one pacman transaction left as an Allow sheet: wold copies the caller's file
into a fresh root-only directory under `/var/cache/wolos/install` before the sheet shows
(`Host::stage`, audit S16), and after Allow Once wold itself runs `wol pkg -- -U` on that copy.
The answer can no longer be forged (S1: the sheet goes to the shower over its `subscribe account`
connection, the answer only comes back as `session.answer` from that kernel-verified executable),
but it is still one click, without the password, for a root install of unreviewed code. The sheet
names the file, not what it contains.

## Decision
- `install-file`, `system.channel` and `system.rollback` are administrator sheets
  (`ApprovalRequest::PackageFile { staged, name, version, sha256 }`, `SystemChannel { channel }`, `SystemRollback`). The
  shell runs them through wol-admind after the password; wol-admind checks itself that the caller
  is an administrator, so a standard account's own shell answering its own sheet gains nothing
  (audit C7-1).
- wold stages the copy before the sheet as before (`Host::stage`, audit S16) and hands only the
  staged copy's path to the sheet. The new admin actions (`crates/shared/src/admin.rs`, planned in
  `crates/core/src/admin.rs`):
  - `pkg-install-file <staged path> <sha256>`: accepted only as `/var/cache/wolos/install/<32 hex>/<file>.pkg.tar.zst`
    (`is_staged_package`), the layout only root can write, with the 64-hex digest the sheet showed
    (`is_sha256_hex`); wol-admind reads the copy again, logs its name, version and SHA-256, and runs
    `wol pkg -- -U --noconfirm -- <path>` only when the digest matches.
  - `system-rollback`: `wol update rollback`.
  - `system-channel <stable|beta|nightly>`: writes `/etc/wolos/channel`.
  All three are `Fresh::Always` (never a cached authentication); `pkg-install-file` and
  `system-rollback` are long actions.
- The sheet says what the copy contains (`crates/api/src/packages.rs`, `inspect_staged`): wold
  decompresses its staged copy once with the system `zstd` (root-side launch, cleared
  environment, 120 s, at most 16 GiB unpacked, `scan_package`), finds `.PKGINFO` in the first
  MiB the way the package manager names members (pax `path`, long-name headers, ustar prefix; every
  header checksum checked, at most 64 KiB), and reads `pkgname` and `pkgver` (each exactly once;
  a name of letters, digits and `@._+-` up to 128 bytes, a version of letters, digits and
  `._+~:-` up to 64 bytes). The SHA-256 is of the whole staged file. Then every later tar header
  is walked to the end-of-archive block (bodies skipped unread, `tar_walk`), and a copy is
  refused when any other member's name (header, pax `path` or long-name header, a leading `./`
  ignored) also resolves to `.PKGINFO`, so the sheet never shows one `.PKGINFO` while the package
  manager reads another, or when the archive is damaged, cut off or holds a member kind the walk
  does not read. A copy without a readable `.PKGINFO` is refused before any sheet ("That package file can't be read (...)"). The title shows name, version and the first 12
  hex digits (`shared::shell::sha_prefix`); the detail line the whole digest
  (`copy::approval_package_file`). wold's own sheets travel as `Event::SessionAsk` (bounded by the
  60 KiB line limit); the 128-byte limit of helper-relayed sheets does not apply to them.
- wold waits for an administrator sheet as long as wol-admind may run it (`packages::ask_wait`),
  then removes its staged copy whatever the answer. wold itself runs one change only: a signed
  release newer than the running one (`wol update apply`, Allow sheet).
- Wire: new action names and new `ApprovalRequest` tags only. An older wol-admind answers
  `invalid` and an older shell does not parse the sheet, so both fail closed (nothing runs).

## Consequences
- Community-collection and recipe installs need the administrator password, like every other system install.
- The staging directory is shared between two root daemons; its layout (one fresh `0700` directory
  per call, the file created with `O_EXCL`, wold's directories 32 hex digits, wol-admind's with
  dashes) is the contract between them.
- The sheet names the package, its version and the SHA-256 of the copy that will be installed, so
  the digest can be compared with the one the builder published; wol-admind's log records them.
- `.PKGINFO` must sit in the first MiB of the decompressed package (makepkg writes it first); a
  package that places it later is refused.

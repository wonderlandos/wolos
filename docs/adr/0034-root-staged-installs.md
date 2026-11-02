# 0034 - Locally built packages: who installs the staged copy
Status: Proposed (SF1, 2026-11-02, after audit2 S1/S2/S16). Builds on 0021 (wopkg asks, wold decides) and 0033 (identity handshake).

## Context
`package.install-file` installs a package built on this computer (AUR, a recipe): it is not from
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
- Keep the Allow sheet for now, so AUR and recipe installs keep working in v0.12, with the copy
  staged before the question so what is allowed is what is installed.
- Proposed for the next release: make `install-file` an administrator sheet like the rest. wold
  stages the copy as today and hands wol-admind only the staged copy's directory name (a new
  `AdminAction::PkgInstallStaged`, one argument, checked to be a direct child of the staging root
  with `shared::safe_path::safe_join`); wol-admind runs `pacman -U` on it after the password, with
  `Fresh::Always` (an unreviewed package never rides a cached authentication). The sheet shows the
  package name, version and the SHA-256 of the staged copy, read by wold from the copy.
- wold then runs no pacman transaction of its own; `System::run` keeps only `wol update apply`
  and `wol update rollback` (signed releases and undo).

## Consequences
- Until the change lands, a person who clicks Allow Once on an "install the package …, which WolOS
  has not reviewed" sheet installs it as root without the password. The sheet text says it is
  unreviewed; that is the whole defence.
- After it lands: one new admin action (allow-list row, FRESH_PASSWORD row, a plan step and a
  validate rule in `crates/core/src/admin.rs`), and a contract test that wold's package service
  never runs `-U` itself. wopkg, the grants and the wire format do not change.
- The staging directory becomes shared between two root daemons; its layout (one fresh
  `0700` directory per call, the file created with `O_EXCL`) is the contract between them.

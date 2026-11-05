# Keyring: post-quantum secret store, unlock factors, access lists, audit, portability (ADR 0054)

Decision record: [ADR 0054](../adr/0054-post-quantum-keyring.md). Code: `crates/wolkeyring` (the store, the formats and the daemon path, BSD-2-Clause) on top of `crates/wolcrypt` (the primitives).

**Concept.** Every secret a person's apps keep (browser passwords, VPN keys, tokens) lives in one
store. Its keys are 256-bit symmetric keys, wrapped by the unlock factors; its exports are encrypted
to the hybrid KEM X25519 + ML-KEM-768 and signed with Ed25519 + ML-DSA-65 (`crates/wolcrypt`). There
is no classical-only path in any format of the crate.

**Why.** gnome-keyring's file is AES-128-CBC under a SHA-256 stretch with an MD5 check, no access
list per app, no audit, no export. A stolen disk today is a harvest for a later quantum computer; the
store must hold then too, and the person must be able to see who read what.

**How.** `crates/wolkeyring` (BSD-2-Clause, no `unsafe`): `store` (format, wraps, records, compaction),
`factor` (password, PIN with the TPM, TPM device key, fingerprint through the TPM, FIDO2 hmac-secret,
recovery code; rules = any-of, so n-of-m is the n-subsets), `kdf` (Argon2id 64 MiB x 3 passes,
PBKDF2-HMAC-SHA512 600 000 read and writable; a header below the floor or above the ceiling is
refused), `item` (metadata and access lists), `audit` (hash chain, ML-DSA-signed checkpoints),
`bundle` (signed, encrypted export to a recipient key or a passphrase), `pairing` (the LAN move with an
8-digit code), `proto` + `daemon` (the fast path), `secret_service` (D-Bus subset), `io` (every file,
process and socket access). Binaries: `wol-keyringd` (session daemon), `wol-keyring` (`wol keyring`),
`wol-keyring-bench` (numbers in `docs/bench/keyring.md`).

## Commands (`wol keyring VERB`, or `wol-keyring [--name NAME] VERB`)

| Verb | What |
|---|---|
| `init` | make the keyring; the password comes on stdin or the terminal; the recovery code is shown once |
| `status`, `id`, `list` | policy and factors; the public identity to give to an exporter; items without secrets |
| `set LABEL ATTR=VALUE...`, `get ATTR=VALUE...`, `delete ATTR=VALUE...` | secrets are read from stdin, never from an argument |
| `export FILE [IDFILE \| --passphrase]`, `import FILE` | a signed, encrypted bundle; import shows the signer and merges (newer wins) |
| `move receive [PORT]`, `move send HOST[:PORT]` | whole keyring between two WolOS machines on one network; both screens show the same 8 digits, confirm they match |
| `audit [verify] [N]`, `audit accept` | the access log; `verify` checks the chain, the signatures and the rollback record kept outside the data folder; `accept` takes the files as they are after a legitimate restore |
| `acl list\|allow\|revoke EXE [ATTR=VALUE...]` | the apps on an item's list, or on every item's without attributes (the owner at the terminal; the shell's sheet later) |
| `enrol recovery\|pin`, `compact` | a new recovery code, or a PIN that works only together with the TPM; drop replaced records |

Exit codes: 0 done, 1 refused (the line on stderr says why), 2 usage. Helpers and `wol mcp` cannot call
any keyring verb (it is not in `HELPER_VERBS`).

## The daemon

`wol-keyringd` (user unit `wolos-keyringd.service`, not enabled by the image yet) serves
`$XDG_RUNTIME_DIR/wolos/keyring.sock` (0600, one JSON line per request: `status unlock lock search get
set delete audit`) and the Secret Service subset on the session bus. The caller is always the process
the kernel or the bus names, pinned through `/proc`: executable path, its SHA-256, the systemd app
scope. An app not on an item's list is refused (no shell sheet exists yet); the creator is on it; an
item nobody is on the list of (imported, migrated, made at the terminal) is owner-only until
`wol keyring acl allow EXE` (every item) or `acl allow EXE ATTR=VALUE` (one). Each request re-checks
that the process is still the program that was identified. The daemon is not dumpable and has no core
file. Wrong passwords back off (none for 3, then 1, 2, 4 ... 30 s). The keyring locks after the
idle time (default 15 min), with the screen (logind session `Lock`), on `PrepareForSleep`, and on
request; `after-first-unlock` items stay readable while locked. At start it opens with the TPM device
key when the keyring has one.

## Settings > Privacy & Security > Keyring (page copy; the page is not built, see the note)

**Title** Keyring. **Subtitle** Your saved passwords and keys, kept encrypted on this computer.

- **Status row** "Unlocked" / "Locked". Button "Lock now". Text under it: "Your keyring locks when you
  lock the screen, close the lid, or after 15 minutes without use."
- **Lock** three switches: "Lock with the screen", "Lock when the computer sleeps", "Lock after [15 minutes
  | 5 | 30 | 1 hour | never]".
- **Ways to unlock** list with one row per factor and an "Add" button: "Login password" (always),
  "Fingerprint" ("Uses this computer's security chip; not enough on its own to export or change
  these settings"), "Security chip (this computer)", "PIN (6 digits or more, works with the security
  chip)", "Security key (FIDO2)", "Recovery code" ("Write it down. Shown once. Making a new one
  cancels the old one.").
- **Recent activity** a list, newest first: "[App icon] Helium read a saved password · 14:02",
  "Unlock with fingerprint · 13:58", "Export of 212 items · yesterday", "NetworkManager was refused
  access to 'Office VPN'". A button "Check the log is intact" runs `wol keyring audit verify` and
  shows "Intact, last signed 14:02" or the warning.
- **Apps** a list of apps with access, each with "Remove access"; a changed app asks again.
- **Move or back up** "Export...", "Import...", "Move to another computer...". Text: "Exports are
  encrypted so that only the chosen computer or passphrase can open them, and signed so you know where
  they came from."

Note for the shell lane: today the daemon answers `status` and `audit` on the socket; the page also needs
`settings get/set`, `factors list/enrol/remove`, `apps list/revoke` in `proto.rs` (a half day) and the
approval sheet "Allow once / Always allow / Deny" through `shared::shell::ApprovalRequest`. Until then
the same actions exist as `wol keyring` verbs and the page can call them.

## Not here

Wi-Fi passwords stay in NetworkManager's store (root-only file on the encrypted volume); Secure Boot
and TPM policy keys are RSA by firmware limit; the TPM's own RSA/ECC is a chip boundary (ADR 0054
Consequences).

# .wol packages (ADR 0011, 0016)

pacman is the package manager, used directly (`pacman -S ...` works as on Arch). `wol` owns only:
- `.wol` apps: metadata, icon, permissions, App Store. A `.wol` is a pacman package plus that metadata.
- The pinned `/etc/pacman.conf` and mirrorlist: every repo (Arch + `[wolos]`) reads the release's
  Arch Linux Archive date, so `pacman -Syu` never leaves the release.
- System releases: snapshot date, A/B root slot, signed manifest (`archicture/modules/security.md`).

Repos: `[wolos]` (WolOS packages, apps, `wolos-base`, `linux-wolos`) next to the Arch repos, all pinned
to the release. A release moves the date forward only after its tests pass.
No parallel package database: what pacman installed is the source of truth. App records
(`/usr/share/wolos/packages/<id>/Info.json`) ship inside the package, so pacman owns them.
Root slots are writable copies; `.wol` apps live in the root like any pacman package and come along
into the next slot on update.

## File format

ustar archive, regular files only (`typeflag` `0`), mode 0644, uid/gid 0, names exactly as below, in this order:

| Member | Required | Content |
|---|---|---|
| `Info.json` | yes | `shared::bundle::Info` (below) |
| `icon.png` | no | 256x256 icon for the install sheet |
| `payload.pkg` | yes | pacman package (`makepkg`, `.pkg.tar.zst` bytes), prefix `/usr` |
| `SHA256` | yes | `SHA256 (<member>) = <64 lowercase hex>\n` for every member above, sorted by name |
| `SHA256.sig` | no | raw 64-byte Ed25519 signature of the `SHA256` bytes |

Verify (`wol_core::package::verify`): order and names exact, no duplicates, every hash matches, `SHA256` lists nothing else, `Info.json` valid, `package` set. Signature verified by a trusted key = signed; absent = unsigned; anything else = refused.

## Info.json

```json
{"id":"io.github.adbeveridge.Raider","version":"3.1.0","name":"File Shredder",
 "summary":"Securely delete your files","icon":"io.github.adbeveridge.Raider",
 "categories":["Work"],"capabilities":["files"],"package":"raider",
 "targets":[{"abi":"freebsd-native","arch":"amd64","entry":"bin/raider"}]}
```

- `version` = the pacman version of `payload.pkg` (`pkgver-pkgrel`); `package` = its pkgname; `icon` = icon theme name.
- `categories`: `Create` `Work` `Play` `Develop`. `capabilities`: `shared::package::CAPABILITIES` (declared, shown, not enforced).
- Entries are relative to `/usr`. The payload installs the same bytes at `share/wolos/packages/<id>/Info.json`.

## Keys

| Key | Form |
|---|---|
| private (CI secret `WOL_SIGNING_KEY`) | PEM PKCS#8 Ed25519: `openssl genpkey -algorithm ed25519 -out wol-packages.key` |
| public (`/usr/local/share/wolos/keys/<name>.pub`) | PEM SPKI: `openssl pkey -in wol-packages.key -pubout -out wol-packages.pub` |

## Index

`https://wonderlandos.github.io/wol-packages/index.json`, signature `index.json.sig` (raw Ed25519 of the file bytes).
Files: Release `<package>-<version>`, asset `<package>-<version>.wol`.

```json
{"format":1,"packages":[{"id":"…","name":"…","summary":"…","version":"3.1.0","package":"raider",
 "categories":["Work"],"capabilities":["files"],"abi":"FreeBSD:15:amd64",
 "url":"https://github.com/wonderlandos/wol-packages/releases/download/raider-3.1.0/raider-3.1.0.wol",
 "sha256":"…","size":123456,"signed":true}]}
```

## Commands

| Command | Does |
|---|---|
| `wol install <file.wol\|id>` | `.wol` file or index id: verify, wol-admind `wol-install` = `pacman -U --noconfirm <payload>` (deps from the pinned repos); catalogue id: `pkg-install` |
| `wol remove <id>` | wol-admind `wol-remove`: `pacman -Rns --noconfirm <package>` |
| `wol list` | installed records in `/usr/share/wolos/packages/*/Info.json` (files pacman owns) |
| `wol search <term>` | curated catalogue + index |
| `wol update apps [--yes]` | installed `.wol` apps whose index version differs; `--yes` installs them |

Paths: staging `/var/cache/wolos/install` (root, 0700); downloads and index cache `~/.cache/wolos/packages`.
Double-click: MIME `application/x-wolos-package` (`*.wol`) opens the App Store install sheet.

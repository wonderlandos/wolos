# 0047 - WolOS Hub: one place to publish apps, plugins and extensions, with nothing hosted but metadata
Status: proposed (draft by HUB-arch 2026-11-03 for the lead orders of 23:18Z; lead decisions H1-H10 at the end). Builds on 0021 (wopkg trust per source), 0022 (judged by laws, never "safe"), 0031 (plugin index), 0042 (web-app runtime), 0046 (author certificates).

Labels: [mem] = from project memory or the code; [fresh] = reasoned for this record. Free-tier limits and prices are [fresh] and unverified against the providers' pages: HUB-worker checks them before launch.

## Context
- [mem] Plugins have a signed index (`crates/wopkg/src/index/plugins.rs`, format 1, `serial` and `expires`), published by the release tooling to `https://cdn.wolos.org/plugins/index.json` (`DEFAULT_URL` on `fix/e2e-0121`; `main` still names the old host at :28) and empty until plugins are reviewed. Its `.wolplug` files must be signed by the index key (`check_download`, :243).
- [mem] The site (wolos.org) is a static build on Cloudflare Pages with a binding design (`DESIGN.md`, `src/index.css`, `public/brand/colors.css`). crystal.wolos.org already runs as a small Cloudflare Worker. The account's tooling login here can write Pages, Workers, KV and D1, but not DNS. The package CDN is S3 behind CloudFront on AWS credits.
- [fresh] The lead wants hub.wolos.org: GitHub sign-in, a CLA, the site's look; apps, plugins and extensions; anyone publishes, certified authors stand out (ADR 0046); a report system; automatic rule analysis; WolOS's own compiled packages listed too; files come from GitHub tags and releases, never from our servers; cheap (static Pages, a TypeScript Worker, Rust compiled to wasm); organisations tied to GitHub organisations; Doctor and a visual maker for apps and functions as web apps.

## Decision

### Components
- **hub site** (static, Cloudflare Pages, hub.wolos.org): listings, author and organisation pages, publish forms, reports, moderation queue, Doctor and Maker. It takes the site's tokens and type scale from a pinned copy of the site's `public/brand/colors.css` and the `@theme` block of `src/index.css` (a sync script with the site commit in it, checked in CI), so the two never drift and the hub never forks the look. Browsing needs no Worker call: pages read the published index (static JSON) directly.
- **hub Worker** (TypeScript, Cloudflare Workers free plan): GitHub sign-in, CLA acceptance, listing registration, release webhooks, certificate requests, reports, moderation actions. It never signs, never downloads an artefact and never hashes one: with about 10 ms of CPU per request on the free plan, heavy work goes elsewhere.
- **hubcheck** (Rust, one crate built twice): as wasm, in the browser (check a file before publishing, "check this file yourself" on every listing, Maker, Doctor) and in the Worker (manifest and certificate-request checks, each well under a millisecond of signature work, ADR 0046); as a native binary, in the check job. It uses the pure trust crate of ADR 0046 and the pure parts of the `.wol`/`.wolplug`/manifest rules; nothing with I/O, libc or memory maps.
- **Pipelines** (GitHub Actions in the public repository `wonderlandos/hub-index`, free for public repositories):
  - `check` (every 15 minutes and on demand, no secrets): fetches pending releases, downloads each asset by URL, checks size and sha256, runs hubcheck natively plus the malware engine (`xguard`, ADR 0024) and the plugin checks (`wol plugin check`), and commits one verdict file per release to the repository.
  - `sign` (after `check`, its own environment, the only job allowed to call the signing role of ADR 0046): builds the index from the registry export and the committed verdicts, signs it and the revocation list, uploads them, commits the signed copies.
  - `ca` (on request, environment with required human approval): issues certificates with the Authors CA.
  Untrusted files are parsed only in `check`, which holds no signing right.
- Nothing else is hosted: artefacts stay GitHub release assets, pinned by sha256; WolOS hosts only metadata.

### Data model (D1, one database)
| Table | Key fields |
|---|---|
| `accounts` | `github_id` (pk), `login`, `name`, `avatar`, `gh_created`, `two_factor` (seen at sign-in), `cla_version`, `cla_at`, `role` (`member|moderator|ca-operator`), `banned_at` |
| `orgs` | `github_org_id` (pk), `login`, `name`, `cla_version`, `cla_by`, `cla_at` |
| `namespaces` | `prefix` (pk, e.g. `alice.`), `owner_kind` (`person|org|wolos`), `owner_id` |
| `listings` | `id` (pk, app or plugin id), `kind` (`app|plugin|extension`), `host` (extensions: the app id it extends), `owner_kind`, `owner_id`, `repo_id` (numeric) and `repo` (`owner/name`), `name`, `summary`, `license`, `state` (`active|hidden|removed`) |
| `releases` | (`listing_id`, `version`) pk, `tag`, `commit`, `asset_url`, `sha256`, `size`, `cert_serial`, `trust` (`wolos|verified|community`), `verdict` (`pending|pass|flag|block|not-checked`), `first_seen`, `yanked_at` |
| `certificates` | `serial` (pk), `kind`, `subject_kind`, `subject_id`, `namespaces`, `body`, `state` (`active|revoked`), `revoked_at`, `reason` |
| `cert_requests` | `id`, `account`, `request` (public keys and proof of possession), `state`, `decided_by`, `decided_at` |
| `reports` | `id`, `listing_id`, `version`, `reporter`, `category` (`malware|impersonation|broken|license|conduct`), `text` (2000 characters), `state` (`open|dismissed|actioned`), `moderator`, `action` |
| `moderation_log` | every action with actor, target, reason and time; published |

- Sessions are signed cookies (HMAC, 7 days), not storage rows; KV holds only read-mostly caches (GitHub signing keys for the pipeline tokens, organisation membership for 10 minutes), because the free KV allows about 1,000 writes a day. Counters for rate limits are D1 rows per account and day. R2 is not used.
- [fresh] Free plans, to be checked: Workers 100,000 requests a day; D1 5 GB, 5 million rows read and 100,000 written a day; KV 100,000 reads and 1,000 writes a day; Pages unlimited static requests.

### Sign-in, organisations, CLA
- One GitHub App, "WolOS Hub" (H1): sign-in with user tokens asking only for the public profile and organisation membership; release webhooks for repositories its owner installs it on; read-only metadata. A GitHub App instead of a plain OAuth app because it gives sign-in and release events in one registration, with narrower permissions.
- An organisation joins when one of its GitHub owners signs in, picks it and accepts the CLA for it. Who may publish or manage comes from GitHub at the moment of the action: owners manage the hub page and certificate requests, members publish under the organisation's namespace. Nothing is copied that GitHub would have to keep in step.
- The CLA text is the lead's (H2). The hub stores the version, its sha256, the GitHub id and the time of acceptance; a new version is accepted again before the next publish. No IP address is stored.

### Publish flow
1. An author signs in, accepts the CLA and registers a listing: id inside a namespace they own, kind, repository. The repository's owner on GitHub must be that person or organisation (checked by numeric id).
2. They push a tag and publish a GitHub release with the file (`.wol`, `.wolplug`, or an extension) as an asset; optionally signed locally with `wol sign` (ADR 0046).
3. The release webhook reaches the Worker (signature checked); it records the release as `pending` with tag, commit, asset URL and size.
4. `check` downloads the asset (at most 1 GiB, H6), computes sha256, verifies the format and any certificate chain, checks the id and version in the file against the listing, runs the rules, and commits `verdicts/<id>/<version>.json`. It reports to the Worker with a GitHub-issued token for its own workflow (the Worker checks repository, workflow and environment claims; there is no shared secret).
5. `sign` rebuilds the index from the registry export and the committed verdicts, re-checks each new entry's owner against GitHub (so a taken-over Worker cannot attach a release to someone else's id), signs and uploads it, and commits the signed copy. The git history of `hub-index` is a public, append-only log of every index ever signed.
6. The OS fetches the index, shows the listing, downloads the asset from GitHub and checks size, sha256 and (for verified files) the author's signature itself; the index pins the file, the author's signature names the maker.

- A moved tag or a replaced asset does not change what is installed: the sha256 is pinned; a replaced asset fails the check and the release is marked so.
- Updates of unsigned (community) items never install on their own; they ask again (ADR 0046's sheet, the plugins' typed `yes`).

### Rule analysis (automatic, never "safe")
- Structure: the format checks of ADR 0011 and 0031, manifest schema, id in the owner's namespace, version higher than the last, size caps, `min_wolos`.
- Changes: permissions and capabilities against the previous release ("Asks for more now"), a new install scriptlet, a new owner.
- Content (`.wol` payloads): the built-package rules of ADR 0021 (setuid, device files, files outside `/usr`, `/etc`, `/opt`, `/var`, hooks, units, the install script shown); plugins: `wol plugin check`; everything: the malware engine.
- Verdict: `pass` (no rule broke), `flag` (listed, with every finding in plain words), `block` (not listed: malware hit, wrong id, damaged file), `not-checked` (too large, or a check timed out). As ADR 0022: a clean result means only that no rule broke.

### Index format shared with the OS
- **Extended format 1** (`index.json` + `index.json.sig` + `index.json.wsig`). The 0.12 parser ignores unknown keys (no `deny_unknown_fields` in `Raw` at :71 or in `Entry`), so new data is added, not changed:
  - top level: `format: 1`, `serial`, `expires` (as today, re-signed at least daily, 7 days ahead), plus `issued`, `apps: [...]`, `extensions: [...]`, `intermediates: [<.wolcert>]`, `revoked: {url, sha256, serial}`;
  - every entry keeps today's plugin fields (`id, name, version, summary, author, license, kinds, permissions, min_wolos, url, sha256, size`) and adds `trust`, `cert` (leaf serial), `owner` (`{kind, github_id, login}`), `source` (`{repo, tag, commit}`), `verdict`, `findings`, `first_seen`, `yanked`; apps add `capabilities` and `categories`, extensions add `host`.
- `index.json.sig` is the legacy Ed25519 signature for 0.12; `index.json.wsig` is the hybrid signature of ADR 0046 over the same bytes. 0.13 requires `.wsig` and never falls back.
- 0.12 checks `.wolplug` signatures with the old key (`check_download` with `require_signed`), so the 0.12-compatible copy lists only plugins WolOS signed itself; hub plugins from GitHub need 0.13, where an exact sha256 match with a `.wsig`-signed index counts as "listed", and an author's signature as "verified".
- WolOS's own pacman packages appear as read-only listings from the package API already on the CDN (`packages.json`), marked WolOS; WolOS-built `.wol` apps are signed by the Packages CA.
- Limits: the 0.12 reader caps the index at 4 MiB and 5,000 entries; at about 800 bytes per entry the hub shards by kind (`plugins/`, `apps/`, `extensions/`) before that.

### Where the index is served [fresh]
| | A. hub Pages (`hub.wolos.org/index/`) | B. `cdn.wolos.org/hub/` (S3 + CloudFront), recommended | C. GitHub (raw files or Pages of `hub-index`) |
|---|---|---|---|
| Cost at 10k people | free | about 180 GB a month inside CloudFront's free 1 TB; on credits beyond | free |
| Already in the OS | no | yes (`DEFAULT_URL`) | no |
| Publish | a Pages deployment each time (build-count limits on the free plan, H7) | one upload and an invalidation from `sign` | a commit |
| Failure shared with artefacts | no | no | yes (same host as the files) |

**Recommended: B as the address the OS uses, C as the log (and a mirror the OS tries second), the hub site reading B.** A host needs no trust because every file is signed; what matters is staying up, cost and how simple publishing is.
**Premortem of B**: the credits run out (a few dollars a month at 10k people, more with growth: watch the bill); the upload role leaks (the attacker can delete or replay, not sign: `serial` and `expires` show it, C stays the mirror); CloudFront caches an old index (short `max-age` on the index, invalidation on every publish); one cloud account behind both the CDN and the signing keys is one place to get wrong (separate roles, no shared credentials).

### Moderation, reports, abuse
- Reports need an account at least 7 days old, at most 5 a day. Moderators are a GitHub team in `wonderlandos` (H3). Actions: dismiss; hide a listing (out of the next index, within the hour); yank a version (`yanked: true`, installed copies show a line and are not offered as updates); ask the CA to revoke (`abuse`, two people, ADR 0046). Every action goes to `moderation_log`, which is published without the reporter's name.
- Rate limits: sign-in 10 a minute per address; 20 releases a day per account; 50 listings per account; certificate requests 1 a week. First listings of accounts younger than 30 days wait for a `pass` verdict before they appear.
- Impersonation: names close to an existing listing's name or id are flagged for a moderator before listing.

### Web apps: Doctor and Maker
- **Doctor (web)**: a static page in the hub and a WolOS web app (ADR 0042) that shows the JSON of the existing doctors (`wol audio doctor --json`, `wol games doctor --json`) as checks with their fixes. It reads the JSON from the local machine (the web-app runtime on WolOS, a pasted or dropped file in a browser); nothing is uploaded. The CLI stays the source of the checks.
- **Maker (web)**: a visual editor, in the calm style of the system's own tools, for plugins (manifest, Lua, live preview of the view tree in wasm) and for small apps and functions; it validates with hubcheck in the browser and exports a folder or `.wolplug` and a ready repository layout with a release workflow. It never asks for write access to a repository; the person pushes it themselves. The native Plugin Maker of ADR 0031 stays.

### Cost per 10,000 people [fresh, list prices, unverified]
| Item | Monthly |
|---|---|
| Pages (site), Workers, D1, KV on free plans | $0 while under the daily limits above; the next step is the $5 Workers plan (H8) |
| Signing keys (about 6) and signatures (about 1,500 a month) | about $6 |
| Index on the CDN | $0 inside the free transfer |
| Pipelines (public repository) | $0 |
| Artefacts (GitHub release assets) | $0 to WolOS |

## Dependencies (names appear only here)
| Name | Use |
|---|---|
| Bun | builds, tests and bundles the TypeScript Worker and the site |
| wrangler | deploys the Worker and Pages (already installed here) |
| wasm-bindgen | the wasm boundary of hubcheck |
| fips204, fips205, ed25519-dalek | through the trust crate of ADR 0046 |

## Consequences
- WolOS hosts metadata only; an attacker who takes the hub can hide or spam, not sign or swap a file.
- One Rust code base checks files in the OS, in the pipelines, in the Worker and in the browser, so the hub cannot judge a file differently from the system.
- Publishing is a git tag; listing appears within about 15 minutes; a takedown within an hour; a revocation reaches online machines within an hour.
- A new repository `wonderlandos/hub` (site, Worker, wasm, web apps) and `wonderlandos/hub-index` (pipelines, verdicts, the signed log) (H9).

## Lead decisions
- H1: register the GitHub App "WolOS Hub" in `wonderlandos` (callback `https://hub.wolos.org/auth/callback`) and add its id and secrets to the Worker.
- H2: the CLA text (and whether it is a contributor agreement or a publisher agreement).
- H3: who moderates, and a takedown contact.
- H4: the index host (B recommended).
- H5: what an "extension" is (draft: a package that extends one named app).
- H6: the largest artefact checked (1 GiB recommended).
- H7: the hub.wolos.org custom domain on Pages (the zone is active; the login here can add Pages domains).
- H8: stay on free plans, or allow the $5 Workers plan when the limits are reached.
- H9: create the two repositories and their licences (GPL-2.0-or-later recommended; the trust crate BSD-2-Clause like libwol).
- H10: list WolOS's own pacman packages on the hub (recommended, read-only).

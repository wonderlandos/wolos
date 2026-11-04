# 0049 - Hub supply-chain policy: how the hub stays small, slow and boring where registries became dangerous
Status: proposed (draft by HUB-policy 2026-11-04 for the lead order of 02:27Z; lead decisions P1-P8 at the end). Builds on 0022 (judged by laws, never "safe"), 0046 (author certificates: Origin and Trusted), 0047 (the hub: metadata only, pipelines, index). Research with incidents and sources, which this record does not name: `.claude/v012/hub/supply-chain-research.md`.

Labels: [mem] = from project memory, the code or the two earlier records; [fresh] = reasoned or researched for this record. Where this record says "today's rule" it names a rule id of hubcheck as it is in `wasm/src/files.rs` and `wasm/src/manifest.rs`.

## Context
- [mem] The hub lists apps (`.wol`), plugins (`.wolplug`) and extensions published by third parties. Files stay on the authors' GitHub releases, pinned by sha256 in a signed index. Every release goes through `check` (hubcheck, the malware engine, the plugin checks) and gets a verdict `pass|flag|block|not-checked`. Two certificate levels exist: **Origin** (automatic after GitHub sign-in and the CLA: says who published and from which repository) and **Trusted** (a person approves; for organisations and well-known people). Every version carries its own signed release statement, checked offline. Revocation is one signed list, checked offline.
- [fresh] The lead's order: do not become what the big package registries became. What was researched (research file, section 1): in the last two years the registries that let anyone publish anything, run code at install, and re-publish from a token lying on a disk suffered, in order of damage: worms that steal publish tokens on install and re-publish themselves into every package the token can reach (several waves, hundreds to over a thousand packages each, some with valid build provenance); maintainer takeovers by phishing a two-factor code on a look-alike domain; a build pipeline that got a valid publish token because the publishing rule trusted a whole repository instead of one workflow on one protected branch, so an orphan commit pushed to a fork was enough to ship a signed release of a popular framework; release pipelines poisoned through a compromised security scanner or a leaked key found in logs; release assets replaced under the same tag; abandoned or sold packages and browser extensions that turned malicious in an update; look-alike names; and a long tail of throwaway accounts publishing one stealer each.
- [fresh] What stopped attacks in practice (research file, section 2): no code at install (the big registries' package managers now default to it, and the attackers moved to runtime triggers, which only a sandbox answers); a wait between publication and visibility (in ten studied incidents the malicious version was removed within 1 to 12 hours in seven cases and within 3 days in eight; a 3-day cooldown skips all but two slow burns, and the dependency tools' defaults converged on 1 to 3 days); names bound to an identity so a look-alike cannot borrow a reputation; publishing bound to one workflow on one protected branch; human review of permission widening (browser stores); builds done by the store from the source (one phone-app store, one desktop-app store); and fast, remote, offline-checkable block lists.
- [fresh] What did not stop attacks: a two-factor mandate alone (codes are phished; sessions are stolen); provenance alone (a hijacked workflow produces valid provenance); scanning alone (payloads were obfuscated, downloaded a second stage, or slept until a condition); a badge given once and never re-checked.

## Decision in one paragraph
The hub is a catalogue of pointers, not a registry of code: it hosts nothing and the system runs nothing at install. Every version, at every level, passes the same automatic gates in the pipeline before it is listed, and the person always sees the findings, never the word "safe". The Origin level is minimal: an automatic certificate, automatic checks, a waiting period for new accounts, name rules, and hard refusals (no code at install, no network at install, no file outside the package's own folders, size caps). The Trusted level adds a human: an organisation or a well-known person, each release reviewed by a WolOS reviewer before the index lists it, build provenance tied to one workflow on one protected branch, an incident contact, and a yearly revocation drill. Updates never widen what an app may do without the person seeing it, and the system never installs an older version over a newer one. Every rule in this record names its enforcement point and owner (section "Mapping"), because a rule with no gate is a wish.

## Threat model by actor
| Actor | What they try | What stops them here |
|---|---|---|
| **A new malicious account** (throwaway GitHub account, one listing, a stealer) | Publish once, get installs before anyone looks | Origin needs: GitHub two-factor on, account at least 90 days old, repository at least 30 days old with commits before the first release; the first listing of a young account waits 7 days after `pass` before it appears; 20 releases a day, 50 listings; no code runs at install so a stealer has to be an app the person opens, with the question sheet if unsigned |
| **A hijacked maintainer** (phished code, stolen session, stolen token, sold account) | Publish a bad version of a good app | A release must be a GitHub release on a tag of the linked repository, whose commit is reachable from the default branch; the release statement is signed with the author's key, which the hub never holds, so a GitHub takeover alone publishes nothing verified; a Trusted release is read by a reviewer; a changed owner, a new maintainer or a new signing key is a `flag` the person sees; revocation reaches online machines within an hour |
| **A compromised build** (CI secrets stolen, workflow poisoned, a tool in the pipeline backdoored) | Ship a signed release from the real repository | Origin: the asset's sha256 is pinned at first sight; a replaced asset is `block`; the version must grow; new permissions, a new install script or a size jump are `flag`. Trusted: provenance must name one workflow file on one protected branch, and the release is read by a person; reproducible builds are checked when the author declares them, and a mismatch is `block` |
| **Dependency poisoning** (a bad library pulled into the author's build) | Reach the app through its parts | A `.wol` is a closed package: nothing is fetched at install or first run by the system; what is inside is scanned as content; plugins may only use the system's own modules (today's `permissions/undeclared`, allow-list in the manifest schema); apps that bundle a runtime are flagged with the runtime named |
| **Look-alike names** (typosquat, homoglyph, reserved words) | Borrow a reputation | Ids live in the owner's namespace (`alice.`), shown with the GitHub login; a name or id close to an existing listing's waits for a moderator; `org.wolos.`, `wolos`, `system`, `store`, `hub` and the system's app names are reserved; mixed scripts and confusable characters are refused |
| **Update hijack** (a good app's next version turns bad) | Use the install base | A version never widens capabilities or permissions silently: the index marks `widened`, the Store asks again; a yanked version is withdrawn from updates; downgrade protection: the system never installs a lower version from the hub than the one it has |
| **Abandoned-project takeover** (expired domain, transferred repository, a new owner) | Inherit the listing | A transferred repository withdraws the listing (today's Worker rule); a new owner is a new namespace, never the old one; a listing with no release for 24 months is marked "unmaintained" and leaves the top of search; a Trusted certificate expires yearly and is renewed only by the same approval |

Out of scope here, with their own records: the OS runtime sandbox (0040), the plugin runtime (0031), the trust format and revocation (0046), the index and hosting (0047).

## Rules per level

### Everyone, every version (the floor)
Gate: **pipeline check** (hubcheck, native, in `check`), unless marked otherwise. What the person sees: the verdict and the findings' sentences, in the hub and in the Store sheet, word for word (ADR 0022).

| Rule | Level | Gate | What it means |
|---|---|---|---|
| F1 Format and hash | block | hubcheck (today's `size/*`, `manifest/*`) | The file is a well-formed `.wol`/`.wolplug`/extension, sizes inside caps (app 1 GiB, plugin file 2 MiB, 256 files of 256 KiB), id, name, version, license present |
| F2 Pinned asset | block | `check` + Worker (today) | The sha256 and size seen first are the only ones; a changed asset under the same tag is `block` and the listing gets a line "A file changed behind this release" |
| F3 From a tag on the linked repository | block | Worker webhook + `sign` (`verify-owners`) | The release is a GitHub release on a tag of the registered repository, whose owner by numeric id is the listing's owner; the tag's commit is reachable from the default branch (an orphan or fork-only commit is `block`, the lesson of the framework case); a draft or pre-release is not listed |
| F4 No code at install | block | hubcheck (new rule `install/script`) and the OS | A `.wol` has no install scriptlet, no package hook, no unit enabled at install; an app that needs setup does it at first launch, in the app's own sandbox, with the person present. The plugin format already has no such thing |
| F5 No network at install | block | OS (admind, 0034) + hubcheck (`install/fetch`) | Installing is unpacking a pinned file; any URL in an install path, a download step or a "fetch the rest later" manifest key is `block`. The runtime's `network` capability is a different thing and is shown as such |
| F6 Files stay in their folders | block | hubcheck (today's `files` rules) and the OS | Nothing outside the app's own folders, nothing setuid, no device files, no hidden names, no duplicates |
| F7 Dependency allow-list | block for plugins, flag for apps | hubcheck (`permissions/undeclared`, new `content/bundled-runtime`) | A plugin uses only the modules the system offers and only those it declared. An app that bundles an interpreter or runtime is flagged with its name ("This app carries its own copy of a script runtime") |
| F8 Version grows | block | Worker (export) and hubcheck | A version equal to or lower than the last listed one is refused; the index keeps the highest; a yanked version never comes back |
| F9 Content rules | flag or block | hubcheck (today's `content/*`), malware engine (0024), `wol plugin check` | Scripts for other languages, binaries in a plugin, secrets in the file (`content/secret`), shell runners, obfuscation (long base64, `eval`-like strings, packed archives inside archives) are flagged in words; a malware engine hit is `block` |
| F10 Timed out or unreadable | not-checked | `check` (120 s) | Never listed as pass; shown as "Not checked: too large to read in time" |

### Origin (automatic; minimal, but real)
Gate: **Worker** at sign-in and registration, **pipeline** at each release. What the person sees: "Made by Alice Doe (alice on GitHub). Origin checked: who published it and from which repository. WolOS has not reviewed what it does."

| Rule | Gate | Detail |
|---|---|---|
| O1 Two-factor on GitHub | Worker at sign-in and before every publish (field `two_factor`) | Without it, no Origin certificate and no listing. Known limit: GitHub tells the hub that two-factor is on, not which kind; one-time codes are relayed by live phishing proxies, only hardware or passkey sign-in resists them, and the hub cannot see the difference. This rule removes the cheapest takeover, not all; the real defence against an account takeover is that the author's signing key is on their machine, never on GitHub or the hub (O8) |
| O2 Account age | Worker | GitHub account at least 90 days old (`gh_created`) for the first listing; younger accounts may sign in, accept the CLA and check their files in the browser, not publish |
| O3 Repository age and activity | Worker at registration, `sign` re-check | Repository public, at least 30 days old, with at least 5 commits before the first release, the default branch protected against force-push and tags protected (both readable through the GitHub API). These are signals, not proofs; they cost an attacker calendar time, the one thing throwaway accounts lack |
| O4 Cooldown | `sign` (index build, `available_at`) | The first listing of an account appears 7 days after its first `pass`; during the week it shows on the author's own page only, with "Listed on 11 November". A report in that week stops the clock until a moderator looks. Later releases of a listed author: a 72-hour cooldown before they are offered as updates (visible on the listing at once, marked "new: available as an update on 7 November"); the author may install their own release at once from their page. Trusted releases skip the cooldown because a person read them. Why 72 hours and not 24: the researched takedown windows (research file, section 2) were 1 to 12 hours for most, about 3 days for a few; 72 hours is where the dependency tools' defaults landed |
| O5 Rate limits | Worker | 20 releases a day per owner, 50 listings per person, 5 new listings a day, 1 certificate request a week (today's numbers, kept) |
| O6 Name rules | Worker at registration + moderator | Id in the owner's namespace, lower case, ASCII letters, digits, dots and dashes; display names at most 40 characters; reserved words refused (`wolos`, `wol`, `system`, `store`, `hub`, `settings`, `update`, `security`, `official`, `verified`, `trusted`, every id of a system app); a name or id within edit distance 2 of an existing listing, or equal after removing dots and dashes, or using a confusable character, goes to the moderation queue before listing (today's "Impersonation" rule, made exact) |
| O7 Release from a tag | as F3 | plus: the tag must have existed on GitHub before the webhook arrived; the Worker records tag, commit and the release's own `created_at` |
| O8 Signed release statement | hubcheck | The release statement (0046) names id, version, sha256, source commit and the certificate; `check` verifies it; a release without it is community level, with the question sheet |
| O9 Reproducible-build bonus | `check` (new job step, best effort) | An author who declares `build: {workflow, reproducible: true}` in the listing gets their asset rebuilt from the tag in a clean runner; a byte-identical result shows "Rebuilt by WolOS from its source: identical" on the listing; a mismatch is `flag` ("WolOS could not rebuild this file from its source"), not `block`, at Origin. No bonus changes the level |
| O10 Changes are shown | hubcheck (`change/*`, new) | Against the previous listed release: permissions or capabilities added, an install path added, size more than doubled, a new signing certificate, a new repository owner: each a `flag` with its sentence; the index carries `widened: true` when permissions or capabilities grew |

### Trusted (manual; organisations and well-known people)
Gate: **human step** by a WolOS reviewer, recorded in the public log, on top of everything above. What the person sees: "Made by Example Labs (examplelabs on GitHub). Trusted publisher. Each release is read by WolOS before it is listed. Last reviewed 4 November by a WolOS reviewer." Never "safe", never "secure".

| Rule | Gate | Detail |
|---|---|---|
| T1 Who | human (two approvers, never the subject; today's `CA_APPROVERS`) | A legal entity (company, foundation, project with a public governance page) or a person whose public work the reviewers can name; the request states the entity's public page and the GitHub organisation; the approval is logged with the reason in one sentence. Trusted is given to the publisher, not to a file |
| T2 Every release reviewed | human, in the hub's moderation queue, before `sign` | A Trusted release enters the index only after a reviewer marks `reviewed` with their login: they read the findings, the diff of the manifest and the file list against the previous release, the install path, and open the source tag; a release the reviewer does not understand is listed at Origin level ("Not reviewed yet") rather than held forever. Target: within 2 working days; if the hub cannot keep that, the lead sees it in the weekly numbers |
| T3 Build provenance | `check` | The asset must come with build provenance that names the repository, one workflow file and one protected branch, and the asset's sha256; a release built on a maintainer's laptop may still be listed, but at Origin level for that version. A provenance that names a different workflow or branch than the one registered is `block`. Provenance is compared with this expectation, never trusted on its own: in the researched cases a hijacked workflow produced valid provenance every time |
| T3b Release workflow and releases are locked | `check` reads the workflow file at the tag; Worker reads the repository settings | The repository has immutable releases on (assets and tag frozen once published); the registered release workflow runs only on tags or the protected branch, has no trigger that runs code from a pull request, an issue or a comment with write rights or secrets, and pins the actions it uses by commit. A breach of any of these lists that version at Origin with "Not reviewed yet: the release workflow needs a fix", and the reviewer tells the publisher what |
| T4 Reproducible when declared | `check` | For Trusted, a declared reproducible build that does not match is `block`, not `flag` |
| T5 Incident contact | Worker, at approval | A security address and a second person; the hub tests the address at approval and every 6 months; a contact that does not answer in 14 days downgrades to Origin |
| T6 Revocation drill | human, yearly | The publisher asks for a revocation of a test certificate (`superseded`) and shows that a new key signs the next release; the drill date is on the publisher's page. The hub runs its own drill on the Index intermediate yearly |
| T7 Key custody | human, at approval | The publisher states where the signing key lives (CI environment, release manager, hardware token); a key in a shared CI secret is accepted only with the provenance of T3 |
| T8 Review of permission widening | human | A release that widens permissions or capabilities is reviewed even where T2 would otherwise be lighter; the reviewer's note is shown ("Now asks for the microphone: the reviewer saw a voice-note feature") |

### Update rules (every level)
- U1 **Never silently wider.** The index marks `widened` when a version asks for more; the Store's update list shows the new permissions and asks before updating that app; automatic updates skip widened versions until the person answers.
- U2 **Major changes re-reviewed.** A new repository owner, a new signing certificate, a change of kind (plugin to app), or a rename: `flag` at Origin, held for a reviewer at Trusted.
- U3 **Downgrade protection.** The system installs from the hub only a version higher than the one installed; the index never lists a version lower than the highest ever listed for that id, except that a yanked highest version leaves the newest clean one as the current (so the publisher can go back, as the browser stores learned to allow); a rolled-back index (lower `serial`) is refused (0047). The one allowed downgrade: when the installed version is yanked, the Store offers the newest clean lower version with "Going back to 2.0: version 2.1 was withdrawn".
- U4 **Cooldown** as O4: Origin updates become available 72 hours after listing; Trusted at once.
- U5 **Yank is loud.** A yanked version installed somewhere shows one line in the Store and in the app's sheet with the reason in words and the version offered instead.

### Reporting and takedown
- R1 Anyone signed in (account 7 days old) reports; 5 a day. Categories as 0047. A malware report from a **trusted reporter** (a moderator-kept list of people and scanners whose reports held up; the registries' quarantine-on-trusted-word cut their response to hours) hides the listing at once and stops updates at the next `sign`; a malware report from anyone else puts the listing in "under review": hidden from search, still visible by direct link with a yellow line "Reported; being looked at". Three reports from different accounts count as one trusted report.
- R2 SLAs (clock from the report): malware or stolen key, a moderator looks within 4 hours in waking time and at most 24 hours; impersonation and licence, 3 days; broken, 7 days. Hide or yank takes effect at the next `sign` (within the hour). Revocation for `abuse` or `key-compromise`: two people, same day.
- R3 Every action is in the public `moderation_log`; the reporter's name is not. The publisher gets a mail with the reason and may answer in the same thread; a hidden listing comes back only through a moderator.
- R4 A security address for the hub itself (lead decision P5) answers within 2 working days; a found weakness in the hub, the checks or the trust crate is fixed in the open, with the finder named if they wish.

### What the hub refuses to host (listings, not just versions)
- Anything that runs code at install or needs network at install (F4, F5).
- Downloaders and launchers whose purpose is to fetch and run code the hub cannot see ("the real app comes later").
- Packages that bundle another store's client or an updater that bypasses the system's update path.
- Obfuscated code where the source tag does not contain the readable source of what is shipped.
- Look-alikes of system apps, of WolOS itself, of a listed app or of a known brand without being its owner (O6).
- Miners, adware, remote-control tools without a visible window and a consent screen, anything whose licence forbids redistribution, and anything illegal where the hub is run.
- Apps whose only content is a web view of a third party's site without that party's permission (extension of O6).

## The contested choice: how a Trusted release is reviewed [fresh]
Three real options, with the favourite's premortem. Numbers: at 200 Trusted publishers with 2 releases a month each, 400 releases a month, about 20 a working day.

| | A. A person reads every Trusted release before listing (favourite, the lead's order) | B. Sampled: every release that widens anything or fails a check, plus 1 in 5 of the rest, before listing; the others listed with "Not yet reviewed" | C. Automated: provenance + reproducible build + checks, no human; "Trusted" means "the publisher was vetted once" |
|---|---|---|---|
| What the badge means | The publisher was vetted and this version was read | The publisher was vetted and risky versions were read | The publisher was vetted |
| Cost at 400 releases a month | 20 reviews a working day, 10-30 minutes each: one person's job, or 4 volunteers at 1 hour a day | about 6 a day | near zero |
| Delay for the publisher | up to 2 working days | 0 for 80% | 0 |
| Would it have caught the researched cases | The worm waves (a new `prepare`-style hook, a new 2 MB file): yes, a diff of the file list shows it. The framework case (valid provenance, a new dependency pointing at a commit): yes, if the reviewer reads the manifest diff. The leaked-key pipeline case (three new files in a library folder): yes. A payload hidden in an existing binary blob: no | The same for widened or flagged releases; 1 in 5 for a silent swap of a binary with the same manifest | None of them: all had valid provenance from the real repository |
| Fails when | reviewers are tired, a release is large, the queue grows and the badge becomes a stamp | the attacker learns the sampling and keeps the manifest unchanged | the badge is read as "WolOS checked this" and it was not |

**Recommended: A**, as ordered, with two guards so that it stays honest: (1) a release the reviewer cannot finish in 30 minutes is listed at Origin level for that version with "Not reviewed yet", never silently stamped; (2) the weekly numbers (reviews done, median time, how many fell to Origin) go to the lead; if more than a fifth fall to Origin for a month, the lead chooses B or more reviewers. The reviewer's work is bounded by what the pipeline prepares: the diff of manifest and file list, the findings, the provenance match, the source tag link; they never download or run the app.

**Premortem of A** (it is 2028 and it failed; why):
1. The queue grew and reviewers stamped. Guard: the 30-minute rule and the weekly numbers; a stamp without a note on a widened release is refused by the Worker (T8 needs a sentence).
2. The payload was in a binary the reviewer could not read. True at every level; the badge text says "read", not "understood every byte", and F9 scans binaries; reproducible builds (T4) turn the binary into source for publishers who opt in. The ADR says this limit out loud on the badge's help page.
3. The reviewer was phished or their hub session stolen and used to approve. Guard: approvals need the reviewer's fresh GitHub sign-in (not the 7-day cookie) and are in the public log; two reviewers for a first release of a publisher and for widened releases.
4. A Trusted publisher's CI was taken over and the malicious release looked like every other. Guard: T3 ties provenance to one workflow on one protected branch, so the fork-and-orphan trick fails; the diff shows new files; the reviewer sees the source tag. If the attacker changes source on the protected branch, that is a repository compromise the publisher's own review should catch; the hub's badge is not a promise against it, and the revocation path (one hour online) is the backstop.
5. Trusted publishers left because 2 days was too slow. Guard: the number is on the dashboard; B is the fallback the lead can pick without a new record.
6. One reviewer was the lead and the hub stalled when they were away. Guard: at least three reviewers before the first Trusted publisher is approved (P3).

## Mapping: rule, enforcement point, owner
| Rule | Enforcement point | Owner |
|---|---|---|
| F1, F6, F9 content, F7 plugin allow-list | hubcheck rules (`size/*`, `manifest/*`, `files`, `content/*`, `permissions/*`) | HUB-wasm |
| F4 no code at install, F5 no network at install, F7 bundled runtime, O10 changes (`install/*`, `content/bundled-runtime`, `change/*`) | new hubcheck rules; `change/*` needs the previous release's manifest passed in by `check` | HUB-wasm (rules), HUB-pipe (passes the previous manifest) |
| F2 pinned asset, F8 version grows | Worker (first sha256 wins; refuses lower versions) and `check` (size and hash) | HUB-worker, HUB-pipe |
| F3/O7 tag on the linked repository, commit reachable from the default branch, no draft or pre-release | Worker webhook (records) and `sign` `verify-owners.mjs` (re-checks with the GitHub compare API) | HUB-worker, HUB-pipe |
| F10 timeout, not-checked | `check-release.mjs` | HUB-pipe |
| O1 two-factor, O2 account age, O3 repository age and activity, O5 rate limits, O6 name rules (refusals and the moderation hold) | Worker at sign-in, registration and publish | HUB-worker |
| O4 cooldown (7 days first listing, 24 hours updates) | `build-index.mjs` (`available_at` in the entry; the OS offers an update only after it) | HUB-pipe (index), Store (honours `available_at`) |
| O8 release statement | hubcheck verify + `check` | HUB-wasm, HUB-pipe |
| O9/T4 reproducible rebuild | new `rebuild` job in `check`, no secrets | HUB-pipe |
| T1 approval, T5 contact, T6 drill, T7 custody | Worker admin panel + `ca` workflow + public log | HUB-worker, HUB-pipe |
| T2, T8 review of every release and of widening | Worker moderation queue (`reviewed_by`, `note`), `build-index.mjs` refuses a Trusted entry without `reviewed_by` (lists it as Origin) | HUB-worker, HUB-pipe |
| T3 provenance pinned to workflow and branch | `check` verifies the provenance statement against the registered workflow and branch | HUB-pipe |
| T3b immutable releases, release workflow triggers and pinned actions | Worker reads the repository setting at registration and before each Trusted listing; `check` reads the workflow file at the tag | HUB-worker, HUB-pipe |
| R1 trusted reporters | Worker (`reporters` list kept by moderators; auto-hide) | HUB-worker |
| U1 widened asks, U3 downgrade protection, U4 cooldown, U5 yank line | Store and update-check (`widened`, `available_at`, `yanked`, highest version rule) | Store (shell, update-check) |
| U2 re-review | Worker (flags) and moderation queue | HUB-worker |
| R1-R3 reports, SLAs, log | Worker + moderation queue + `moderation_log` | HUB-worker |
| R4 hub security contact | lead | lead |
| Refusals list | hub rules page + moderators + hubcheck where mechanical (F4, F5, O6) | HUB-site (text), HUB-wasm, moderators |

## Store wording (binding)
- The word **"safe"** never appears, nor "secure", "verified safe", "malware-free", "protected". Allowed: "checked", "read", "reviewed", "found nothing to report", "found 2 things to look at".
- **Trusted**: "Made by Example Labs (examplelabs on GitHub). Trusted publisher: each release is read by WolOS before it is listed. Last reviewed 4 November." Help line: "Reading is not a guarantee. WolOS looks at what the release declares and what changed; it does not run it for you."
- **Origin**: "Made by Alice Doe (alice on GitHub). Origin checked: who published it and from which repository. WolOS has not reviewed what it does. Its automatic check found nothing to report." (or the findings).
- **Not reviewed yet** (a Trusted publisher's release waiting): the Origin line plus "A WolOS reviewer has not read this version yet."
- **Widened**: "This version asks for more than the one you have: Microphone, Background. Update anyway?" Buttons "Not now" (default) and "Update".
- **Cooldown**: "New version 2.1 is listed. It becomes available as an update on 7 November." For a first listing: "Listed on 11 November."
- **Going back**: "Version 2.1 was withdrawn by the hub: 'stole saved passwords'. Going back to 2.0, the newest version that was not withdrawn."
- **Reported**: "Someone reported this app on 4 November. A moderator is looking at it."
- **Yanked**: "Version 2.0 was withdrawn by the hub: 'stole saved passwords'. Version 2.1 is available."
- **Withdrawn** and **Not signed**: as 0046.

## Consequences
- Four new hubcheck rule families (`install/*`, `content/bundled-runtime`, `change/*`, provenance match) and one new job (`rebuild`) in the pipeline; the Worker learns account and repository age checks, name distance, the review queue with notes, and `available_at`; the index gains `widened`, `available_at`, `reviewed_by`, `provenance`; the Store honours three new fields.
- The hub is slower than a registry by design: a week for a stranger's first app, three days for an Origin update, two working days for a Trusted release. This is the cost of the order; it is said on the publish page in one sentence so nobody is surprised.
- Trusted needs people: at least three reviewers and one hour a day between them at launch scale. If the lead cannot staff it, the fallback is option B, not a quieter A.
- None of this makes an app harmless. The sandbox (0040) and the question sheet (0046) remain the last line; this record only decides what the hub lists and what it says.

## Lead decisions
- P1: approve option A (every Trusted release read) with the 30-minute and weekly-numbers guards, or choose B.
- P2: the numbers: account 90 days, repository 30 days and 5 commits, first-listing cooldown 7 days, update cooldown 72 hours (all recommended; 24 hours is the lighter choice and would have missed about a quarter of the researched cases).
- P3: the reviewers (at least three) and whether two are needed for a publisher's first release and for widened releases (recommended).
- P4: the reserved-word list and the edit-distance rule (2) for look-alike names.
- P5: the hub's security contact address and who answers it.
- P6: whether the reproducible rebuild job runs at launch or in a second step (recommended: second step, Origin bonus first, Trusted `block` once stable).
- P7: SLAs for reports (4 h waking / 24 h malware; 3 days impersonation; 7 days broken) and the "under review" hiding.
- P8: the exact badge and sheet sentences above.

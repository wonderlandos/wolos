# Crystal shards: the curated list and `wol shard`

Status: wave 3 CR-4 (libwopkg 1.1). Code: `crates/wopkg/src/index/shards.rs`, the C entry points
`wopkg_shard_index`, `wopkg_shard_pin`, `wopkg_shard_check` in `crates/wopkg/src/ffi.rs`
(`include/wopkg.h`, node `WOPKG_1.1`), `wopkg/src/wopkg/shard.cr` (`wopkg shard`),
`crates/controller/src/shard.rs` (`wol shard`). Rules: [RULES-crystal.md](../RULES-crystal.md)
RCR-040 to RCR-044.

## Why

A Crystal project names its dependencies in `shard.yml`, and `shards install` fetches them from
git. A version range or a tag can move, so the same project can build different code on two days
(RCR-040). WolOS answers with one signed list of shards it has read, each pinned to one full
commit, and with tools that write the pins and the lock for you. A build then runs
`shards install --frozen --local` and reads only what is already on the machine (RCR-044).

## Commands

| Command | Access | What it does |
|---|---|---|
| `wol shard list [words]` | read | The shards the list vouches for: name, version, licence, summary. |
| `wol shard add <shard> [dir]` | change | Adds the shard to `dependencies:` with `git:` and `commit:` from the list, writes `shard.lock` for it and every curated shard it needs, and fills the shards cache at those commits. |
| `wol shard update [shard] [dir]` | change | Downloads the list again and moves the named pins (all of them when none is named) to the commits it vouches for now; rewrites `shard.lock`. |
| `wol shard check [dir]` | read | RCR-040 and RCR-041 on the two files, and against the list: every dependency is curated, at the listed git URL and commit, and the lock names exactly the curated closure. Exit status 1 when anything is found. |

`dir` is the project folder (default: the current one). `--local` on `add` and `update` skips
the cache fetch (the files are still written); on `check` it skips the list and checks the files
only. `--json` prints libwopkg's documents unchanged. Off a terminal (a pipe, a script, `wol mcp`)
`wol shard` asks for that JSON itself, captures it and prints it, and the exit status still comes
back (1 when `check` found something), so a helper gets the findings and the failure together.
`wol shard` runs `wopkg shard` with the same words: nothing needs the administrator, and only the project's two files and the person's shards
cache change.

After `add`:

```yaml
dependencies:
  wolkit:
    git: https://git.wolos.org/shards/wolkit.git
    commit: 0123456789abcdef0123456789abcdef01234567
```

and `shard.lock` (the format `shards` 0.19 reads):

```yaml
version: 2.0
shards:
  wolkit:
    git: https://git.wolos.org/shards/wolkit.git
    version: 0.3.0+git.commit.0123456789abcdef0123456789abcdef01234567
```

Other keys of the dependency (`version:`, `branch:`, `tag:`, `github:`) are dropped, and the rest
of `shard.yml` stays as it was. A dependency the list does not carry makes `add` and `update`
refuse: the lock has to name the whole tree, and the tree is curated.

## The offline install

`shards install --frozen --local` installs exactly the lock and reads only its cache: one git
mirror per shard under `$SHARDS_CACHE_PATH`, else `$XDG_CACHE_HOME/shards`, else
`~/.cache/shards`, at `<host>/<path>.git`. `add` and `update` make that mirror (`git clone
--mirror`, then `git fetch`) and check that it holds the pinned commit; a commit no branch carries
any more is fetched by its hash and kept under `refs/wolos/pinned/<commit>` so git does not prune
it. When a commit cannot be had, the files are written and the command fails with the shard's
name, so the next online `wol shard update` finishes the job.

A package build (PKGBUILD) does not fetch at all: the shard arrives in `source=(...)` with a real
checksum and is built with `shards install --frozen --local` (RCR-044).

## The list

`https://shards.wolos.org/index.json` and `index.json.sig`, published from the `wolos-shards`
repository (layout and generator: [wolos-shards/README.md](../../wolos-shards/README.md)).

```json
{"format":1,"serial":7,"expires":1767225600,"shards":[
  {"name":"wolkit","git":"https://git.wolos.org/shards/wolkit.git","version":"0.3.0",
   "commit":"<40 hex>","license":"BSD-2-Clause","summary":"Reactive UI on libWolkit",
   "crystal":">= 1.22.0","dependencies":["cryswol"]}]}
```

| Field | Rule |
|---|---|
| `name` | lowercase letter, then lowercase letters, digits, `-`, `_`; at most 50 |
| `git` | `https://host/path`; no user part, query, fragment, space, quote, `.` or `..` segment |
| `version` | digit first, then letters, digits, `.`, `-` (the lock adds `+git.commit.<sha>`) |
| `commit` | 40 lowercase hex digits |
| `license` | not empty, at most 64 characters |
| `summary`, `crystal` | no control characters; at most 200 and 64 |
| `dependencies` | names of other entries of the same list, at most 32 |

An entry that breaks a rule is dropped on its own, as is a second entry with the same name.

Trust, in order (the same code paths as the plugin list, `index/plugins.rs`):

1. The Ed25519 signature over the exact bytes is checked before anything is parsed. The keys are
   `*.pub` files in `/usr/share/wolos/keys/shards/` (with its own `revoked.list`). They are kept
   apart from the package and release keys in `/usr/share/wolos/keys/`: a key read for one
   purpose never vouches for another.
2. A list with a lower `serial` than the cached one is refused (replay), and so is a list past its
   `expires`.
3. The cache (`~/.cache/wopkg/shard-index/`) is verified again on every read, and replaced only by
   a list that passed 1 and 2 (temp file, then rename).

### How the key is made and rotated

The shard list has its own Ed25519 key, a trust root apart from the package and release keys.

- **Made** in the publish pipeline, on the release owner's offline machine, never on a CI runner:
  `openssl genpkey -algorithm ed25519 -out shards.key.pem`. The private key stays offline; it signs
  `index.json` with `crystal run tools/gen_index.cr -- sign shards.key.pem` in `wolos-shards`
  (`openssl pkeyutl -sign -rawin`, 64 raw bytes in `index.json.sig`).
- **Shipped** as `openssl pkey -in shards.key.pem -pubout > shards-<year>.pub` (PEM, SubjectPublicKeyInfo) in
  `/usr/share/wolos/keys/shards/` of the system image. Every `*.pub` there is a trusted shard key;
  nothing else is.
- **Rotated** in two releases: the first ships the new `.pub` beside the old one and the list is
  signed with the new key from then on; a later release removes the old file.
- **Withdrawn** at once by a line in `/usr/share/wolos/keys/shards/revoked.list` (the same
  format as the package keys' list: 64 hex characters of the key, a space, the reason); a revoked
  key verifies nothing, even while its file is still there.

A list signed by a package or release key is refused, and the shard key never verifies a package,
a release or the plugin list.

`list`, `add` and `check` use the cached list while it is valid and download it when there is
none; `update` always downloads, and falls back to a valid cache when the download fails.

## C entry points (libwopkg 1.1)

All three take a JSON request and return a JSON document (`wopkg_string`), like `wopkg_verify`.

| Function | Request | Result |
|---|---|---|
| `wopkg_shard_index` | `{"fresh":bool,"term":str,"url":str}` | `{"shards":[entry],"serial","expires"}` |
| `wopkg_shard_pin` | `{"dir":path,"names":[..],"update":bool,"fresh":bool,"url":str}` | `{"changed":[{name,from,to,version}],"locked":[{name,git,commit,version}],"shard_yml","shard_lock"}` |
| `wopkg_shard_check` | `{"dir":path,"offline":bool,"url":str}` | `{"ok":bool,"index":bool,"findings":[{rule,line,file,message}]}` |

`url` is another signed list for the same keys (empty: the default). Errors: `NOT_FOUND` for a
shard the list does not carry or a missing `shard.yml`, `UNTRUSTED` for a list that does not
verify, is older or expired, or a curated shard that needs one the list lacks, `UNAVAILABLE` when
no list can be had, `INVALID_ARGUMENT` for a bad request or name.

## Tests

- `cargo test -p wopkg --lib index::shards`: signature, entry rules, bounds, closure, the
  `shard.yml` edits, lock text, check findings, keys folder, cache and replay; and the signed
  fixture the Crystal specs use (`wopkg/spec/fixtures/shards`, test-only key; `WOL_BLESS=1`
  writes it again).
- `cargo test -p wopkg --test abi --test c_smoke`: the 1.1 node, header and snapshot.
- `cargo test -p controller --test shard`: argv mapping, exit status, catalogue access, and the
  app template after `wopkg shard add` passing the RCR-040/041 scanners (needs the built CLI).
- `make spec` in `wopkg/`: `spec/shard_spec.cr`, including a real `shards install --frozen
  --local` from a primed cache whose remote is gone (when git and shards are installed).

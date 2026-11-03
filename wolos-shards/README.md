# wolos-shards: the curated Crystal shard list

This folder is the layout of the `wolos-shards` repository. It is kept in the OS repository until
that repository is created. It publishes the signed list that `wol shard add|update|list|check`
reads ([docs/modules/shards.md](../docs/modules/shards.md)). A shard is on the list when a person
has read it at one commit, and the list carries that commit and nothing newer.

## Layout

| Path | What |
|---|---|
| `curated.yml` | The reviewed shards: `name`, `git` (the published `https://` URL), `commit` (40 hex), `summary`, optional `fetch` (another place to read the same commit) and `path` (the shard's folder inside `fetch`, for a shard still in the WolOS tree). |
| `tools/gen_index.cr` | The generator: `check`, `build [days]`, `sign <key.pem>` (standard library only). |
| `.github/workflows/check.yml` | CI: format check and `gen_index.cr check` on every pull request. |
| `public/` | What gets published, never committed: `index.json` and `index.json.sig`. |

## Adding a shard

1. Read the shard at the commit you propose: its code, its `shard.yml`, its build steps, and every
   shard it depends on. A dependency has to be on the list already, pinned in the shard's own
   `shard.yml` by `commit:` at the commit the list has.
2. Add the entry to `curated.yml`. The pull request says what was read and why it can be trusted.
3. CI runs `crystal run tools/gen_index.cr -- check`. It clones each shard, reads `shard.yml` at
   the commit, and refuses the list on any of these: a name that is not a shard name or differs
   from the shard's own, a URL that is not plain `https://`, a commit that is not 40 hex digits or
   is not there, a version that is not plain, no licence, or a dependency that is not curated or
   is at another commit.
4. Moving a shard to a newer commit is the same review over the difference.

`version`, `license`, `crystal` and `dependencies` are never typed by hand: the generator copies
them from the shard's own `shard.yml` at the commit.

## Shards still in the WolOS tree

The first curated shards, `cryswol` and `wolkit`, are developed inside the WolOS source tree
(`wolos-crystal/shards/`), so their rows carry two more keys: `fetch:` (the WolOS repository) and
`path:` (the shard's folder in it). `check` reads `<path>/shard.yml` at the commit from there, with
every other rule unchanged. `build` refuses to publish while any row has `path:`: the list promises
that `git` holds `commit`, and the standalone repository does not exist yet.

The move, once per shard: split the folder's history into its own repository at the `git` URL
(the commit hashes change), review the difference to the tree at the old commit (none is expected),
put the new commit in `commit`, and delete `fetch` and `path`. Until then the `commit` of both rows here and the `cryswol` pin in `wolos-crystal/shards/wolkit/shard.yml` (with its `shard.lock`) move together, because `check` refuses the list if they differ: `tools/crystal/pin-shards.sh <commit>` moves them; the release runbook runs it before the wolos content commit, so the pin names the previous release commit. Shards that depend on it move their
`commit:` pins in the same pull request (`check` refuses a pin that differs from the list).

## Publishing (release owner, offline key)

```sh
crystal run tools/gen_index.cr -- build 30           # serial = last published + 1, expires in 30 days
crystal run tools/gen_index.cr -- sign shards.key.pem
# upload public/index.json and public/index.json.sig to https://shards.wolos.org/
```

Keep the last published `public/index.json` next to the generator before `build`: the serial
continues from it, and a client refuses a list whose serial is lower than the one it has seen.
Publish again before `expires` (a list past it is refused, so a frozen mirror cannot pin people
to an old list). The signing key is an Ed25519 key that never sits on a CI runner:

```sh
openssl genpkey -algorithm ed25519 -out shards.key.pem
openssl pkey -in shards.key.pem -pubout > shards-<year>.pub   # ships in /usr/share/wolos/keys/shards/
```

The public key ships in its own folder, `/usr/share/wolos/keys/shards/`, apart from the package and
release keys. A key is withdrawn by its line in `/usr/share/wolos/keys/shards/revoked.list` (the
same format as the package keys' list). Rotation: ship the new `.pub` beside the old one, sign
with the new key, and remove the old file in a later release.

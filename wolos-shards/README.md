# wolos-shards: the curated Crystal shard list

This folder is the layout of the `wolos-shards` repository. It is kept in the OS repository until
that repository is created. It publishes the signed list that `wol shard add|update|list|check`
reads ([docs/modules/shards.md](../docs/modules/shards.md)). A shard is on the list when a person
has read it at one commit, and the list carries that commit and nothing newer.

## Layout

| Path | What |
|---|---|
| `curated.yml` | The reviewed shards: `name`, `git` (the published `https://` URL), `commit` (40 hex), `summary`, optional `fetch` (another place to read the same commit). |
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

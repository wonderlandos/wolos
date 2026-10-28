#!/usr/bin/env bash
# Build [wolos-cachy] (ADR 0019): x86-64-v3 builds of the image's packages, frozen at the release.
# Roots = image/arch/packages.x86_64 + desktop/packages.txt + image/repo/cachy-extra; their
# dependency closure is resolved in the Arch repos at image/arch/snapshot. A package is taken from
# the upstream x86-64-v3 repos (cachyos-v3, cachyos-core-v3, cachyos-extra-v3, first hit wins)
# only when its version matches the Arch snapshot's (same upstream version, another build);
# otherwise the Arch package stays. An epoch ':' in a file name becomes '.' (release assets cannot
# hold ':'; pacman downloads the database's file name, the signature covers the content). Each file is
# checked against the database sha256 and its upstream signature, then goes into
# wolos-cachy.db signed with the WolOS key. Packages keep their upstream signatures.
# Resumable: databases and verified packages already in $OUT are kept. Deterministic: the same
# databases give the same selection (selected.tsv) and revision.
# Output: $OUT/x86_64/{wolos-cachy.db,*.pkg.tar.zst,*.sig}, $OUT/selected.tsv, $OUT/sizes.txt,
#         $OUT/revision. Size report on stdout.
# Env: WOLOS_CACHY_DIR (OUT, default $WOLOS_WORK/cachy), WOLOS_CACHY_PKGS="a b" (roots instead of
#      the lists), WOLOS_CACHY_PLAN=1 (select and report sizes, download no package),
#      WOLOS_CACHY_URL (upstream server, $repo; default the x86_64_v3 mirror),
#      WOLOS_ARCH_URL (Arch server, $repo; default the archive at the snapshot),
#      WOLOS_CACHY_KEY (upstream keyring, default /usr/share/pacman/keyrings/cachyos.gpg),
#      WOLOS_CACHY_TRUST (trusted fingerprints, default image/repo/cachy-trusted),
#      WOLOS_CACHY_NOVERIFY=1 (tests: sha256 only), WOLOS_SIGN_KEY | WOLOS_UNSIGNED=1 as build.sh,
#      WOLOS_CACHY_REFRESH=1 (download the databases again), WOLOS_REPO_ADD (tests).
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd); IMG=$(dirname "$HERE"); REPO=$(dirname "$IMG")
WORK=${WOLOS_WORK:-${WOLOS_VM:-$HOME/WolOS-vm}/build}
OUT=${WOLOS_CACHY_DIR:-$WORK/cachy}
SNAP=$(tr -d '[:space:]' < "$IMG/arch/snapshot")
ARCH_URL=${WOLOS_ARCH_URL:-https://archive.archlinux.org/repos/${SNAP//-//}/\$repo/os/x86_64}
CACHY_URL=${WOLOS_CACHY_URL:-https://mirror.cachyos.org/repo/x86_64_v3/\$repo}
ARCH_REPOS="core extra multilib"
CACHY_REPOS="cachyos-v3 cachyos-core-v3 cachyos-extra-v3"
PLAN=${WOLOS_CACHY_PLAN:-0}
KEY=${WOLOS_SIGN_KEY:-}
if [ "$PLAN" != 1 ] && [ -z "$KEY" ] && [ "${WOLOS_UNSIGNED:-0}" != 1 ]; then
  echo "mirror-cachy.sh: set WOLOS_SIGN_KEY=<gpg key id> (or WOLOS_UNSIGNED=1 for a local test repo)" >&2
  exit 2
fi
PKG=$OUT/x86_64; DB=$OUT/dbs
mkdir -p "$PKG" "$DB"
say() { echo "mirror-cachy: $*" >&2; }
fetch() { # URL FILE: download once (resumable through .part), atomically
  [ -s "$2" ] && return 0
  curl -fsSL --retry 3 -C - -o "$2.part" "$1" && mv "$2.part" "$2"
}
url() { local u=${1//\$repo/$2}; echo "${u%/}/$3"; }

# repo name version filename csize sha256 depends provides (comma lists), one line per package.
index() {
  { bsdtar -xOf "$2" '*/desc' 2>/dev/null || :; } | awk -v repo="$1" '
    function flush() { if (n != "") print repo "\t" n "\t" v "\t" f "\t" c "\t" h "\t" d "\t" p; n = v = f = c = h = d = p = "" }
    /^%FILENAME%$/ { flush(); k = "F"; next }
    /^%[A-Z0-9]+%$/ { k = $0; next }
    /^$/ { k = ""; next }
    k == "F" { f = $0 } k == "%NAME%" { n = $0 } k == "%VERSION%" { v = $0 }
    k == "%CSIZE%" { c = $0 } k == "%SHA256SUM%" { h = $0 }
    k == "%DEPENDS%" { d = d (d == "" ? "" : ",") $0 } k == "%PROVIDES%" { p = p (p == "" ? "" : ",") $0 }
    END { flush() }'
}

# 1. Databases (small; kept unless WOLOS_CACHY_REFRESH=1).
[ "${WOLOS_CACHY_REFRESH:-0}" = 1 ] && rm -f "$DB"/*.db
: > "$DB/arch.tsv"; : > "$DB/cachy.tsv"
for r in $ARCH_REPOS; do
  fetch "$(url "$ARCH_URL" "$r" "$r.db")" "$DB/$r.db" || { say "no Arch database $r"; exit 1; }
  index "$r" "$DB/$r.db" >> "$DB/arch.tsv"
done
for r in $CACHY_REPOS; do
  fetch "$(url "$CACHY_URL" "$r" "$r.db")" "$DB/$r.db" || { say "no upstream database $r"; exit 1; }
  index "$r" "$DB/$r.db" >> "$DB/cachy.tsv"
done

# 2. Roots and their closure in the Arch snapshot (names or provides; version constraints dropped).
if [ -n "${WOLOS_CACHY_PKGS:-}" ]; then
  roots=$WOLOS_CACHY_PKGS
else
  roots=$(cat "$IMG/arch/packages.x86_64" "$REPO/desktop/packages.txt" "$HERE/cachy-extra" 2>/dev/null |
    sed 's/#.*//' | tr -s ' \t' '\n' | grep . | sort -u | tr '\n' ' ')
fi
awk -F'\t' -v roots="$roots" '
  function base(s) { sub(/[<>=].*/, "", s); return s }
  function want(s,  t) { s = base(s); t = (s in pkg) ? s : prov[s]; if (t != "" && !(t in seen)) { seen[t] = 1; q[++qn] = t } }
  { if (!($2 in pkg)) { pkg[$2] = 1; dep[$2] = $7 }
    np = split($8, pp, ","); for (i = 1; i <= np; i++) { s = base(pp[i]); if (s != "" && !(s in prov)) prov[s] = $2 } }
  END {
    nr = split(roots, r, " "); for (i = 1; i <= nr; i++) want(r[i])
    for (j = 1; j <= qn; j++) { nd = split(dep[q[j]], dd, ","); for (i = 1; i <= nd; i++) want(dd[i]) }
    for (j = 1; j <= qn; j++) print q[j]
  }' "$DB/arch.tsv" | LC_ALL=C sort > "$DB/closure"

# 3. Selection: same upstream version (epoch:pkgver, pkgrel ignored).
LC_ALL=C awk -F'\t' -v OFS='\t' '
  function up(v) { sub(/-[^-]*$/, "", v); return v }
  FILENAME == ARGV[1] { if (!($2 in av)) av[$2] = $3; next }
  FILENAME == ARGV[2] { if (!($2 in cl)) cl[$2] = $0; next }
  { total++; n = $1; if (!(n in cl)) { nocachy++; next }
    split(cl[n], c, "\t")
    if (up(c[3]) != up(av[n])) { skew++; print "skew", n, av[n], c[3] > "/dev/stderr"; next }
    print n, c[1], av[n], c[3], c[4], c[5], c[6] }
  END { printf "closure %d, upstream build %d, version skew %d\n", total, total - nocachy, skew > "/dev/stderr" }
' "$DB/arch.tsv" "$DB/cachy.tsv" "$DB/closure" 2> "$DB/select.log" | LC_ALL=C sort > "$OUT/selected.tsv" || true
grep -v '^skew' "$DB/select.log" | sed 's/^/mirror-cachy: /' >&2
say "version skew (Arch kept): $(grep -c '^skew' "$DB/select.log" || true)"

# 4. Download + verify (skipped when PLAN=1). Stale files of an older selection are removed.
gh=
if [ "$PLAN" != 1 ] && [ "${WOLOS_CACHY_NOVERIFY:-0}" != 1 ]; then
  ring=${WOLOS_CACHY_KEY:-/usr/share/pacman/keyrings/cachyos.gpg}
  trust=${WOLOS_CACHY_TRUST:-$HERE/cachy-trusted}
  [ -s "$ring" ] || { say "no upstream keyring $ring (WOLOS_CACHY_KEY)"; exit 1; }
  gh=$(mktemp -d); trap 'rm -rf "$gh"' EXIT
  gpg --homedir "$gh" --batch --quiet --import "$ring" 2>/dev/null
fi
verify_sig() { # FILE: valid signature by a trusted fingerprint
  [ -z "$gh" ] && return 0
  local st; st=$(gpg --homedir "$gh" --batch --status-fd 1 --verify "$1.sig" "$1" 2>/dev/null) || return 1
  local f; for f in $(sed 's/[:#].*//' "$trust"); do
    printf '%s\n' "$st" | grep -q "^\[GNUPG:\] VALIDSIG .*$f" && return 0
  done
  return 1
}
if [ "$PLAN" != 1 ]; then
  for f in "$PKG"/*.pkg.tar.zst; do
    [ -e "$f" ] || continue
    cut -f5 "$OUT/selected.tsv" | tr : . | grep -qxF "$(basename "$f")" || rm -f "$f" "$f.sig"
  done
  while IFS=$'\t' read -r n r _ _ file _ sum; do
    f=$PKG/${file//:/.}
    # Kept from an earlier run when it still matches the database (its .sig is small: fetched again).
    [ -s "$f" ] && [ "$(sha256sum "$f" | cut -d' ' -f1)" = "$sum" ] || rm -f "$f" "$f.sig"
    fetch "$(url "$CACHY_URL" "$r" "$file")" "$f" && fetch "$(url "$CACHY_URL" "$r" "$file.sig")" "$f.sig" ||
      { say "download failed: $n"; exit 1; }
    [ "$(sha256sum "$f" | cut -d' ' -f1)" = "$sum" ] || { rm -f "$f" "$f.sig"; say "sha256 mismatch: $n"; exit 1; }
    verify_sig "$f" || { rm -f "$f" "$f.sig"; say "bad signature: $n"; exit 1; }
  done < "$OUT/selected.tsv"
  # 5. The database, signed with the WolOS key.
  rm -f "$PKG"/wolos-cachy.db* "$PKG"/wolos-cachy.files*
  if [ -s "$OUT/selected.tsv" ]; then
    (cd "$PKG" && cut -f5 "$OUT/selected.tsv" | tr : . | xargs "${WOLOS_REPO_ADD:-repo-add}" -q ${KEY:+--sign --key "$KEY"} wolos-cachy.db.tar.zst)
  fi
fi
echo "${SNAP//-/}-$(sha256sum "$OUT/selected.tsv" | cut -c1-8)" > "$OUT/revision"

# 6. Size report. GitHub release assets: at most 2 GiB each and 1000 per release; the signatures
# travel inside the database (repo-add embeds each .sig), so the assets are the packages + 4 db files.
awk -F'\t' -v plan="$PLAN" -v rev="$(cat "$OUT/revision")" '
  { t += $6; if ($6 > big) { big = $6; bn = $5 } if ($6 >= 2147483648) over = over " " $5 }
  END { printf "revision %s\n%s %d packages, %.1f MiB\nassets %d (limit 1000)\nlargest %s (%.1f MiB)\nover 2 GiB:%s\n", rev,
          (plan == 1 ? "estimated" : "mirrored"), NR, t / 1048576, NR + 4, (bn == "" ? "-" : bn), big / 1048576, (over == "" ? " none" : over) }
' "$OUT/selected.tsv" | tee "$OUT/sizes.txt"

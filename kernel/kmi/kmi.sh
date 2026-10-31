#!/bin/sh
# WolKernel KMI: freeze exported symbols + CRCs per release line (like Android GKI).
#   kmi.sh extract O        > wolkernel-<line>.symvers   (at the first release of a line)
#   kmi.sh check O BASELINE                               (every build of that line; exit 1 = KMI break)
# O = kernel build dir with Module.symvers (CONFIG_MODVERSIONS=y). Lines: CRC<TAB>symbol<TAB>module<TAB>type.
set -eu
D=$(cd "$(dirname "$0")" && pwd)
frozen() {
  mods=$(sed -e 's/#.*//' -e 's/[[:space:]].*//' -e '/^$/d' "$D/modules.list")
  awk -F'\t' -v m="vmlinux $mods" 'BEGIN{n=split(m,a," ");for(i=1;i<=n;i++)K[a[i]]=1} ($3 in K)' "$1/Module.symvers" | sort -k2,2
}
case "${1:-}" in
extract) frozen "$2" ;;
check)
  [ -f "$3" ] || { echo "kmi: no baseline $3" >&2; exit 2; }
  frozen "$2" > "${TMPDIR:-/tmp}/kmi.$$"
  awk -F'\t' 'NR==FNR{C[$2]=$1;next} !($2 in C){print "KMI removed: "$2" ("$3")";bad=1;next}
    C[$2]!=$1{print "KMI changed: "$2" ("$3") "$1" -> "C[$2];bad=1} END{exit bad}' \
    "${TMPDIR:-/tmp}/kmi.$$" "$3" && r=0 || r=1
  echo "kmi: $(wc -l < "$3") frozen, $(($(wc -l < "${TMPDIR:-/tmp}/kmi.$$") - $(wc -l < "$3"))) added"
  rm -f "${TMPDIR:-/tmp}/kmi.$$"; exit $r ;;
*) sed -n 2,5p "$0"; exit 2 ;;
esac

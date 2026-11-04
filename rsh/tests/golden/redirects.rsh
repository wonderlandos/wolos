#!/usr/bin/rsh
# SPDX-License-Identifier: GPL-2.0-or-later
# Redirects: > >> < 2> 2>> 2>&1 &> <<< and a path given as one value.
echo first > out.txt
echo second >> out.txt
cat < out.txt
sh -c 'echo to-out; echo to-err >&2' > o.txt 2> e.txt
echo "o.txt: #{File.read("o.txt").strip}"
echo "e.txt: #{File.read("e.txt").strip}"
sh -c 'echo more-err >&2' 2>> e.txt
cat e.txt
sh -c 'echo merged-out; echo merged-err >&2' 2>&1 | sort
sh -c 'echo both; echo err >&2' &> both.txt
sort both.txt
dest = "name with spaces.txt"
echo into a path value > #{dest}
cat #{dest}
cat <<< "here string #{dest}"
cat /no/such/file 2> /dev/null || echo "silenced"
wc -c < out.txt

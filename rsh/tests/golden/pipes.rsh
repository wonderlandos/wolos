#!/usr/bin/rsh
# SPDX-License-Identifier: GPL-2.0-or-later
# Pipelines: pipefail status, Ruby stages, |&, redirects, a here-document at the end.
$ printf 'b\na\nc\na\n' | sort | uniq -c | sed 's/^ *//'
$ printf 'one\ntwo\nthree\n' | { |l| l.upcase if l.size > 3 } | cat
$ printf 'x\ny\n' | { |l| "#{l}!" }
seq 1 5 | { |l| l.to_i.even? } | paste -sd+ -
yes | head -2
sh -c 'echo to-err >&2' |& cat
ok = (sh -c 'exit 3' | cat)
puts "pipefail: #{ok.inspect} #{$?.exitstatus} #{$?.statuses.inspect}"
echo first > out.txt
echo second >> out.txt
cat < out.txt
sh -c 'echo both; echo err >&2' &> both.txt
cat both.txt
ls /no/such 2>/dev/null || echo "silenced"
who = "here-doc"
cat <<EOF
  plain #{who} $HOME
EOF

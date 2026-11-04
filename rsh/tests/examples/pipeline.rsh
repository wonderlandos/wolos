#!/usr/bin/rsh
# SPDX-License-Identifier: GPL-2.0-or-later
# pipeline: a Ruby block is one stage of a pipeline; map, filter and count on the way.
seq 1 12 | { |l| n = l.to_i; "#{n}:#{n * n}" if n.odd? } | paste -sd' ' -

words = %w[kiwi apple fig banana cherry]
printf_out = `printf '%s\n' #{words}`
puts printf_out.lines.map(&:strip).select { |w| w.size > 3 }.map(&:capitalize).join(",")

echo #{words} | tr ' ' '\n' | { |w| w.upcase unless w.start_with?("f") } | sort -r | head -3

total = 0
seq 1 5 | { |l| total += l.to_i; nil }
puts "sum of 1..5 through a stage: #{total}"
words.each_slice(2) { |pair| echo pair: #{pair} }

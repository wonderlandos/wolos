#!/usr/bin/rsh
# SPDX-License-Identifier: GPL-2.0-or-later
# && and || inside a command line, and a command line as a condition and as a value.
test -d / && echo "and: ran"
test -e /no/such && echo "and: not shown"
test -e /no/such || echo "or: ran"
test -d / || echo "or: not shown"
test -e /no/such && echo x || echo "and-or: else branch"
test -d / && echo "a" && echo "b"
echo one; echo two
if test -d / && test -e /etc
  puts "if with &&"
end
unless test -e /no/such || test -e /also/no
  puts "unless with ||"
end
ok = (test -d /)
bad = (test -e /no/such)
puts "values: #{ok.inspect} #{bad.inspect} #{$?.exitstatus}"
n = 0
while test #{n} -lt 3
  n += 1
end
puts "loop: #{n}"
sh -c "exit 7" || echo "or after a failure"

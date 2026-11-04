#!/usr/bin/rsh
# SPDX-License-Identifier: GPL-2.0-or-later
# logscan: count the errors per service in a log, show the worst three.
log = <<~TEXT
  Nov 03 10:00:01 web error: connection reset
  Nov 03 10:00:02 db  info: ready
  Nov 03 10:00:03 web error: timeout
  Nov 03 10:00:04 cache error: full
  Nov 03 10:00:05 web error: timeout
  Nov 03 10:00:06 db  error: deadlock
  Nov 03 10:00:07 web info: ok
  Nov 03 10:00:08 db  error: deadlock
TEXT
File.write("app.log", log)

found = (grep -q error app.log)
errors = `grep error app.log`.lines
puts "#{errors.size} error lines (any found: #{found})"

grep error app.log | { |l| l.split[3] } | sort | uniq -c | sort -rn | head -3

timeouts = errors.select { |l| l =~ /timeout|deadlock/ }.map { |l| l.split[3] }.uniq.sort
puts "services with timeouts or deadlocks: #{timeouts.join(', ')}"
if grep -q fatal app.log
  puts "fatal found"
else
  puts "no fatal lines"
end

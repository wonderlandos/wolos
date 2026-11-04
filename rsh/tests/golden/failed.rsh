#!/usr/bin/rsh
# SPDX-License-Identifier: GPL-2.0-or-later
# A failing command line raises Rsh::Failed unless it is a condition, left of || or &&, or the
# script said relaxed!. $? is set either way.
begin
  sh -c 'echo oops >&2; exit 3'
rescue Rsh::Failed => e
  puts "caught: #{e.message}"
  puts "status #{e.status.exitstatus}, argv #{e.argv.inspect}"
  puts "stderr tail: #{e.stderr.strip}"
end
cond = (test -e /no/such)
puts "condition: #{cond.inspect}"
sh -c 'exit 2' || echo "or branch ran"
relaxed!
sh -c 'exit 5'
puts "relaxed, status #{$?.exitstatus}"
relaxed!(false)
begin
  sh -c 'exit 1' | cat
rescue Rsh::Failed => e
  puts "pipeline: #{e.message}"
end
begin
  nonexistent-program-x
rescue Rsh::Failed => e
  puts "missing: #{e.status.exitstatus}"
end
puts "uncaught next"
sh -c 'exit 9'
puts "not reached"

#!/usr/bin/rsh
# SPDX-License-Identifier: GPL-2.0-or-later
# remote: run the same check on several computers, report which ones failed.
# Usage: remote.rsh HOST...
hosts = ARGV.empty? ? %w[alpha beta down] : ARGV
failed = []
hosts.each do |host|
  begin
    ssh -o BatchMode=yes #{host} echo hello from "#{host}"
    ssh -T #{host} <<~SCRIPT
      echo "script on #{host}"
    SCRIPT
  rescue Rsh::Failed => e
    puts "#{host}: failed (#{e.status.exitstatus}) #{e.stderr.strip}"
    failed << host
  end
end
puts failed.empty? ? "all hosts ok" : "failed: #{failed.join(', ')}"
exit(failed.empty? ? 0 : 1)

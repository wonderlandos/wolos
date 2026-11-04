#!/usr/bin/rsh
# SPDX-License-Identifier: GPL-2.0-or-later
# backup: pack a directory into a dated archive, check it, keep the newest three.
# Usage: backup.rsh [DIRECTORY]
src = ARGV[0] || "data"
stamp = "2026-11-%02d"
mkdir -p #{src} archive
echo alpha > #{src}/one.txt
echo beta > #{src}/two.txt
5.times do |i|
  name = "archive/#{src}-#{stamp % (i + 1)}.tar"
  tar -cf #{name} #{src}
end
listing = `tar -tf archive/#{src}-2026-11-05.tar`.lines.map(&:strip).sort
puts "archive holds: #{listing.join(', ')}"
old = Dir.glob("archive/*.tar").sort[0...-3]
rm -f #{old}
puts "kept: #{Dir.glob("archive/*.tar").sort.join(' ')}"
begin
  tar -tf archive/missing.tar 2> /dev/null
rescue Rsh::Failed => e
  puts "check failed: #{e.message}"
end

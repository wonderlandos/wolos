#!/usr/bin/rsh
# SPDX-License-Identifier: GPL-2.0-or-later
# Globbing: * ? [] {a,b} **, quoted text is never expanded, no match is an error.
touch a.txt b.txt c.log "star*.txt"
mkdir -p d/e
touch d/e/f.txt d/g.txt
echo *.txt
echo *.{txt,log}
echo [ab].txt
echo ?.log
echo **/*.txt
echo "*.txt" '*.log'
echo ~/x
home = "*.txt"
echo #{home}
begin
  echo *.zzz
rescue Rsh::Failed, Rsh::Error => e
  puts "no match: #{e.class}"
end
Rsh.nullglob = true
echo none:*.zzz:
echo x{1,2}y {a,b}{1,2}

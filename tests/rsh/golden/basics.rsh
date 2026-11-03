#!/usr/bin/rsh
# SPDX-License-Identifier: GPL-2.0-or-later
# Values go in as whole arguments; arrays as one argument per element.
name = "a b; echo injected *"
$ printf "<%s>\n" #{name}
list = ["one", "two words", ""]
$ printf "[%s]\n" #{list}
echo pre-#{name.size}-post
export RSH_GOLDEN="x y"
sh -c 'echo "child: $RSH_GOLDEN"'
ENV["RSH_GOLDEN"] = "from ruby"
echo $RSH_GOLDEN
echo "quoted $RSH_GOLDEN and #{1 + 1}" 'single $RSH_GOLDEN #{x}'
LANG=C sh -c 'echo "prefix: $LANG"'
n = `printf 'a\nb\nc\n'`.lines.size
puts "lines: #{n}"
if grep -q root /etc/passwd
  puts "condition true"
end
unless test -e /no/such/file
  puts "unless works"
end
ok = (test -d /)
puts "value: #{ok} #{$?.exitstatus}"
test -e /no/such/file || echo "or ran"
test -d / && echo "and ran"
echo x{1,2}y
puts ARGV.inspect

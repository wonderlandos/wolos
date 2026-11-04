#!/usr/bin/rsh
# SPDX-License-Identifier: GPL-2.0-or-later
# How each line is read: R Ruby, C command line, X a capture inside Ruby (see classify.explain).
name = "world"
puts "hello #{name}"
echo hello #{name}
$ echo forced command
$ echo also forced
ls -d /
def never
  ./local-tool --flag
  ~/bin/tool arg
end
def helper(x)
  x * 2
end
helper 21
helper(21)
nothing = helper 4
if grep -q root /etc/passwd
  puts "yes"
else
  puts "no"
end
while test -e /no/such/file
  puts "never"
end
[1, 2].each do |n|
  echo item #{n}
end
[3].each { |n| echo brace #{n} }
cd /tmp do
  pwd
end
files = `ls /`.lines
n = `echo a`.size + 1
x = 5
x += 1
x
printf("%s\n", "ruby printf")
sleep 0
test -d / && echo and
test -d / || echo or
echo a; echo b
echo trailing # a comment
echo "quote # not a comment"
Struct.new(:a)
exit 0

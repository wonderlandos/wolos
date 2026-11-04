#!/usr/bin/ruby
# SPDX-License-Identifier: GPL-2.0-or-later
# Drives rsh through a pseudo-terminal: editing, completion, jobs, multi-line input, history search.
# Usage: ruby tests/interactive.rb path/to/rsh
# frozen_string_literal: true

require 'pty'
require 'io/console'
require 'tmpdir'

rsh = File.expand_path(ARGV[0] || File.join(__dir__, '../build/rsh'))
home = Dir.mktmpdir('rsh-int')
env = { 'HOME' => home, 'TERM' => 'xterm', 'PATH' => '/usr/bin:/bin', 'LC_ALL' => 'C.UTF-8' }
checks = 0
failed = []

PTY.spawn(env, rsh, unsetenv_others: true) do |out, inp, pid|
  drain = lambda do |wait|
    buf = +''
    buf << out.read_nonblock(8192) while IO.select([out], nil, nil, wait)
    buf
  rescue EOFError, Errno::EIO
    buf
  end
  plain = ->(s) { s.gsub(/\e\[[0-9;]*[A-Za-z]/, '') }
  drain.call(1)
  send_line = lambda do |text, wait = 0.4|
    inp.write(text)
    plain.call(drain.call(wait))
  end
  expect = lambda do |label, text, want|
    checks += 1
    got = send_line.call(text)
    failed << "#{label}: wanted #{want.inspect} in #{got.inspect}" unless got.include?(want)
  end
  expect.call('command runs', "echo hi there\r", "\nhi there")
  expect.call('ruby value', "x = 3\r", '=> 3')
  expect.call('ruby uses it', "puts x * 2\r", "\n6")
  expect.call('tab completes a file', "ls /etc/hostn\t\r", '/etc/hostname')
  expect.call('background job', "sleep 1 &\r", '[1]')
  expect.call('jobs lists it', "jobs\r", 'Running')
  inp.write("[1,2].map do |n|\r")
  checks += 1
  more = plain.call(drain.call(0.4))
  failed << "continuation prompt: #{more.inspect}" unless more.include?('...')
  inp.write("n * 3\r")
  drain.call(0.3)
  expect.call('block finishes', "end\r", '=> [3, 6]')
  expect.call('status in prompt', "sh -c 'exit 4'\r", ' 4 >')
  expect.call('history search', "\x12ech", '(search)')
  inp.write("\x03")
  drain.call(0.3)
  # colours: a known command word is green, an unknown one red
  inp.write("echo")
  coloured = drain.call(0.4)
  checks += 1
  failed << "known command is green: #{coloured.inspect}" unless coloured.include?("\e[32mecho")
  inp.write("\x15zzqq-nothing")
  coloured = drain.call(0.4)
  checks += 1
  failed << "unknown command is red: #{coloured.inspect}" unless coloured.include?("\e[31m")
  inp.write("\x15\x04")
  drain.call(0.3)
  Process.kill('TERM', pid) rescue nil # rubocop:disable Style/RescueModifier
end

hist = File.join(home, '.local/state/rsh/history')
checks += 1
failed << 'history file not written' unless File.exist?(hist)
checks += 1
failed << "history mode #{File.stat(hist).mode.to_s(8)}" if File.exist?(hist) && (File.stat(hist).mode & 0o77) != 0
puts "interactive: #{checks} checks, #{failed.size} failed"
failed.each { |f| warn f }
exit(failed.empty? ? 0 : 1)

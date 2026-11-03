#!/usr/bin/ruby
# SPDX-License-Identifier: GPL-2.0-or-later
# Scanner fixture: a Ruby script that meets every RB rule (argv commands, typed decode,
# timeouts, a temporary folder with a block); it trips nothing.
# It is data for rules_ruby.rs and never runs (exec_bits NOT_RUN).
# frozen_string_literal: true

require 'json'
require 'net/http'
require 'open3'
require 'tmpdir'

# Reads the version of a program through an argv, never a shell string.
def version(program)
  out, status = Open3.capture2(program, '--version')
  status.success? ? out.lines.first.to_s.strip : nil
end

# Fetches a JSON document with both timeouts set.
def fetch(uri)
  http = Net::HTTP.new(uri.host, uri.port)
  http.open_timeout = 5
  http.read_timeout = 10
  JSON.parse(http.get(uri.request_uri).body)
end

Dir.mktmpdir do |dir|
  path = File.join(dir, 'state.json')
  File.write(path, JSON.generate({ 'ok' => true }))
  File.chmod(0o600, path)
  system('ls', '-l', dir, exception: false)
end

begin
  puts version('ruby')
rescue Errno::ENOENT => e
  warn e.message
end

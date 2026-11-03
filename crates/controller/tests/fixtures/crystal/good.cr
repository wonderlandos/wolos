# SPDX-License-Identifier: GPL-2.0-or-later
# Scanner self-test: code that meets every Crystal rule (docs/RULES-crystal.md). It is read by
# rules_crystal.rs and also has to be crystal-format and ameba clean; it is never compiled.
require "http/client"
require "json"
require "./sibling"

# A typed document: decoding into a struct, never JSON.parse into JSON::Any.
struct Release
  include JSON::Serializable

  getter tag : String
  getter assets : Array(String)
end

module Fixture
  # Runs a program with an argument array, no shell.
  def self.run(dir : String) : Bool
    Process.run("git", ["-C", dir, "status"], output: Process::Redirect::Close).success?
  end

  def self.fetch(url : String) : String
    client = HTTP::Client.new(URI.parse(url))
    client.connect_timeout = 5.seconds
    client.read_timeout = 15.seconds
    client.get("/").body
  end

  def self.release(body : String) : Release?
    Release.from_json(body)
  rescue ex : JSON::ParseException | JSON::SerializableError
    STDERR.puts "bad release document: #{ex.message}"
    nil
  end

  def self.scratch : String
    file = File.tempfile("fixture")
    File.chmod(file.path, 0o600)
    file.path
  end

  def self.token : String
    Random::Secure.hex(16)
  end
end

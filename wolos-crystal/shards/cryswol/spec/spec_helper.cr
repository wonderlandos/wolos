# SPDX-License-Identifier: BSD-2-Clause
# Shared spec support: every group runs against `wol_fake` (crates/wol-abi/examples/wol_fake.rs), a
# scripted fake wold and Kit host on the real wires, so plain `crystal spec` needs no WolOS
# desktop. `make spec` builds the fake and libwol and passes WOL_FAKE and the link flags.
require "spec"
require "json"
require "file_utils"
require "../src/cryswol"

module Wol::Spec
  # The shard root (holds spec/fixtures and build/).
  ROOT = Path[__DIR__].parent

  # The fake's executable: $WOL_FAKE, or build/wol_fake as `make spec` leaves it.
  def self.fake_path : ::String
    ENV["WOL_FAKE"]? || (ROOT / "build" / "wol_fake").to_s
  end

  # One running fake, with the sockets in `dir` and a log of what it received.
  class Fake
    # The directory holding wold.sock, kit.sock, the log and attached files.
    getter dir : ::String
    # The app id the scenario names (`"app"`), or "org.example.spec".
    getter app : ::String

    @process : ::Process
    @log : ::String

    # Start the fake for spec/fixtures/<group>.json and wait until it listens.
    def initialize(group : ::String)
      scenario = (ROOT / "spec" / "fixtures" / "#{group}.json").to_s
      @app = JSON.parse(::File.read(scenario))["app"]?.try(&.as_s?) || "org.example.spec"
      @dir = ::File.tempname("cryswol-#{group}")
      Dir.mkdir_p(::File.join(@dir, "config"))
      @log = ::File.join(@dir, "log.jsonl")
      @process = ::Process.new(Wol::Spec.fake_path, [scenario, @dir, @log],
        input: :pipe, output: :pipe, error: :inherit)
      ready = @process.output.gets
      raise "wol_fake did not start (#{ready.inspect})" unless ready == "ready"
    end

    # Every message the fake received so far, as `{"wold": call}` / `{"kit": message}` objects.
    def log : Array(JSON::Any)
      return [] of JSON::Any unless ::File.exists?(@log)
      ::File.read_lines(@log).reject(&.empty?).map { |l| JSON.parse(l) }
    end

    # The calls wold received, in order.
    def calls : Array(JSON::Any)
      log.compact_map { |e| e["wold"]? }
    end

    # The tags of the calls wold received ("notification.post", ...).
    def call_tags : Array(::String)
      calls.map(&.["call"].as_s)
    end

    # What the app sent the Kit host, in order (the hello first).
    def shown : Array(JSON::Any)
      log.compact_map { |e| e["kit"]? }
    end

    # Lines the fake refused or could not resolve (a broken fixture or a malformed message).
    def problems : Array(JSON::Any)
      log.reject { |e| e["wold"]? || e["kit"]? }
    end

    # Stop the fake (closing its stdin) and remove its directory.
    def stop : Nil
      @process.input.close rescue nil
      @process.wait rescue nil
      FileUtils.rm_rf(@dir)
    end
  end

  # Start the fake for `group`, point libwol at it, open a context for the scenario's app id (or
  # `app`), yield both, then close everything.
  def self.with_fake(group : ::String, app : ::String? = nil, & : Wol::Context, Fake ->) : Nil
    fake = Fake.new(group)
    begin
      ENV["WOL_SOCKET"] = ::File.join(fake.dir, "wold.sock")
      ENV["WOL_KIT_SOCKET"] = ::File.join(fake.dir, "kit.sock")
      ENV["XDG_CONFIG_HOME"] = ::File.join(fake.dir, "config")
      ctx = Wol::Context.open(app || fake.app)
      begin
        yield ctx, fake
      ensure
        ctx.close
      end
    ensure
      fake.stop
    end
  end

  # Run the loop until the block is true or `timeout` passes; true when the block said so.
  def self.run_until(ctx : Wol::Context, timeout : Time::Span = 3.seconds, & : -> Bool) : Bool
    Wol::Loop.run(ctx, timeout) { yield }
  end

  # Collect what `Wol::Loop.on_error` receives while the block runs, then restore the default.
  def self.capture_errors(& : Array(Exception) ->) : Array(Exception)
    errors = [] of Exception
    Wol::Loop.on_error = ->(ex : Exception) { errors << ex; nil }
    begin
      yield errors
    ensure
      Wol::Loop.on_error = ->(ex : Exception) { STDERR.puts "cryswol: a callback raised #{ex.class}: #{ex.message}"; nil }
    end
    errors
  end
end

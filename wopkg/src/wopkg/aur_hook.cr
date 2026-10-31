# SPDX-License-Identifier: GPL-2.0-or-later
require "json"

module Wopkg
  # The one hook between the sandboxed build and the install (the `aurlaws` analyser). It gets the
  # package folder, the built file and, if the build was traced, the system-call trace, and returns
  # a verdict with evidence. wopkg's own static signals (AurGuard) stay the cheap first pass. A
  # missing or broken analyser is "not checked", never "pass".
  module AurHook
    enum Kind
      Pass
      Flag
      Block
      Unavailable
    end

    record Verdict, kind : Kind, evidence : Array(String) do
      def pass? : Bool
        kind.pass?
      end

      def flag? : Bool
        kind.flag?
      end

      def block? : Bool
        kind.block?
      end

      def unavailable? : Bool
        kind.unavailable?
      end
    end

    NOT_CHECKED = "The package analyser did not run, so the build was not checked beyond the basic warning signs."

    def self.find : String?
      ENV["WOPKG_AURLAWS"]? || Process.find_executable("wopkg-aurlaws")
    end

    def self.check(dir : String, artifact : String, trace : String?, analyser : String? = find) : Verdict
      return Verdict.new(Kind::Unavailable, [NOT_CHECKED]) unless analyser
      args = ["--dir", dir, "--artifact", artifact]
      args.concat(["--trace", trace]) if trace
      args << "--json"
      buf = IO::Memory.new
      ok = Process.run(analyser, args, output: buf, error: Process::Redirect::Close).success?
      return Verdict.new(Kind::Unavailable, [NOT_CHECKED]) unless ok
      j = JSON.parse(buf.to_s)
      kind = case j["verdict"]?.try(&.as_s?)
             when "pass"  then Kind::Pass
             when "flag"  then Kind::Flag
             when "block" then Kind::Block
             else              return Verdict.new(Kind::Unavailable, [NOT_CHECKED])
             end
      Verdict.new(kind, (j["evidence"]?.try(&.as_a?) || [] of JSON::Any).compact_map(&.as_s?))
    rescue
      Verdict.new(Kind::Unavailable, [NOT_CHECKED])
    end
  end
end
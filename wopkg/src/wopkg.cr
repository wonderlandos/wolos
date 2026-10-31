# SPDX-License-Identifier: BSD-2-Clause
# wopkg: WolOS's package tool (ADR 0021, archicture/modules/wopkg.md).
require "./wopkg/app"

module Wopkg
  VERSION = "0.1.0"

  def self.main(argv : Array(String)) : Int32
    args = Args.parse(argv)
    App.new(args).run
  rescue e : UsageError | Recipe::Invalid | Sandbox::Refused | ArgumentError
    STDERR.puts e.message
    2
  rescue e : Failure
    if argv.includes?("--json")
      STDOUT.puts({"error" => e.kind, "message" => e.message}.to_json)
    else
      STDERR.puts e.message
    end
    e.code == 2 ? 77 : 1
  end
end

exit Wopkg.main(ARGV)

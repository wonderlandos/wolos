# SPDX-License-Identifier: GPL-2.0-or-later
# wol-hue: a WolOS theme from a picture (palette engine) or from a Base16 / colors.toml palette;
# docs/modules/hue.md, ADR 0031.
require "./hue/cli"

module Hue
  def self.main(argv : Array(String)) : Int32
    Cli.new.run(argv)
  rescue e : UsageError | ArgumentError
    STDERR.puts "wol-hue: #{e.message}"
    2
  rescue e : Failure | ImageError | Extract::Failed | Imports::Invalid | Look::Invalid | Theme::Invalid | YAML::ParseException | File::Error
    if argv.includes?("--json")
      STDOUT.puts({"error" => e.class.name.split("::").last.downcase, "message" => e.message}.to_json)
    else
      STDERR.puts "wol-hue: #{e.message}"
    end
    1
  end
end

exit Hue.main(ARGV)

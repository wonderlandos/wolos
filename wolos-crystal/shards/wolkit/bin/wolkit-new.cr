# SPDX-License-Identifier: BSD-2-Clause
# wolkit-new: `wolkit-new NAME [DIR] [TEMPLATE]` writes a new Wolkit project (ADR 0036 plans a
# `wol app new` over it; that command does not exist yet). It prints the files it wrote, one per line.
require "../src/wolkit/template"

name = ARGV[0]? || abort "usage: wolkit-new NAME [DIR] [TEMPLATE]"
target = ARGV[1]? || name
begin
  Wolkit::Template.render(name, target, ARGV[2]? || "app").each { |file| puts File.join(target, file) }
rescue ex : ArgumentError
  abort ex.message
end

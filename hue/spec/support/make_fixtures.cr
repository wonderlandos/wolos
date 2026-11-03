# SPDX-License-Identifier: GPL-2.0-or-later
# Writes the ten test pictures as PNG into the folder given (to feed another engine the same
# pixels when re-recording golden palettes).
require "./scenes"

dir = ARGV[0]? || abort("usage: make_fixtures.cr DIR")
Dir.mkdir_p(dir)
Scenes.all.each do |name, pic|
  path = File.join(dir, "#{name}.png")
  pic.save(path)
  puts path
end

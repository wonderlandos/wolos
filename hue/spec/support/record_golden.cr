# SPDX-License-Identifier: GPL-2.0-or-later
# Re-records spec/fixtures/golden.json from this engine: 10 scenes x dark/light x 3 modes. Only
# for a deliberate engine change (bump Engine::CACHE_VERSION in the same commit).
require "json"
require "../../src/hue/engine"
require "./scenes"

dir = ARGV[0]? || abort("usage: record_golden.cr spec/fixtures")
Dir.mkdir_p(dir)
tmp = File.join(Dir.tempdir, "wol-hue-golden-#{Process.pid}")
Dir.mkdir_p(tmp)
cases = {} of String => Array(String)
Scenes.all.each do |name, pic|
  path = File.join(tmp, "#{name}.png")
  pic.save(path)
  {"dark", "light"}.each do |variant|
    %w(normal monochromatic pastel).each do |mode|
      cases["#{name}/#{variant}/#{mode}"] = Hue::Engine.from_picture(path, variant == "light", mode, cache: false).palette
    end
  end
end
File.write(File.join(dir, "golden.json"), {"engine" => "wol-hue #{Hue::Engine::CACHE_VERSION}", "cases" => cases}.to_pretty_json + "\n")
puts "#{cases.size} palettes"

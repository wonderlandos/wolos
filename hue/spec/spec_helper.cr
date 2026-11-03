# SPDX-License-Identifier: GPL-2.0-or-later
require "spec"
require "file_utils"
require "../src/hue/engine"
require "../src/hue/imports"
require "../src/hue/look"
require "./support/scenes"

module SpecHelp
  @@files : Hash(String, String)? = nil

  # The ten scenes as PNG files (drawn once per run).
  def self.scene_files : Hash(String, String)
    @@files ||= begin
      dir = File.join(Dir.tempdir, "wol-hue-spec-#{Process.pid}")
      Dir.mkdir_p(dir)
      Scenes.all.to_h { |n, pic| path = File.join(dir, "#{n}.png"); pic.save(path); {n, path} }
    end
  end

  @@dominant = {} of String => {Array(String), Array(Int32)}

  # The picture is decoded and quantised once per scene (that is the slow part); every mode and
  # variant then runs the same palette step `Engine.from_picture` runs.
  def self.palette(name : String, light : Bool, mode : String) : Array(String)
    raise ArgumentError.new("unknown mode “#{mode}”") unless Hue::Palette::MODES.includes?(mode)
    dom, counts = @@dominant[name] ||= Hue::Extract.dominant(Hue::Extract.sample(Hue::Picture.load(scene_files[name])))
    Hue::Engine.from_dominant(dom, counts, light, mode)
  end

  def self.tmpdir : String
    d = File.join(Dir.tempdir, "wol-hue-#{Process.pid}-#{Random.rand(1_000_000)}")
    Dir.mkdir_p(d)
    d
  end
end

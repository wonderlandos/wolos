# SPDX-License-Identifier: GPL-2.0-or-later
# Picture -> 16 colours, with a cache: $XDG_CACHE_HOME/wolos/hue/<sha256 of the file>-<variant>-
# <mode>-v<CACHE_VERSION>.json. Keyed by content, not path, so a renamed or copied wallpaper hits
# and an edited one misses.
require "digest/sha256"
require "json"
require "./palette"
require "./theme"

module Hue
  module Engine
    # Bump when the engine's output changes for the same input (old entries are then ignored).
    CACHE_VERSION = 1

    record Result, palette : Array(String), cached : Bool

    def self.cache_dir : String
      x = ENV["XDG_CACHE_HOME"]?
      File.join(x && !x.empty? ? x : File.join(ENV["HOME"]? || "/", ".cache"), "wolos", "hue")
    end

    def self.cache_key(path : String, light : Bool, mode : String) : String
      d = Digest::SHA256.new
      File.open(path, "rb") do |f|
        buf = Bytes.new(1 << 20)
        while (n = f.read(buf)) > 0
          d.update(buf[0, n])
        end
      end
      "#{d.final.hexstring}-#{light ? "light" : "dark"}-#{mode}-v#{CACHE_VERSION}"
    end

    def self.from_dominant(dom : Array(String), counts : Array(Int32), light : Bool, mode : String) : Array(String)
      raise Extract::Failed.new("not enough colours in the picture") if dom.size < 8
      Palette.generate(dom, Extract.weights(counts), light, mode)
    end

    def self.from_picture(path : String, light : Bool, mode : String, cache = true) : Result
      raise ArgumentError.new("unknown mode “#{mode}” (#{Palette::MODES.join(", ")})") unless Palette::MODES.includes?(mode)
      raise ImageError.new("#{path}: no such file") unless File.file?(path)
      file = nil
      if cache
        file = File.join(cache_dir, cache_key(path, light, mode) + ".json")
        if (hit = read_cache(file))
          return Result.new(hit, true)
        end
      end
      dom, counts = Extract.dominant(Extract.sample(Picture.load(path)))
      p = from_dominant(dom, counts, light, mode)
      if file
        begin
          Theme.atomic_write(file, {"version" => CACHE_VERSION, "palette" => p}.to_json)
        rescue File::Error
          # A read-only cache only costs time.
        end
      end
      Result.new(p, false)
    end

    private def self.read_cache(file : String) : Array(String)?
      return nil unless File.file?(file)
      j = JSON.parse(File.read(file))
      return nil unless j["version"]?.try(&.as_i?) == CACHE_VERSION
      p = j["palette"].as_a.map(&.as_s)
      p.size == 16 && p.all? { |c| Color.valid_hex?(c) } ? p : nil
    rescue JSON::ParseException | TypeCastError | KeyError | File::Error
      nil
    end
  end
end

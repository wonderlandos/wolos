# SPDX-License-Identifier: GPL-2.0-or-later
# A look: the 16 base colours plus slider positions and where they came from. Every theme this
# tool writes keeps its look next to it (themes/<id>/look.json) so `adjust` always starts from
# the original colours (sliders are positions, not steps); `save` copies one to
# $XDG_DATA_HOME/wolos/hue/looks/<name>.json to keep it after the theme is regenerated.
require "json"
require "./color"
require "./theme"

module Hue
  class Look
    include JSON::Serializable

    property name : String
    property colors : Array(String)
    property variant : String
    property mode : String = "normal"
    property source : String = "picture"
    property wallpaper : String? = nil
    property accent : String? = nil
    property adjustments : Hash(String, Float64) = {} of String => Float64
    property saved : Int64 = 0_i64

    class Invalid < Exception
    end

    NAME = /\A[A-Za-z0-9][A-Za-z0-9 ._-]{0,63}\z/

    def initialize(@name, @colors, @variant, @mode = "normal", @source = "picture", @wallpaper = nil, @accent = nil)
      @saved = Time.utc.to_unix
    end

    def dark? : Bool
      @variant == "dark"
    end

    # Refuses what the theme writer could not use; returns self.
    def validate! : Look
      raise Invalid.new("a look has 16 colours, this one #{@colors.size}") unless @colors.size == 16
      bad = @colors.reject { |c| Color.valid_hex?(c) }
      raise Invalid.new("not colours: #{bad.join(", ")}") unless bad.empty?
      raise Invalid.new("variant must be dark or light") unless {"dark", "light"}.includes?(@variant)
      @adjustments.each do |k, v|
        lo, hi, _ = Color::SLIDERS[k]? || raise Invalid.new("unknown slider #{k}")
        raise Invalid.new("#{k} #{v} is outside #{lo}..#{hi}") unless lo <= v <= hi
      end
      self
    end

    # The colours after the sliders.
    def final_colors : Array(String)
      return @colors.map(&.downcase) if @adjustments.all? { |k, v| v == Color::SLIDERS[k][2] }
      @colors.map { |c| Color.adjust(c, @adjustments) }
    end

    def final_accent : String?
      a = @accent
      return nil unless a
      @adjustments.empty? ? a : Color.adjust(a, @adjustments)
    end

    def theme_json(id : String) : String
      Theme.json(id, @name, final_colors, dark?, @wallpaper, final_accent)
    end

    def self.data_home : String
      x = ENV["XDG_DATA_HOME"]?
      x && !x.empty? ? x : File.join(ENV["HOME"]? || "/", ".local", "share")
    end

    def self.dir : String
      File.join(data_home, "wolos", "hue", "looks")
    end

    def self.path(name : String) : String
      raise Invalid.new("look name “#{name}”: letters, digits, space . _ -, up to 64") unless name.matches?(NAME)
      File.join(dir, "#{name}.json")
    end

    def self.read(file : String) : Look
      from_json(File.read(file)).validate!
    rescue e : JSON::SerializableError | JSON::ParseException
      raise Invalid.new("#{file}: #{e.message}")
    end

    def self.list : Array(Look)
      return [] of Look unless Dir.exists?(dir)
      Dir.children(dir).select(&.ends_with?(".json")).sort.compact_map do |f|
        read(File.join(dir, f))
      rescue Invalid
        nil
      end
    end
  end
end

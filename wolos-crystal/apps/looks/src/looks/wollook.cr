# SPDX-License-Identifier: GPL-2.0-or-later
# The .wollook file: one look as JSON, to hand to another person, and the reader for it and for the
# palettes other tools write (Base16 YAML, colors.toml). Everything that comes in is untrusted: it is
# size-limited, parsed into types, checked by the engine's own look rules and given a name the
# store accepts.
#
# Concept: {"format":"wollook","version":1,"name":..,"variant":..,"mode":..,"colors":[16],
# "accent":..,"adjustments":{..}}. Why: a look should travel as one small file that says what it
# is. How: Wollook.dump leaves out the wallpaper path (it names a file on this computer); parse
# refuses another format or a newer version instead of guessing.
require "json"
require "file_utils"
require "./engine_link"

module Looks
  module Wollook
    FORMAT    = "wollook"
    VERSION   = 1
    MAX_BYTES = 64 * 1024

    class Invalid < Exception
    end

    struct Doc
      include JSON::Serializable

      property format : String
      property version : Int32
      property name : String
      property variant : String
      property mode : String = "normal"
      property colors : Array(String)
      property accent : String? = nil
      property adjustments : Hash(String, Float64) = {} of String => Float64

      def initialize(@format, @version, @name, @variant, @mode, @colors, @accent, @adjustments)
      end
    end

    # The file text for a look (pretty-printed, a final newline).
    def self.dump(look : Hue::Look) : String
      doc = Doc.new(FORMAT, VERSION, look.name, look.variant, look.mode, look.colors.map(&.downcase), look.accent, look.adjustments)
      doc.to_pretty_json + "\n"
    end

    # The look in `text`. Raises Invalid with a sentence.
    def self.parse(text : String) : Hue::Look
      raise Invalid.new("This file is too large to be a look.") if text.bytesize > MAX_BYTES
      doc = Doc.from_json(text)
      raise Invalid.new("This is not a .wollook file.") unless doc.format == FORMAT
      raise Invalid.new("This look was made by a newer version of Looks.") if doc.version > VERSION
      look = Hue::Look.new(Names.clean(doc.name), doc.colors.map(&.downcase), doc.variant, doc.mode, "imported", nil, doc.accent)
      doc.adjustments.each { |k, v| look.adjustments[k] = v }
      look.validate!
    rescue JSON::ParseException | JSON::SerializableError
      raise Invalid.new("This file is not a valid .wollook look.")
    rescue ex : Hue::Look::Invalid
      raise Invalid.new("This look cannot be used: #{ex.message}")
    end
  end

  module Names
    # A name the store accepts (`Hue::Look::NAME`): letters, digits, space, dot, dash, underscore.
    def self.clean(raw : String, fallback : String = "Imported look") : String
      s = raw.gsub(/[^A-Za-z0-9 ._-]+/, " ").gsub(/\s+/, " ").strip
      s = s.lstrip(" ._-")
      s = s[0, 64].rstrip
      s.matches?(Hue::Look::NAME) ? s : fallback
    end

    # `base`, or `base 2`, `base 3`... when that name is taken.
    def self.unique(base : String, taken : Enumerable(String)) : String
      set = taken.map(&.downcase).to_set
      return base unless set.includes?(base.downcase)
      n = 2
      loop do
        suffix = " #{n}"
        cand = "#{base[0, 64 - suffix.size].rstrip}#{suffix}"
        return cand unless set.includes?(cand.downcase)
        n += 1
      end
    end
  end

  # Reading what a person picks in the Import panel.
  module Importer
    # `file_name` decides the kind: .wollook or .json, .yaml or .yml (Base16), .toml (colors.toml).
    # `scratch` is a private folder for the temporary copy the engine's readers need (they take a path).
    def self.read(file_name : String, text : String, scratch : String) : Hue::Look
      raise Wollook::Invalid.new("This file is too large to be a palette.") if text.bytesize > Wollook::MAX_BYTES
      case File.extname(file_name).downcase
      when ".wollook", ".json"
        Wollook.parse(text)
      when ".yaml", ".yml"
        palette(text, ".yaml", scratch, file_name, "base16") { |p| Hue::Imports.base16(p) }
      when ".toml"
        palette(text, ".toml", scratch, file_name, "colors") { |p| Hue::Imports.colors_toml(p) }
      else
        raise Wollook::Invalid.new("Looks reads .wollook, Base16 (.yaml) and colors (.toml) files.")
      end
    end

    private def self.palette(text : String, ext : String, scratch : String, file_name : String, source : String, & : String -> Hue::Imports::Imported) : Hue::Look
      FileUtils.mkdir_p(scratch, 0o700)
      base = File.basename(file_name, File.extname(file_name))
      tmp = File.join(scratch, "#{Names.clean(base, "palette")}#{ext}")
      begin
        File.write(tmp, text, perm: 0o600)
        imp = yield tmp
        light = imp.light
        light = Hue::Color.hex_to_lab(imp.colors[0]).l >= 0.5 if light.nil?
        name = Names.clean(imp.name.empty? ? base : imp.name)
        Hue::Look.new(name, imp.colors.map(&.downcase), light ? "light" : "dark", "imported", source, nil, imp.accent).validate!
      rescue ex : Hue::Imports::Invalid | Hue::Look::Invalid | YAML::ParseException
        raise Wollook::Invalid.new("This palette could not be read: #{ex.message}")
      ensure
        File.delete?(tmp)
      end
    end
  end
end

# SPDX-License-Identifier: MIT
# Portions: see docs/THIRD-PARTY.md
require "yaml"
require "../../../wopkg/src/wopkg/toml"
require "./color"

module Hue
  # Palettes from other tools: a Base16 scheme (YAML) or a colors.toml (key = "#hex").
  module Imports
    class Invalid < Exception
    end

    record Imported, name : String, colors : Array(String), light : Bool?, accent : String?

    private def self.hex(v : String) : String?
      v = v.strip.strip('"').strip('\'').strip
      v = "##{v}" unless v.starts_with?('#')
      Color.valid_hex?(v) ? v.downcase : nil
    end

    # Base16: base00..base0F at the top level or under `palette:` (both scheme formats).
    # base00 -> 0, base08 red -> 1/9, base0B green -> 2/10, base0A yellow -> 3/11, base0D blue ->
    # 4/12, base0E magenta -> 5/13, base0C cyan -> 6/14, base05 -> 7, base03 -> 8, base07 -> 15.
    def self.base16(path : String) : Imported
      doc = begin
        Hash(String, Entry).from_yaml(File.read(path))
      rescue e : YAML::ParseException
        raise Invalid.new("#{path}: not a Base16 scheme (#{e.message})")
      end
      top = scalars(doc)
      pal = doc["palette"]?.try(&.fields)
      vals = pal ? scalars(pal) : top
      base = {} of Int32 => String
      16.times do |i|
        k = "base%02X" % i
        v = vals[k]? || vals[k.downcase]?
        next unless v
        base[i] = hex(v) || raise Invalid.new("#{path}: #{k} is not a colour (#{v})")
      end
      raise Invalid.new("#{path}: not enough colours (#{base.size} of 16)") if base.size < 16
      order = [0x00, 0x08, 0x0B, 0x0A, 0x0D, 0x0E, 0x0C, 0x05, 0x03, 0x08, 0x0B, 0x0A, 0x0D, 0x0E, 0x0C, 0x07]
      name = top["scheme"]? || top["name"]? || File.basename(path, File.extname(path))
      light = case top["variant"]?.try(&.downcase)
              when "light" then true
              when "dark"  then false
              end
      Imported.new(name, order.map { |i| base[i] }, light, nil)
    end

    # One value in a scheme file: a scalar's text (kept as written, `000000` stays `000000`), or a
    # mapping of such values. A list or an alias is neither.
    class Entry
      getter text : String?
      getter fields : Hash(String, Entry)?

      def initialize(@text : String? = nil, @fields : Hash(String, Entry)? = nil)
      end

      def self.new(ctx : YAML::ParseContext, node : YAML::Nodes::Node) : Entry
        case node
        when YAML::Nodes::Scalar  then new(node.value)
        when YAML::Nodes::Mapping then new(nil, Hash(String, Entry).new(ctx, node))
        else                           new
        end
      end
    end

    private def self.scalars(m : Hash(String, Entry)) : Hash(String, String)
      h = {} of String => String
      m.each { |k, e| e.text.try { |t| h[k] = t } }
      h
    end

    # Each slot takes the first key present: the semantic name, then the colourN alias.
    SLOTS = [
      %w(background bg color0 black),
      %w(red color1),
      %w(green color2),
      %w(yellow color3),
      %w(blue color4),
      %w(magenta purple color5),
      %w(cyan color6),
      %w(foreground fg color7 white light_foreground light_fg),
      %w(muted color8 bright_black dark_foreground dark_fg),
      %w(bright_red color9 red),
      %w(bright_green color10 green),
      %w(bright_yellow color11 yellow),
      %w(bright_blue color12 blue),
      %w(bright_magenta bright_purple color13 magenta purple),
      %w(bright_cyan color14 cyan),
      %w(bright_foreground bright_fg color15 bright_white foreground light_foreground light_fg),
    ]

    def self.colors_toml(path : String) : Imported
      t = begin
        Wopkg::Toml.parse(File.read(path))
      rescue e : Wopkg::Toml::ParseError
        raise Invalid.new("#{path}: #{e.message}")
      end
      kv = {} of String => String
      t.each { |k, v| kv[k] = v.to_s if v.is_a?(String) || v.is_a?(Bool) }
      first = ->(keys : Array(String)) { keys.compact_map { |k| kv[k]?.try { |v| hex(v) } }.first? }
      colors = SLOTS.map { |chain| first.call(chain) }
      colors[0] ||= "#1a1b26"
      colors[7] ||= "#a9b1d6"
      colors[8] ||= colors[0]
      colors[15] ||= colors[7]
      (9..14).each { |k| colors[k] ||= colors[k - 8] } # no bright twin: the normal colour
      missing = (0...16).reject { |i| colors[i] }
      raise Invalid.new("#{path}: no colour for slot(s) #{missing.join(", ")}") unless missing.empty?
      mode = (kv["mode"]? || kv["theme_type"]?).try(&.downcase)
      light = case mode
              when "light" then true
              when "dark"  then false
              else
                case kv["light_mode"]?.try(&.downcase)
                when "true", "1", "yes" then true
                when "false", "0", "no" then false
                end
              end
      Imported.new(File.basename(path, File.extname(path)), colors.compact, light, kv["accent"]?.try { |v| hex(v) })
    end
  end
end

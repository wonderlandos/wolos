# SPDX-License-Identifier: GPL-2.0-or-later
# The wol-hue command line. Every verb ends in one theme folder under
# $XDG_CONFIG_HOME/wolos/themes/<id>/ (theme.json + look.json) and, unless --no-apply, in
# `wol theme set <id>`: the desktop's one theme path restyles every app (ADR 0031). This tool
# never writes an app's configuration itself.
require "json"
require "./engine"
require "./imports"
require "./look"

module Hue
  VERSION = "0.1.0"

  class UsageError < Exception
  end

  class Cli
    USAGE = <<-T
      wol-hue: a WolOS theme from a picture or a palette

        wol-hue from-wallpaper <picture> [--dark|--light] [--mode MODE] [--id ID] [--name NAME]
        wol-hue adjust [THEME-ID] [--SLIDER N ...] [--reset]
        wol-hue import base16|colors <file> [--dark|--light] [--id ID] [--wallpaper PICTURE]
        wol-hue save <look> [--theme THEME-ID]
        wol-hue list
        wol-hue apply <look> [--id ID]

      Common options: --no-apply (write the theme, do not switch to it), --json, --no-cache.
      Modes: #{Palette::MODES.join(", ")}; --mode dark|light picks the variant.
      Sliders: #{Color::SLIDERS.map { |k, v| "--#{Cli.dash(k)} #{Cli.num(v[0])}..#{Cli.num(v[1])}" }.join(", ")}.
      T

    def self.dash(k : String) : String
      k.gsub(/[A-Z]/) { |c| "-#{c.downcase}" }
    end

    def self.num(x : Float64) : String
      x == x.to_i ? x.to_i.to_s : x.to_s
    end

    def initialize(@io : IO = STDOUT)
    end

    # Options that take a value; everything else starting with -- is a switch.
    VALUED = %w(--mode --id --name --theme --wallpaper) + Color::SLIDERS.keys.map { |k| "--#{Cli.dash(k)}" }

    record Args, words : Array(String), opts : Hash(String, String), flags : Set(String)

    def parse(argv : Array(String)) : Args
      words = [] of String
      opts = {} of String => String
      flags = Set(String).new
      i = 0
      while i < argv.size
        a = argv[i]
        if a == "--"
          words.concat(argv[(i + 1)..])
          break
        elsif a.starts_with?("--")
          k, eq, v = a.partition('=')
          if VALUED.includes?(k)
            v = eq.empty? ? (argv[i += 1]? || raise UsageError.new("#{k} needs a value")) : v
            opts[k] = v
          elsif %w(--dark --light --no-apply --json --no-cache --reset --help --version).includes?(k) && eq.empty?
            flags << k
          else
            raise UsageError.new("unknown option #{a}")
          end
        else
          words << a
        end
        i += 1
      end
      Args.new(words, opts, flags)
    end

    def run(argv : Array(String)) : Int32
      a = parse(argv)
      return (@io.puts(VERSION); 0) if a.flags.includes?("--version")
      verb = a.words.shift?
      return (@io.puts(USAGE); 0) if verb.nil? || verb == "help" || a.flags.includes?("--help")
      case verb
      when "from-wallpaper", "generate" then from_wallpaper(a)
      when "adjust"                     then adjust(a)
      when "import"                     then import(a)
      when "save"                       then save(a)
      when "list"                       then list(a)
      when "apply"                      then apply(a)
      else                                   raise UsageError.new("unknown command “#{verb}” (wol-hue help)")
      end
    end

    private def light?(a : Args, default : Bool? = nil) : Bool
      m = a.opts["--mode"]?
      return true if a.flags.includes?("--light") || m == "light"
      return false if a.flags.includes?("--dark") || m == "dark"
      default || false
    end

    private def from_wallpaper(a : Args) : Int32
      pic = a.words.shift? || raise UsageError.new("from-wallpaper needs a picture")
      raise UsageError.new("one picture at a time") unless a.words.empty?
      pic = File.expand_path(pic, home: true)
      m = a.opts["--mode"]?
      mode = m.nil? || m == "dark" || m == "light" ? "normal" : m
      raise UsageError.new("unknown mode “#{mode}” (#{Palette::MODES.join(", ")}, dark, light)") unless Palette::MODES.includes?(mode)
      light = light?(a)
      base = File.basename(pic, File.extname(pic))
      id = a.opts["--id"]? || "picture-#{Theme.slug(base)}#{light ? "-light" : ""}"
      name = a.opts["--name"]? || base.gsub(/[-_]+/, " ").strip
      r = Engine.from_picture(pic, light, mode, cache: !a.flags.includes?("--no-cache"))
      look = Look.new(name, r.palette, light ? "light" : "dark", mode, "picture", pic)
      write(id, look, a, {"cached" => JSON::Any.new(r.cached)})
    end

    private def adjust(a : Args) : Int32
      id = a.words.shift? || current_theme || raise UsageError.new("adjust needs a theme id (no theme is active)")
      file = File.join(Theme.dir(id), "look.json")
      raise UsageError.new("theme “#{id}” was not made by wol-hue (no #{file})") unless File.file?(file)
      look = Look.read(file)
      look.adjustments.clear if a.flags.includes?("--reset")
      Color::SLIDERS.each do |k, (lo, hi, _)|
        next unless v = a.opts["--#{Cli.dash(k)}"]?
        f = v.to_f? || raise UsageError.new("--#{Cli.dash(k)} needs a number")
        raise UsageError.new("--#{Cli.dash(k)} #{v} is outside #{lo}..#{hi}") unless lo <= f <= hi
        look.adjustments[k] = f
      end
      write(id, look.validate!, a)
    end

    private def import(a : Args) : Int32
      kind = a.words.shift? || raise UsageError.new("import base16|colors <file>")
      file = a.words.shift? || raise UsageError.new("import #{kind} needs a file")
      file = File.expand_path(file, home: true)
      raise UsageError.new("#{file}: no such file") unless File.file?(file)
      imp = case kind
            when "base16"                then Imports.base16(file)
            when "colors", "colors.toml" then Imports.colors_toml(file)
            else                              raise UsageError.new("import base16 or colors, not “#{kind}”")
            end
      light = if a.flags.includes?("--light") || a.flags.includes?("--dark")
                light?(a)
              else
                imp.light.nil? ? Color.hex_to_lab(imp.colors[0]).l >= 0.5 : imp.light.not_nil!
              end
      wp = a.opts["--wallpaper"]?.try { |w| File.expand_path(w, home: true) }
      id = a.opts["--id"]? || Theme.slug(imp.name)
      look = Look.new(a.opts["--name"]? || imp.name, imp.colors, light ? "light" : "dark", "imported",
        kind == "base16" ? "base16" : "colors", wp, imp.accent)
      write(id, look.validate!, a)
    end

    private def save(a : Args) : Int32
      name = a.words.shift? || raise UsageError.new("save needs a look name")
      id = a.opts["--theme"]? || current_theme || raise UsageError.new("save --theme ID (no theme is active)")
      src = File.join(Theme.dir(id), "look.json")
      raise UsageError.new("theme “#{id}” was not made by wol-hue (no #{src})") unless File.file?(src)
      look = Look.read(src)
      look.name = name
      look.saved = Time.utc.to_unix
      dst = Look.path(name)
      Theme.atomic_write(dst, look.to_pretty_json + "\n")
      if a.flags.includes?("--json")
        @io.puts({"look" => name, "file" => dst, "theme" => id}.to_json)
      else
        @io.puts "Saved “#{name}” (#{dst})."
      end
      0
    end

    private def list(a : Args) : Int32
      looks = Look.list
      if a.flags.includes?("--json")
        @io.puts looks.map { |l| {"name" => l.name, "variant" => l.variant, "mode" => l.mode, "source" => l.source, "colors" => l.final_colors} }.to_json
      elsif looks.empty?
        @io.puts "No saved looks (wol-hue save <name>)."
      else
        looks.each { |l| @io.puts "#{l.name}\t#{l.variant}\t#{l.source}/#{l.mode}" }
      end
      0
    end

    private def apply(a : Args) : Int32
      name = a.words.shift? || raise UsageError.new("apply needs a look name (wol-hue list)")
      file = Look.path(name)
      raise UsageError.new("no look “#{name}” (wol-hue list)") unless File.file?(file)
      look = Look.read(file)
      write(a.opts["--id"]? || "look-#{Theme.slug(name)}", look, a)
    end

    private def current_theme : String?
      f = File.join(Theme.config_home, "wolos", "theme")
      File.file?(f) ? File.read(f).lines.first?.try(&.strip).presence : nil
    end

    # theme.json + look.json, then `wol theme set`.
    private def write(id : String, look : Look, a : Args, extra = {} of String => JSON::Any) : Int32
      dir = Theme.dir(id)
      text = look.theme_json(id)
      Theme.atomic_write(File.join(dir, "look.json"), look.to_pretty_json + "\n")
      Theme.atomic_write(File.join(dir, "theme.json"), text)
      applied = false
      unless a.flags.includes?("--no-apply")
        wol = Process.find_executable("wol") || raise Failure.new("theme written to #{dir}; `wol` is missing, so it was not switched on")
        st = Process.run(wol, ["theme", "set", "--", id], output: Process::Redirect::Close, error: Process::Redirect::Inherit)
        raise Failure.new("wol theme set #{id} failed (exit #{st.exit_code}); the theme is in #{dir}") unless st.success?
        applied = true
      end
      if a.flags.includes?("--json")
        j = JSON.parse(text).as_h
        j["dir"] = JSON::Any.new(dir)
        j["mode"] = JSON::Any.new(look.mode)
        j["applied"] = JSON::Any.new(applied)
        extra.each { |k, v| j[k] = v }
        @io.puts j.to_json
      else
        @io.puts "#{look.name} (#{look.variant}, #{look.mode}): #{look.final_colors.join(" ")}"
        @io.puts applied ? "Theme “#{id}” is on (#{dir})." : "Theme “#{id}” written to #{dir} (wol theme set #{id} to use it)."
      end
      0
    end
  end

  class Failure < Exception
  end
end

# SPDX-License-Identifier: GPL-2.0-or-later
# The studio: from a picture to candidate looks. A picture is read once (its dominant colours); every
# mode in both variants then comes from the engine's palette step, and the sliders and the contrast
# contract run on top, so moving a slider re-colours all sixteen candidates at once.
#
# Concept: a Candidate is one palette (16 colours) with its variant, mode and slider positions.
# Why: the person compares modes side by side and moves sliders with instant feedback, so reading
# the picture (the slow part) must happen once and everything after it must be cheap arithmetic.
# How: Analysis holds the dominant colours; Candidate resolves tokens with Hue::Theme exactly as
# `wol-hue` does, so what is previewed is what `wol theme set` receives.
require "./engine_link"

module Looks
  # The five sliders the window offers (the engine has twelve; `wol-hue adjust` takes the rest).
  record SliderSpec, key : String, label : String, lo : Float64, hi : Float64, step : Float64

  SLIDERS = [
    SliderSpec.new("vibrance", "Vibrance", -50.0, 50.0, 5.0),
    SliderSpec.new("contrast", "Contrast", -30.0, 30.0, 5.0),
    SliderSpec.new("temperature", "Temperature", -50.0, 50.0, 5.0),
    SliderSpec.new("shadows", "Shadows", -50.0, 50.0, 5.0),
    SliderSpec.new("highlights", "Highlights", -50.0, 50.0, 5.0),
  ]

  MODES = Hue::Palette::MODES

  # Names of the modes as a person reads them.
  MODE_LABELS = {
    "normal" => "Natural", "monochromatic" => "Monochrome", "analogous" => "Analogous", "pastel" => "Pastel",
    "material" => "Material", "colorful" => "Colourful", "muted" => "Muted", "bright" => "Bright",
  }

  # The tokens of a look as opaque colours, ready to paint (translucent ones composited).
  struct Tokens
    getter surface : Hue::Theme::Rgba
    getter sidebar : Hue::Theme::Rgba
    getter card : Hue::Theme::Rgba
    getter line : Hue::Theme::Rgba
    getter text : Hue::Theme::Rgba
    getter dim : Hue::Theme::Rgba
    getter accent : Hue::Theme::Rgba
    getter accent_text : Hue::Theme::Rgba
    getter glass : Hue::Theme::Rgba
    getter control : Hue::Theme::Rgba
    getter success : Hue::Theme::Rgba
    getter warning : Hue::Theme::Rgba
    getter danger : Hue::Theme::Rgba

    def initialize(raw : Hash(String, String))
      surface = Hue::Theme.parse!(raw["surface"])
      @surface = surface.over(Hue::Theme::WHITE)
      pick = ->(k : String) { Hue::Theme.parse!(raw[k]).over(@surface) }
      @sidebar = pick.call("sidebar")
      @card = pick.call("card")
      @line = pick.call("line")
      @text = pick.call("text")
      @dim = pick.call("dim")
      @accent = pick.call("accent")
      @accent_text = pick.call("accentText")
      @glass = pick.call("glass")
      @control = pick.call("control")
      @success = pick.call("success")
      @warning = pick.call("warning")
      @danger = pick.call("danger")
    end
  end

  # One palette with the variant, mode and slider positions it was made with.
  class Candidate
    getter base : Array(String)
    getter? light : Bool
    getter mode : String
    getter adjustments : Hash(String, Float64)
    @final : Array(String)? = nil
    @tokens : Tokens? = nil
    @checked = false

    def initialize(@base : Array(String), @light : Bool, @mode : String, @adjustments = {} of String => Float64)
    end

    # The 16 colours after the sliders.
    def final : Array(String)
      @final ||= @base.map { |c| Hue::Color.adjust(c, @adjustments) }
    end

    # The 13 tokens, or nil when no readable theme can be made from these colours.
    def raw_tokens : Hash(String, String)?
      Hue::Theme.tokens(final, !@light)
    rescue Hue::Theme::Invalid
      nil
    end

    def tokens : Tokens?
      return @tokens if @checked
      @checked = true
      @tokens = raw_tokens.try { |t| Tokens.new(t) }
    end

    def readable? : Bool
      !tokens.nil?
    end

    def variant : String
      @light ? "light" : "dark"
    end

    # The look this candidate becomes when it is applied or saved.
    def to_look(name : String, wallpaper : String?, source : String = "picture") : Hue::Look
      l = Hue::Look.new(name, @base.map(&.downcase), variant, @mode, source, wallpaper)
      @adjustments.each { |k, v| l.adjustments[k] = v unless v == Hue::Color::SLIDERS[k][2] }
      l
    end
  end

  # What was read from one picture: enough to make a palette for any mode and variant.
  class Analysis
    # Pictures are read at most this long on a side: the colours come from a 400 px copy anyway, and a
    # vector wallpaper drawn at 3840 px takes seconds.
    MAX_SIDE = 1200

    getter path : String
    getter dominant : Array(String)
    getter counts : Array(Int32)

    @palettes = {} of {Bool, String} => Array(String)

    def initialize(@path, @dominant, @counts)
    end

    # Reads the picture (the slow part: decode, sample, quantise). Raises Hue::ImageError or
    # Hue::Extract::Failed with a sentence about the file.
    def self.read(path : String) : Analysis
      raise Hue::ImageError.new("#{path}: no such file") unless File.file?(path)
      pic = Hue::Picture.load(path, MAX_SIDE)
      Looks.trace("decoded #{pic.width}x#{pic.height}")
      pixels = Hue::Extract.sample(pic)
      Looks.trace("sampled #{pixels.size} pixels")
      dom, counts = Hue::Extract.dominant(pixels)
      Looks.trace("quantised")
      new(path, dom, counts)
    end

    # The 16 colours for a mode and variant (made once, then kept).
    def palette(light : Bool, mode : String) : Array(String)
      @palettes[{light, mode}] ||= Hue::Engine.from_dominant(@dominant, @counts, light, mode)
    end

    # All sixteen candidates, dark then light for each mode, with the slider positions.
    def candidates(adjustments : Hash(String, Float64)) : Array(Candidate)
      out = [] of Candidate
      MODES.each do |m|
        {false, true}.each do |light|
          out << Candidate.new(palette(light, m), light, m, adjustments)
        end
      end
      out
    end
  end

  # The sentence for a picture that could not be read.
  def self.picture_problem(ex : Exception) : String
    case ex
    when Hue::ImageError      then "This picture could not be read. Try a PNG, JPEG or WebP file."
    when Hue::Extract::Failed then "This picture has too few colours to make a look from."
    else                           "This picture could not be used."
    end
  end
end

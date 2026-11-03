# SPDX-License-Identifier: GPL-2.0-or-later
# Looks in the real headless engine: pictures drawn from code, a fake system (file panels), a fake
# desktop that remembers what was applied, a fixed clock, and the golden files (screenshots with the
# pinned fonts, accessibility trees). WOLKIT_BLESS=1 rewrites them.
require "spec"
require "file_utils"
require "../../../../shards/wolkit/src/wolkit/test"
require "../../src/looks"
require "../support/fakes"
require "../../../../../hue/spec/support/scenes"

module LooksSpec
  NOW = Time.local(2026, 11, 3, 15, 0, 0)

  # One folder for the example: pictures, store, previews, panels' answers.
  class Env
    getter dir : String

    def initialize
      @dir = File.join(Dir.tempdir, "looks-engine-#{Random::Secure.hex(6)}")
      Dir.mkdir_p(@dir, 0o700)
    end

    def picture(name : String) : String
      path = File.join(@dir, "pics", "#{name}.png")
      unless File.file?(path)
        FileUtils.mkdir_p(File.dirname(path))
        Scenes.scene(name).save(path)
      end
      path
    end

    def store : Looks::Store
      Looks::Store.new(File.join(@dir, "data", "state.json"), File.join(@dir, "looks"))
    end

    def previews : Looks::Previews
      Looks::Previews.new(File.join(@dir, "cache", "previews"))
    end

    def strips : Looks::Previews
      Looks::Previews.new(File.join(@dir, "cache", "strips"))
    end

    def clean : Nil
      FileUtils.rm_rf(@dir)
    end
  end

  class FixedPictures < Looks::PictureSource
    def initialize(@list : Array(Looks::Pic))
    end

    def load(&done : Array(Looks::Pic) ->) : Nil
      done.call(@list)
    end
  end

  # Answers for the panels, recorded.
  class FakeSystem < Looks::System
    getter opened = [] of String
    getter saved = [] of {String, String}
    property handed : Looks::Handed? = nil
    property refusal : String? = nil
    property save_result : String? = nil

    def open(title : String, &done : Looks::Handed?, String? ->) : Nil
      @opened << title
      done.call(@handed, @refusal)
    end

    def save(name : String, text : String, &done : String? ->) : Nil
      @saved << {name, text}
      done.call(@save_result)
    end
  end

  # The app of one example. The work that would go to the background pool runs at once, unless the
  # example holds it (the "reading" state).
  class App < Looks::App
    class_property env : Env = Env.new
    class_property applier : FakeApplier = FakeApplier.new
    class_property system : FakeSystem = FakeSystem.new
    class_property pictures : Array(String) = [] of String
    class_property? hold : Bool = false
    class_property? compact : Bool = false
    class_property size : {Int32, Int32} = {Looks::App::WIDTH, Looks::App::HEIGHT}
    class_getter held = [] of Proc(Nil)

    def self.prepare(env : Env, pictures : Array(String) = [] of String, size = {Looks::App::WIDTH, Looks::App::HEIGHT}, hold = false, compact = false) : App.class
      @@env = env
      @@applier = FakeApplier.new
      @@system = FakeSystem.new
      @@pictures = pictures
      @@size = size
      @@hold = hold
      @@compact = compact
      @@held.clear
      self
    end

    def initialize
      list = @@pictures.map { |p| Looks::Pic.new(p, Looks::Pictures.title(p), nil) }
      super(@@env.store, @@applier, @@system, FixedPictures.new(list), -> { NOW }, @@env.previews, @@env.strips, @@size, @@compact)
    end

    def view
      super
      start_up
    end

    def offload(work : -> T, &done : T | Exception ->) : Nil forall T
      job = -> { done.call(Wolkit.capture { work.call }); nil }
      @@hold ? @@held << job : job.call
    end
  end

  # The folders of the XDG environment point into the example's folder, so nothing touches the
  # person's own.
  def self.with_home(env : Env, & : ->) : Nil
    keys = {"XDG_DATA_HOME" => "data", "XDG_CACHE_HOME" => "cache", "XDG_STATE_HOME" => "state", "XDG_CONFIG_HOME" => "config"}
    old = keys.keys.to_h { |k| {k, ENV[k]?} }
    keys.each { |k, d| ENV[k] = File.join(env.dir, d) }
    begin
      yield
    ensure
      old.each { |k, v| v ? (ENV[k] = v) : ENV.delete(k) }
    end
  end

  def self.run(env : Env, pictures : Array(String) = [] of String, size = {Looks::App::WIDTH, Looks::App::HEIGHT}, hold = false, compact = false, & : Wolkit::Test::Session, App ->) : Nil
    with_home(env) do
      Wolkit::Test.run(App.prepare(env, pictures, size, hold, compact)) do |t|
        yield t, t.app.as(App)
      end
    end
  ensure
    env.clean
  end

  def self.scenes(env : Env, names = %w(sunset forest ocean)) : Array(String)
    names.map { |n| env.picture(n) }
  end

  # Compare with (or, with WOLKIT_BLESS=1, write) spec/golden/<name>.
  def self.golden(name : String, actual : String | Bytes) : Nil
    path = File.join(__DIR__, "..", "golden", name)
    bytes = actual.is_a?(String) ? actual.to_slice : actual
    if ENV["WOLKIT_BLESS"]?
      Dir.mkdir_p(File.dirname(path))
      File.write(path, bytes)
    else
      File.exists?(path).should be_true, "#{path} is missing; run WOLKIT_BLESS=1 make spec and review it"
      File.read(path).to_slice.should eq(bytes), "#{path} differs; run WOLKIT_BLESS=1 make spec and review the diff"
    end
  end

  # One state: the screenshot and the accessibility tree.
  def self.pin(t : Wolkit::Test::Session, name : String) : Nil
    golden("#{name}.png", t.screenshot)
    golden("#{name}.a11y", t.a11y_snapshot)
  end

  # The number of parts in the window (the Kit backend takes at most 200).
  def self.parts(t : Wolkit::Test::Session) : Int32
    t.scene.each_line.count(&.matches?(/^\s*\d+ \w+/))
  end

  # The deepest level of the window's tree (the window itself is level 0). The Kit backend takes
  # parts down to level 11, counting from the window's content.
  def self.depth(t : Wolkit::Test::Session) : Int32
    t.scene.each_line.compact_map { |l| l.match(/^( *)\d+ \w+/).try { |m| m[1].size // 2 } }.max? || 0
  end

  # Both limits of the Kit backend at once.
  def self.fits_kit?(t : Wolkit::Test::Session) : Bool
    parts(t) < 200 && depth(t) <= 10
  end
end

Wolkit.on_error { |ex| STDERR.puts ex.inspect_with_backtrace }

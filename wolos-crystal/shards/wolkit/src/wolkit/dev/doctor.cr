# SPDX-License-Identifier: BSD-2-Clause
# The doctor: "why does my window not show?" answered by checks, each with the sentence that says
# what to do. `wolkit-doctor` (bin/wolkit-doctor.cr) runs the checks of the machine: the runtime
# folder, a display or the Kit host socket, the two libraries. `Wolkit::Doctor.check_app(app)` runs
# the app's view against a recorder (no engine, no display) and checks what it made: a window, shown,
# with content, within the Kit backend's limits.
#
# Concept: every check is a pure function of the environment, the file system and the patch stream,
# so the doctor is tested with made-up environments and never needs the session it diagnoses.
require "../test/recorder"
require "../guard"

module Wolkit
  module Doctor
    enum Level
      Ok
      Warn
      Fail
    end

    # One result: what was checked, how it went and, unless it is fine, what to do.
    record Finding, level : Level, check : String, detail : String

    # The Kit backend's limits (docs/modules/libwolkit.md, Backends).
    KIT_PARTS  = 200
    KIT_LEVELS =  12

    LIBRARIES = {"libwolkit.so.1", "libwol.so.1"}

    # The checks of the machine. `env` is the environment; `exists` and `mode` read the file system
    # (specs pass their own).
    def self.environment(env : Hash(String, String) = ENV.to_h,
                         exists : Proc(String, Bool) = ->(path : String) { File.exists?(path) },
                         mode : Proc(String, Int32?) = ->(path : String) { File.info?(path).try(&.permissions.value.to_i) }) : Array(Finding)
      found = [] of Finding
      runtime = env["XDG_RUNTIME_DIR"]?
      if runtime.nil? || runtime.empty?
        found << Finding.new(Level::Fail, "runtime folder", "XDG_RUNTIME_DIR is not set, so the app cannot reach the display or the dev tools. Start the app from a WolOS session, not from a plain ssh or su shell.")
      elsif !exists.call(runtime)
        found << Finding.new(Level::Fail, "runtime folder", "XDG_RUNTIME_DIR is #{runtime}, which does not exist. Log in again so the session makes it.")
      elsif (m = mode.call(runtime)) && m & 0o077 != 0
        found << Finding.new(Level::Warn, "runtime folder", "#{runtime} can be read by other users (mode #{m.to_s(8)}); the session should make it 0700.")
      else
        found << Finding.new(Level::Ok, "runtime folder", runtime)
      end

      wayland = env["WAYLAND_DISPLAY"]?
      kit = env["WOL_KIT_SOCKET"]?
      if env.has_key?("WOLKIT_ENGINE_HEADLESS")
        found << Finding.new(Level::Warn, "display", "WOLKIT_ENGINE_HEADLESS is set: the engine draws into memory and never shows a window. Unset it to see the app.")
      elsif kit && !kit.empty?
        if exists.call(kit)
          found << Finding.new(Level::Ok, "display", "the Kit host at #{kit}")
        else
          found << Finding.new(Level::Fail, "display", "WOL_KIT_SOCKET names #{kit}, which does not exist: the shell is not running or this is another session's socket.")
        end
      elsif wayland && !wayland.empty?
        socket = wayland.starts_with?('/') ? wayland : File.join(runtime || "", wayland)
        if exists.call(socket)
          found << Finding.new(Level::Ok, "display", "Wayland display #{wayland}")
        else
          found << Finding.new(Level::Fail, "display", "WAYLAND_DISPLAY is #{wayland}, but #{socket} does not exist. The compositor is not running in this session.")
        end
      else
        found << Finding.new(Level::Fail, "display", "Neither WAYLAND_DISPLAY nor WOL_KIT_SOCKET is set, so there is nowhere to show a window. Run the app in the WolOS desktop; for tests, use Wolkit::Test.run, which needs no display.")
      end

      dirs = library_dirs(env)
      LIBRARIES.each do |library|
        if at = dirs.find { |dir| exists.call(File.join(dir, library)) }
          found << Finding.new(Level::Ok, library, File.join(at, library))
        else
          package = library.starts_with?("libwolkit") ? "libwolkit" : "libwol"
          found << Finding.new(Level::Fail, library, "#{library} is not in #{dirs.join(", ")}. Install the #{package} package, or build it with `make all` in the shard and set WOLKIT_BUILD.")
        end
      end

      if fonts = env["WOLKIT_FONT_DIR"]?
        found << (exists.call(fonts) ? Finding.new(Level::Ok, "fonts", fonts) : Finding.new(Level::Warn, "fonts", "WOLKIT_FONT_DIR is #{fonts}, which does not exist; text is drawn as missing-glyph boxes."))
      end
      if styles = env["WOLKIT_DEV_STYLES"]?
        missing = styles.split(':').reject { |p| p.empty? || exists.call(p) }
        found << (missing.empty? ? Finding.new(Level::Ok, "style sheets", styles) : Finding.new(Level::Warn, "style sheets", "WOLKIT_DEV_STYLES names #{missing.join(", ")}, which do not exist; those sheets are not watched."))
      end
      found
    end

    # Where the dynamic loader looks, in the doctor's order: WOLKIT_BUILD, LD_LIBRARY_PATH, the system.
    def self.library_dirs(env : Hash(String, String)) : Array(String)
      dirs = [] of String
      env["WOLKIT_BUILD"]?.try { |d| dirs << d unless d.empty? }
      env["LD_LIBRARY_PATH"]?.try { |p| dirs.concat(p.split(':').reject(&.empty?)) }
      dirs.concat(["/usr/lib", "/usr/lib64", "/usr/local/lib"])
      dirs.uniq
    end

    # Run `app`'s view against a recorder and check what it made. The app is mounted and forgotten.
    def self.check_app(app : App) : Array(Finding)
      found = [] of Finding
      # Whatever the view raises is the finding (Wolkit.capture is the one place that catches all).
      recorder = Wolkit.capture { Test.record(app) }
      if recorder.is_a?(Exception)
        Test.reset
        return [Finding.new(Level::Fail, "view", "The view raised #{recorder.class}: #{recorder.message}. Fix that first; the window is never made.")]
      end
      begin
        windows = recorder.windows
        if windows.empty?
          found << Finding.new(Level::Fail, "window", "The view made no window. Put the content in `window \"Title\" do ... end` or `app_window \"Title\" do ... end`.")
          return found
        end
        windows.each do |window|
          name = window.title.inspect
          unless window.shown?
            found << Finding.new(Level::Fail, "window #{name}", "The window was made but never shown. Make it with the `window` or `app_window` element, which shows it once the view has run.")
          end
          parts, levels = shape(window.ops)
          if parts == 0
            found << Finding.new(Level::Warn, "window #{name}", "The window is empty: its block made no element, so it opens as a blank rectangle.")
          else
            found << Finding.new(Level::Ok, "window #{name}", "#{parts} parts, #{levels} levels deep")
          end
          if parts > KIT_PARTS
            found << Finding.new(Level::Warn, "Kit backend", "#{parts} parts: where the engine draws through the Kit host (no native backend) a window may hold #{KIT_PARTS}, so it will not open there. Use `table` or `vlist` for long lists.")
          end
          if levels > KIT_LEVELS
            found << Finding.new(Level::Warn, "Kit backend", "#{levels} levels of nesting: the Kit host takes #{KIT_LEVELS}. Flatten rows and columns that hold one element.")
          end
        end
      ensure
        Test.reset
      end
      found
    end

    # Parts made and how deep the tree goes, read from the ops (`CREATE id kind`, `INSERT parent child ...`).
    def self.shape(ops : Array(String)) : {Int32, Int32}
      parent = {} of String => String
      made = 0
      ops.each do |op|
        words = op.split(' ')
        case words[0]?
        when "CREATE" then made += 1
        when "INSERT" then parent[words[2]] = words[1] if words.size > 2
        when "FREE"   then made -= 1
        end
      end
      deepest = 0
      parent.each_key do |node|
        depth = 0
        at = node
        while (up = parent[at]?) && depth <= 4096
          depth += 1
          at = up
        end
        deepest = depth if depth > deepest
      end
      {made, deepest}
    end

    # The findings as lines: `ok`, `warn` or `FAIL`, the check, the detail.
    def self.report(findings : Array(Finding)) : String
      String.build do |io|
        findings.each do |f|
          word = case f.level
                 in .ok?   then "ok  "
                 in .warn? then "warn"
                 in .fail? then "FAIL"
                 end
          io << word << "  " << f.check << ": " << f.detail << '\n'
        end
      end
    end

    # True when nothing failed.
    def self.healthy?(findings : Array(Finding)) : Bool
      findings.none?(&.level.fail?)
    end
  end
end

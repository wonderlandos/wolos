# SPDX-License-Identifier: BSD-2-Clause
# Live `.wolcss` in a dev build: the running app watches its style sheets and, when one is saved,
# compiles them again with the engine's compiler and restyles every window, with no rebuild and no
# restart. A sheet that does not compile leaves the last good look in place and shows the error, as
# `file:line:column`, in the dev banner; the next good save clears it. Dev builds only (`-Ddev`).
#
# Which sheets: `$WOLKIT_DEV_STYLES`, a `:`-separated list of files and folders (a folder adds its
# `*.wolcss`, sorted), in the order the app's `make styles` compiles them: the kit sheet first
# (`build/00-kit.wolcss` in the template), then the app's (`src/styles`). `wol app run --watch`
# sets it; unset, nothing is watched.
#
# Concept: the sheets are compiled as one source (the engine's verb takes one), so the line in an
# error is a line of the joined text; `StyleWatch.locate` turns it back into the sheet's own file
# and line. How often: the files' modification times are read every 250 ms, a few `stat` calls.
{% if flag?(:dev) %}
{% verbatim do %}
  require "../runtime"
  require "./errors"

  module Wolkit
    module Dev
      module StyleWatch
        # One sheet in the joined source: its path and the joined line its first line became.
        record Part, path : String, first_line : Int32, lines : Int32

        @@paths = [] of String
        @@stamps = {} of String => Time
        @@running = false
        @@last_error : String? = nil

        # The sheets named by `$WOLKIT_DEV_STYLES`.
        def self.paths_from_env : Array(String)
          expand((ENV["WOLKIT_DEV_STYLES"]? || "").split(':').reject(&.empty?))
        end

        # Files stay as given; a folder becomes its `*.wolcss`, sorted.
        def self.expand(entries : Array(String)) : Array(String)
          entries.flat_map do |entry|
            Dir.exists?(entry) ? Dir.glob(File.join(entry, "*.wolcss")).sort : [entry]
          end
        end

        # Watch `paths` (files or folders; the environment's by default) every `every`. Returns false when there is
        # nothing to watch.
        def self.start(paths : Array(String) = paths_from_env, every : Time::Span = 250.milliseconds) : Bool
          @@paths = expand(paths)
          @@stamps = stamps
          return false if @@paths.empty? || @@running
          @@running = true
          spawn(name: "wolkit-style-watch") do
            while @@running
              sleep every
              check
            end
          end
          true
        end

        def self.stop : Nil
          @@running = false
        end

        # The sheets being watched.
        def self.paths : Array(String)
          @@paths
        end

        # The last error shown, or nil after a good compile.
        def self.last_error : String?
          @@last_error
        end

        # Reload when a sheet changed since the last look. Returns true when it reloaded.
        def self.check : Bool
          now = stamps
          return false if now == @@stamps
          @@stamps = now
          reload
          true
        end

        # Compile the sheets and set the table on every window; on an error keep the old look and
        # show where the error is.
        def self.reload : Nil
          source, parts = join(@@paths)
          bytes = FFI::StyleTable.compile(source)
          Wolkit.batch { Server.load_style(bytes) }
          if @@last_error
            @@last_error = nil
            Banner.clear
          end
        rescue ex : Wol::Error | UsageError | IO::Error
          message = locate(ex.message.to_s.lines.first? || "The style sheets do not compile.", parts || [] of Part)
          @@last_error = message
          Banner.show(message)
        end

        # The sheets as one source, and where each one starts in it.
        def self.join(paths : Array(String)) : {String, Array(Part)}
          parts = [] of Part
          line = 1
          source = String.build do |io|
            paths.each do |path|
              text = File.read(path)
              text += "\n" unless text.ends_with?('\n')
              count = text.count('\n')
              parts << Part.new(path, line, count)
              line += count
              io << text
            end
          end
          {source, parts}
        end

        # `style.wolcss:LINE:COL: ...` (a line of the joined source) as `sheet:line:COL: ...`.
        def self.locate(message : String, parts : Array(Part)) : String
          m = message.match(/\A[^:\s]+:(\d+):(\d+): (.*)\z/m) || return message
          joined = m[1].to_i
          part = parts.find { |p| joined >= p.first_line && joined < p.first_line + p.lines } || return message
          "#{part.path}:#{joined - part.first_line + 1}:#{m[2]}: #{m[3]}"
        end

        private def self.stamps : Hash(String, Time)
          @@paths.each_with_object({} of String => Time) do |path, acc|
            info = File.info?(path)
            acc[path] = info ? info.modification_time : Time::UNIX_EPOCH
          end
        end
      end
    end
  end
{% end %}
{% end %}

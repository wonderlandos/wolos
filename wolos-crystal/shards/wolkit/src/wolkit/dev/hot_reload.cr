# SPDX-License-Identifier: BSD-2-Clause
# The dev server: a running app in a dev build (`-Ddev`) listens on a socket only its owner can
# use (`$XDG_RUNTIME_DIR/wolkit-dev-<pid>.sock`, in the user's 0700 runtime directory) and takes
# JSON lines from a client (plan-wolkit D2 and D5 plan `wol app run --watch` and `wol app inspect`;
# neither ships yet):
#
#   {"cmd":"style","data":"<base64 WSB1>"}   load a compiled style table into every window, no restart
#   {"cmd":"error","message":"..."}         show the build error banner
#   {"cmd":"clear"}                         remove the banner
#   {"cmd":"snapshot","path":"..."}         write the app's state to a file for the next process
#   {"cmd":"inspect","what":"scene|layout|a11y"}   reply with the dump of the first window
#
# The server also watches the app's `.wolcss` sheets (`$WOLKIT_DEV_STYLES`, dev/style_watch.cr) and
# restyles on save, so a style change needs no CLI at all.
#
# Release builds contain none of this code. A line is at most 8 MiB (a style table is at most 4 MiB
# before base64) and a connection that stops talking is dropped after 30 s (RCR-191, RCR-192).
{% if flag?(:dev) %}
{% verbatim do %}
  require "json"
  require "base64"
  require "socket"
  require "../runtime"
  require "./errors"
  require "./snapshot"
  require "./style_watch"

  module Wolkit
    module Dev
      MAX_LINE = 8 * 1024 * 1024

      # One command from the CLI.
      struct Command
        include JSON::Serializable

        getter cmd : String
        getter data : String? = nil
        getter message : String? = nil
        getter path : String? = nil
        getter what : String? = nil
      end

      module Server
        @@server : UNIXServer? = nil
        @@path : String? = nil

        # The socket path for this process.
        def self.socket_path : String
          dir = ENV["XDG_RUNTIME_DIR"]? || raise UsageError.new("The dev server needs XDG_RUNTIME_DIR, the user's runtime directory.")
          File.join(dir, "wolkit-dev-#{Process.pid}.sock")
        end

        # Start listening (called by `Wolkit.run` in dev builds, with the running app).
        def self.start(app : App) : Nil
          path = socket_path
          File.delete(path) if File.exists?(path)
          server = UNIXServer.new(path)
          File.chmod(path, 0o600)
          @@server = server
          @@path = path
          Snapshot.restore_from_env(app)
          StyleWatch.start
          spawn(name: "wolkit-dev") do
            while client = server.accept?
              spawn(name: "wolkit-dev-client") { serve(client, app) }
            end
          end
        end

        def self.stop : Nil
          StyleWatch.stop
          @@server.try &.close
          @@path.try { |path| File.delete(path) if File.exists?(path) }
          @@server = nil
        end

        private def self.serve(client : UNIXSocket, app : App) : Nil
          client.read_timeout = 30.seconds
          while line = client.gets(MAX_LINE)
            client.puts(handle(line, app))
          end
        rescue IO::Error
          # The CLI went away or stopped talking; nothing to answer.
        ensure
          client.close
        end

        # Run one command line; the reply is one JSON line.
        def self.handle(line : String, app : App) : String
          command = Command.from_json(line)
          case command.cmd
          when "style"    then reply { load_style(Base64.decode(command.data || "")) }
          when "error"    then reply { Banner.show(command.message || "The build failed.") }
          when "clear"    then reply { Banner.clear }
          when "snapshot" then reply { Snapshot.save(app, command.path || raise UsageError.new("snapshot needs a path")) }
          when "inspect"  then {ok: true, text: inspect_text(command.what || "scene")}.to_json
          else                 {ok: false, error: "Unknown command #{command.cmd.inspect}."}.to_json
          end
        rescue ex : JSON::Error
          {ok: false, error: "Not a command: #{ex.message}"}.to_json
        end

        private def self.reply(& : ->) : String
          Wolkit.batch { yield }
          {ok: true}.to_json
        rescue ex : Wol::Error | UsageError | ArgumentError
          {ok: false, error: ex.message}.to_json
        end

        # Compile a style table and set it on every window.
        def self.load_style(bytes : Bytes) : Nil
          backend = Runtime.backend.as(EngineBackend)
          table = FFI::StyleTable.load(backend.engine, bytes)
          Runtime.windows.each { |window| window.backend.as(EngineWindow).handle.style_table = table }
        end

        # The dump of the first window: the scene, the layout or the accessibility tree.
        def self.inspect_text(what : String) : String
          window = Runtime.windows.first?.try(&.backend.as(EngineWindow).handle) || return "No window is open."
          case what
          when "layout" then window.dump(FFI::Window::LAYOUT)
          when "a11y"   then window.dump(FFI::Window::A11Y)
          else               window.dump(FFI::Window::SCENE)
          end
        end
      end
    end
  end
{% end %}
{% end %}

# SPDX-License-Identifier: BSD-2-Clause
# State snapshot for code reload: before a rebuilt process takes over, the old one writes every signal
# of the app whose value can be written as JSON (by `Class#ivar` name and type), and the new one
# reads them back, so editing a view does not lose what you typed or where you were (plan-wolkit D2).
# Dev builds only. `persisted` signals do not need it: they come back from the store.
{% if flag?(:dev) %}
{% verbatim do %}
  require "json"
  require "../app"

  module Wolkit
    module Dev
      module Snapshot
        # Where the new process finds the old one's state (`WOLKIT_DEV_SNAPSHOT=<file>`).
        ENV_KEY = "WOLKIT_DEV_SNAPSHOT"

        # The JSON of the app's signals, `{"Todos#items": "...", ...}`.
        def self.take(app : App) : String
          data = {} of String => String
          app.wolkit_snapshot(data)
          data.to_json
        end

        # Put saved values back. A name or a type that no longer fits is skipped.
        def self.restore(app : App, text : String) : Int32
          data = Hash(String, String).from_json(text)
          app.wolkit_restore(data)
        rescue JSON::Error
          0
        end

        def self.save(app : App, path : String) : Nil
          File.write(path, take(app), perm: 0o600)
        end

        def self.restore_from_env(app : App) : Int32
          path = ENV[ENV_KEY]?
          return 0 unless path && File.file?(path)
          restore(app, File.read(path))
        end
      end
    end

    abstract class View
      # Fill `into` with the JSON of each plain signal this class holds (written for each subclass by
      # the `finished` hook below; the default holds none).
      def wolkit_snapshot(into : Hash(String, String)) : Nil
      end

      # Set each saved signal; returns how many were restored.
      def wolkit_restore(from : Hash(String, String)) : Int32
        0
      end

      # Each concrete view class gets its own pair, so the macro sees that class's instance variables
      # (a method body is expanded with the concrete type, where they are known).
      macro finished
        {% for sub in ::Wolkit::App.all_subclasses %}
          {% unless sub.abstract? %}
            class ::{{ sub.name.id }}
              def wolkit_snapshot(into : Hash(String, String)) : Nil
                \{% for iv in @type.instance_vars %}
                  \{% if iv.type.name.starts_with?("Wolkit::Signal(") %}
                    into[\{{ @type.name.stringify + "#" + iv.name.stringify }}] = @\{{ iv.name.id }}.peek.to_json
                  \{% end %}
                \{% end %}
              end

              def wolkit_restore(from : Hash(String, String)) : Int32
                restored = 0
                \{% for iv in @type.instance_vars %}
                  \{% if iv.type.name.starts_with?("Wolkit::Signal(") %}
                    if text = from[\{{ @type.name.stringify + "#" + iv.name.stringify }}]?
                      begin
                        @\{{ iv.name.id }}.value = \{{ iv.type.type_vars[0] }}.from_json(text)
                        restored += 1
                      rescue JSON::Error
                      end
                    end
                  \{% end %}
                \{% end %}
                restored
              end
            end
          {% end %}
        {% end %}
      end
    end
  end
{% end %}
{% end %}

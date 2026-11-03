# SPDX-License-Identifier: BSD-2-Clause
# Wolkit.run: start an App against the real engine. It creates the engine, mounts the app's view
# in one batch, sends the first patches, and hands the thread to the loop until the last window
# closes. The app id defaults to the program name under org.wolos.
require "./app"
require "./engine_backend"

module Wolkit
  # The reverse-DNS id of this program: `$WOLKIT_APP_ID`, or `org.wolos.<program name>`.
  def self.default_app_id : String
    ENV["WOLKIT_APP_ID"]? || "org.wolos.#{Path[Process.executable_path || PROGRAM_NAME].basename.gsub(/[^A-Za-z0-9]/, "")}"
  end

  # Run `app_class` until its windows are closed.
  #
  # ```
  # Wolkit.run Todos
  # ```
  def self.run(app_class : T.class, app_id : String = default_app_id, flags : UInt32 = 0_u32) : Nil forall T
    engine = FFI::Engine.create(app_id, flags)
    begin
      Store.current = PrefsStore.new if Store.current.is_a?(MemoryStore)
      Runtime.backend = EngineBackend.new(engine)
      app = app_class.new
      app.start
      Frame.flush_all
      {% if flag?(:dev) %}
        Dev::Server.start(app)
        Dev::Hud.start
      {% end %}
      Loop.run(engine)
    ensure
      {% if flag?(:dev) %}
        Dev::Server.stop
      {% end %}
      Runtime.reset
      Services.close
      Scope.reset_root
      Runtime.backend = nil
      engine.close
    end
  end
end

# SPDX-License-Identifier: BSD-2-Clause
# The dev tools, in a dev build (`crystal spec -Ddev`; `make spec-engine` runs this file that way).
# Without -Ddev there is nothing to test: release builds contain none of this code.
require "./engine_helper_spec"
require "../dsl_app_spec"

{% if flag?(:dev) %}
  describe Wolkit::Dev do
    it "takes a snapshot of the app's signals and puts it back" do
      Wolkit::Test.run(SpecTodos) do |t|
        t.type "milk", into: "What needs doing?"
        t.click "Add"
        app = t.app.as(SpecTodos)
        text = Wolkit::Dev::Snapshot.take(app)
        text.should contain("SpecTodos#draft")
        fresh = SpecTodos.new
        Wolkit::Dev::Snapshot.restore(fresh, %({"SpecTodos#draft":"\\"typed\\""})).should eq 1
        fresh.draft.peek.should eq "typed"
      end
    end

    it "answers commands on its socket and shows a build error banner" do
      ENV["XDG_RUNTIME_DIR"] = Dir.tempdir
      Wolkit::Test.run(SpecTodos) do |t|
        app = t.app.as(SpecTodos)
        Wolkit::Dev::Server.start(app)
        begin
          path = Wolkit::Dev::Server.socket_path
          File.info(path).permissions.should eq File::Permissions.new(0o600)
          client = UNIXSocket.new(path)
          client.puts %({"cmd":"error","message":"src/views/todo.cr:12: text_field has no property `placehodler`."})
          client.gets.to_s.should eq %({"ok":true})
          t.scene.should contain("overlay")
          client.puts %({"cmd":"inspect","what":"a11y"})
          client.gets.to_s.should contain("textbox")
          client.puts %({"cmd":"clear"})
          client.gets.to_s.should eq %({"ok":true})
          t.scene.should_not contain("overlay")
          bytes = Wolkit::FFI::StyleTable.compile(".greeting { font-size: title; color: accent; }")
          client.puts %({"cmd":"style","data":"#{Base64.strict_encode(bytes)}"})
          client.gets.to_s.should eq %({"ok":true})
          client.puts %({"cmd":"style","data":"bm90IGEgdGFibGU="})
          client.gets.to_s.should contain(%("ok":false))
          client.puts %({"cmd":"nonsense"})
          client.gets.to_s.should contain(%("ok":false))
          client.close
        ensure
          Wolkit::Dev::Server.stop
        end
      end
    end
  end
{% end %}

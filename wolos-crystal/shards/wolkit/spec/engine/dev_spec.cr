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

    it "restyles when a watched sheet is saved and points at the line of a broken one" do
      dir = File.join(Dir.tempdir, "wolkit-style-watch-#{Process.pid}")
      Dir.mkdir_p(dir)
      kit = File.join(dir, "00-kit.wolcss")
      app_sheet = File.join(dir, "app.wolcss")
      File.write(kit, File.read(File.join(__DIR__, "../../styles/kit.wolcss")))
      File.write(app_sheet, ".greeting { font-size: title; }\n")
      Wolkit::Test.run(SpecTodos) do |t|
        Wolkit::Dev::StyleWatch.start([dir], every: 1.hour).should be_true
        Wolkit::Dev::StyleWatch.paths.should eq [kit, app_sheet]
        Wolkit::Dev::StyleWatch.check.should be_false
        File.write(app_sheet, ".greeting {\n  font-size: title;\n  colour: accent;\n}\n")
        File.touch(app_sheet, Time.utc + 1.second)
        Wolkit::Dev::StyleWatch.check.should be_true
        Wolkit::Dev::StyleWatch.last_error.to_s.should start_with("#{app_sheet}:3:")
        t.scene.should contain("overlay")
        File.write(app_sheet, ".greeting { font-size: title; color: accent; }\n")
        File.touch(app_sheet, Time.utc + 2.seconds)
        Wolkit::Dev::StyleWatch.check.should be_true
        Wolkit::Dev::StyleWatch.last_error.should be_nil
        t.scene.should_not contain("overlay")
      ensure
        Wolkit::Dev::StyleWatch.stop
      end
    ensure
      FileUtils.rm_rf(dir) if dir
    end

    it "maps a line of the joined sheets back to its file" do
      parts = [Wolkit::Dev::StyleWatch::Part.new("a.wolcss", 1, 10), Wolkit::Dev::StyleWatch::Part.new("b.wolcss", 11, 5)]
      Wolkit::Dev::StyleWatch.locate("style.wolcss:13:4: error[WS-001]: unknown", parts).should eq "b.wolcss:3:4: error[WS-001]: unknown"
      Wolkit::Dev::StyleWatch.locate("something else", parts).should eq "something else"
    end
  end
{% end %}

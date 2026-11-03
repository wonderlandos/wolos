# SPDX-License-Identifier: BSD-2-Clause
# The app template: names are filled in, unsafe names are refused. That a generated project builds, is
# formatted and passes its own spec is checked by spec/scripts/template.sh and template_engine.sh (the
# compiler is started from a script, not from a spec: RCR-021).
require "./spec_helper"
require "../src/wolkit/template"

describe Wolkit::Template do
  it "fills in the names and drops the .tmpl suffix" do
    target = File.join(Dir.tempdir, "wolkit-template-#{Process.pid}-a")
    begin
      files = Wolkit::Template.render("my-notes", target)
      files.should contain("shard.yml")
      files.should contain("src/app.cr")
      files.should contain("src/views/main.cr")
      files.should contain("spec/app_spec.cr")
      files.should contain("assets/.keep")
      files.none?(&.ends_with?(".tmpl")).should be_true
      File.read(File.join(target, "shard.yml")).should contain("name: my-notes")
      File.read(File.join(target, "src/app.cr")).should contain("class MyNotes < Wolkit::App")
      File.read(File.join(target, "README.md")).should contain("org.wolos.mynotes")
      Dir.glob(File.join(target, "**/*")).each do |path|
        next unless File.file?(path)
        File.read(path).should_not contain("{{")
      end
    ensure
      FileUtils.rm_rf(target)
    end
  end

  it "refuses a name that is not safe" do
    expect_raises(ArgumentError, /lower-case letters/) { Wolkit::Template.render("../evil", File.join(Dir.tempdir, "x")) }
    expect_raises(ArgumentError, /lower-case letters/) { Wolkit::Template.render("Big", File.join(Dir.tempdir, "x")) }
  end

  it "refuses to write over a directory" do
    expect_raises(ArgumentError, /exists already/) { Wolkit::Template.render("todos", Dir.tempdir) }
  end
end

# SPDX-License-Identifier: BSD-2-Clause
# The app template: a generated project must type-check, be formatted, and (against the engine, in
# spec/engine/template_spec.cr) build and pass its own spec.
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

  it "type-checks and formats a generated project" do
    target = File.join(Dir.tempdir, "wolkit-template-#{Process.pid}-b")
    begin
      Wolkit::Template.render("demo", target)
      shards = File.expand_path("../..", __DIR__)
      path = "#{shards}:#{`crystal env CRYSTAL_PATH`.strip}"
      output = IO::Memory.new
      status = Process.run("crystal", ["build", "--no-codegen", "--error-on-warnings", "src/main.cr"], chdir: target, env: {"CRYSTAL_PATH" => path}, output: output, error: output)
      status.success?.should be_true, output.to_s
      output = IO::Memory.new
      status = Process.run("crystal", ["tool", "format", "--check", "src", "spec"], chdir: target, output: output, error: output)
      status.success?.should be_true, output.to_s
    ensure
      FileUtils.rm_rf(target)
    end
  end
end

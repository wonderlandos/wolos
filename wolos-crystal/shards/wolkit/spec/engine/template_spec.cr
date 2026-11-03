# SPDX-License-Identifier: BSD-2-Clause
# The app template against the real engine: a project made by `wolkit-new` builds, and its own spec
# (a headless click test) passes. `$WOLKIT_BUILD` is the directory with libwolkit.so.1 and libwol.so.1
# (`make spec-engine` sets it to build/).
require "./engine_helper_spec"
require "../../src/wolkit/template"

describe "the app template" do
  it "builds and passes its own spec" do
    build = ENV["WOLKIT_BUILD"]? || File.expand_path("../../build", __DIR__)
    target = File.join(Dir.tempdir, "wolkit-template-#{Process.pid}-engine")
    begin
      Wolkit::Template.render("demo", target)
      shards = File.expand_path("../../..", __DIR__)
      env = {"CRYSTAL_PATH" => "#{shards}:#{`crystal env CRYSTAL_PATH`.strip}"}
      link = "-L#{build} -lwol -Wl,-rpath,#{build}"
      Dir.mkdir_p(File.join(target, "bin"))
      output = IO::Memory.new
      status = Process.run("crystal", ["build", "--error-on-warnings", "--link-flags", link, "src/main.cr", "-o", "bin/demo"], chdir: target, env: env, output: output, error: output)
      status.success?.should be_true, output.to_s
      File.exists?(File.join(target, "bin/demo")).should be_true
      output = IO::Memory.new
      status = Process.run("crystal", ["spec", "--error-on-warnings", "--link-flags", link], chdir: target, env: env, output: output, error: output)
      status.success?.should be_true, output.to_s
      output.to_s.should contain("1 examples, 0 failures")
    ensure
      FileUtils.rm_rf(target)
    end
  end
end

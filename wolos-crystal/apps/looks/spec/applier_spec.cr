# SPDX-License-Identifier: GPL-2.0-or-later
# The applier: the theme folder it writes is the one `wol-hue` writes, the id rule holds, and the
# only process it starts is `wol theme set -- <id>` (a stand-in `wol` on the PATH records it).
require "./spec_helper"

private BASE = %w(#101820 #d04a4a #4ad08a #d0b04a #4a7ad0 #b04ad0 #4ad0d0 #e8e8e8 #303840 #ff6a6a #6aff9a #ffd06a #6a9aff #d06aff #6affff #ffffff)

private def with_env(vars : Hash(String, String?), & : ->) : Nil
  old = vars.keys.to_h { |k| {k, ENV[k]?} }
  vars.each { |k, v| v ? (ENV[k] = v) : ENV.delete(k) }
  begin
    yield
  ensure
    old.each { |k, v| v ? (ENV[k] = v) : ENV.delete(k) }
  end
end

describe Looks do
  it "builds theme ids the engine accepts" do
    Looks.theme_id("Desert day").should eq "looks-desert-day"
    Looks.theme_id("../../x; rm").should eq "looks-x-rm"
    Looks.theme_id("a" * 100).size.should be <= 64
    Looks.theme_id("***").should match(Hue::Theme::ID)
  end
end

describe Looks::Desktop do
  it "writes the theme folder and runs wol with a fixed argument list" do
    with_tmpdir do |dir|
      bin = File.join(dir, "bin")
      FileUtils.mkdir_p(bin)
      log = File.join(dir, "wol.log")
      File.write(File.join(bin, "wol"), "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '#{log}'\n")
      File.chmod(File.join(bin, "wol"), 0o755)
      cfg = File.join(dir, "config")
      with_env({"XDG_CONFIG_HOME" => cfg, "PATH" => "#{bin}:#{ENV["PATH"]}"}) do
        look = Hue::Look.new("Desert day", BASE, "dark", "normal", "picture", "/p/d.png")
        look.adjustments["vibrance"] = 10.0
        Looks::Desktop.new.apply(look, "looks-desert-day").should be_nil
        theme = File.join(cfg, "wolos", "themes", "looks-desert-day")
        j = JSON.parse(File.read(File.join(theme, "theme.json")))
        j["id"].as_s.should eq "looks-desert-day"
        j["variant"].as_s.should eq "dark"
        j["tokens"].as_h.keys.sort!.should eq Hue::Theme::TOKENS.sort
        j["terminal"].as_a.size.should eq 16
        Hue::Look.read(File.join(theme, "look.json")).adjustments.should eq({"vibrance" => 10.0})
        File.read(log).should eq "theme set -- looks-desert-day\n"
        Hue::Theme.check(Hash(String, String).from_json(j["tokens"].to_json)).should be_empty
      end
    end
  end

  it "says so when wol is missing or fails, and refuses an id with a space or a dot" do
    with_tmpdir do |dir|
      cfg = File.join(dir, "config")
      look = Hue::Look.new("X", BASE, "dark")
      with_env({"XDG_CONFIG_HOME" => cfg, "PATH" => dir}) do
        said(Looks::Desktop.new.apply(look, "looks-x")).should contain("wol command is missing")
        said(Looks::Desktop.new.switch("looks x")).should eq Looks::Desktop::WOL_FAILED
        said(Looks::Desktop.new.switch("../x")).should eq Looks::Desktop::WOL_FAILED
        said(Looks::Desktop.new.apply(look, "bad id")).should contain("could not be written")
        Dir.exists?(File.join(cfg, "wolos", "themes", "bad id")).should be_false
      end
      File.write(File.join(dir, "wol"), "#!/bin/sh\nexit 3\n")
      File.chmod(File.join(dir, "wol"), 0o755)
      with_env({"XDG_CONFIG_HOME" => cfg, "PATH" => dir}) do
        said(Looks::Desktop.new.apply(look, "looks-x")).should eq Looks::Desktop::WOL_FAILED
      end
    end
  end

  it "reads the theme in use from the file wol writes" do
    with_tmpdir do |dir|
      with_env({"XDG_CONFIG_HOME" => dir}) do
        Looks::Desktop.new.current.should be_nil
        FileUtils.mkdir_p(File.join(dir, "wolos"))
        File.write(File.join(dir, "wolos", "theme"), "wol-dark\n")
        Looks::Desktop.new.current.should eq "wol-dark"
      end
    end
  end
end

describe Looks::Dirs do
  it "resolves the folders from an environment" do
    env = {"HOME" => "/h", "XDG_DATA_HOME" => "/d", "XDG_CACHE_HOME" => "rel", "XDG_STATE_HOME" => "/s"}
    Looks::Dirs.app_data(env).should eq "/d/org.wolos.looks"
    Looks::Dirs.cache(env).should eq "/h/.cache/org.wolos.looks"
    Looks::Dirs.night_shift_flag(env).should eq "/s/wolos/toggles/org.wolos.nightlight"
    Looks::Dirs.active_theme_file(env).should eq "/h/.config/wolos/theme"
    Looks::Dirs.wallpapers(env).should eq ["/usr/share/wolos/wallpapers", "/d/wolos/wallpapers"]
  end

  it "reads the Pictures folder from the folder settings and refuses odd values" do
    with_tmpdir do |dir|
      FileUtils.mkdir_p(File.join(dir, ".config"))
      env = {"HOME" => dir}
      Looks::Dirs.pictures(env).should eq File.join(dir, "Pictures")
      File.write(File.join(dir, ".config", "user-dirs.dirs"), %(XDG_PICTURES_DIR="$HOME/Photos"\n))
      Looks::Dirs.pictures(env).should eq File.join(dir, "Photos")
      File.write(File.join(dir, ".config", "user-dirs.dirs"), %(XDG_PICTURES_DIR="$HOME/../etc"\n))
      Looks::Dirs.pictures(env).should eq File.join(dir, "Pictures")
    end
  end
end

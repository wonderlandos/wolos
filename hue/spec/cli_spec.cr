# SPDX-License-Identifier: GPL-2.0-or-later
# The built binary end to end, with private XDG folders and a stand-in `wol` that records its
# arguments. Skipped when WOL_HUE_BIN is not set (`make spec` sets it).
require "./spec_helper"

BIN = ENV["WOL_HUE_BIN"]?

class Sandbox
  getter root : String
  getter env : Hash(String, String?)

  def initialize
    @root = SpecHelp.tmpdir
    bin = File.join(@root, "bin")
    Dir.mkdir_p(bin)
    File.write(File.join(bin, "wol"), "#!/bin/sh\necho \"$@\" >> \"#{@root}/wol.log\"\n")
    File.chmod(File.join(bin, "wol"), 0o755)
    @env = {
      "XDG_CONFIG_HOME" => File.join(@root, "config"), "XDG_CACHE_HOME" => File.join(@root, "cache"),
      "XDG_DATA_HOME" => File.join(@root, "data"), "PATH" => "#{bin}:#{ENV["PATH"]}",
    } of String => String?
  end

  def run(*args : String) : {Int32, String, String}
    o = IO::Memory.new
    e = IO::Memory.new
    st = Process.run(BIN || raise("WOL_HUE_BIN is not set"), args.to_a, env: @env, output: o, error: e)
    {st.exit_code, o.to_s, e.to_s}
  end

  def wol_log : String
    f = File.join(@root, "wol.log")
    File.exists?(f) ? File.read(f) : ""
  end

  def files(under : String) : Array(String)
    Dir.glob(File.join(@root, under, "**", "*")).select { |f| File.file?(f) }.map(&.sub(File.join(@root, under) + "/", "")).sort!
  end
end

describe "wol-hue (binary)" do
  next unless BIN

  it "makes a theme from a picture, writes only its theme folder and switches through wol" do
    s = Sandbox.new
    pic = SpecHelp.scene_files["sunset"]
    code, res, err = s.run("from-wallpaper", pic, "--mode", "pastel", "--json")
    {code, err}.should eq({0, ""})
    j = JSON.parse(res)
    j["id"].should eq("picture-sunset")
    j["applied"].should be_true
    j["cached"].should be_false
    j["terminal"].as_a.size.should eq(16)
    s.files("config").should eq(["wolos/themes/picture-sunset/look.json", "wolos/themes/picture-sunset/theme.json"])
    s.wol_log.should eq("theme set -- picture-sunset\n")
    _, out2, _ = s.run("from-wallpaper", pic, "--mode", "pastel", "--json", "--no-apply")
    JSON.parse(out2)["cached"].should be_true
    JSON.parse(out2)["terminal"].should eq(j["terminal"])
  end

  it "writes but does not switch with --no-apply, and --light makes its own id" do
    s = Sandbox.new
    code, res, _ = s.run("from-wallpaper", SpecHelp.scene_files["desert"], "--light", "--no-apply")
    code.should eq(0)
    res.should contain("picture-desert-light")
    s.wol_log.should eq("")
    JSON.parse(File.read(File.join(s.root, "config/wolos/themes/picture-desert-light/theme.json")))["variant"].should eq("light")
  end

  it "adjusts from the original colours, saves, lists and applies looks" do
    s = Sandbox.new
    s.run("from-wallpaper", SpecHelp.scene_files["forest"], "--id", "woods", "--no-apply")
    theme = File.join(s.root, "config/wolos/themes/woods/theme.json")
    before = JSON.parse(File.read(theme))["terminal"]
    code, _, err = s.run("adjust", "woods", "--hue-shift", "120", "--no-apply")
    {code, err}.should eq({0, ""})
    shifted = JSON.parse(File.read(theme))["terminal"]
    shifted.should_not eq(before)
    s.run("adjust", "woods", "--hue-shift", "120", "--no-apply")
    JSON.parse(File.read(theme))["terminal"].should eq(shifted) # positions, not steps
    s.run("adjust", "woods", "--reset", "--no-apply")
    JSON.parse(File.read(theme))["terminal"].should eq(before)
    s.run("adjust", "woods", "--gamma", "3")[0].should eq(2)
    s.run("save", "Green Room", "--theme", "woods")[0].should eq(0)
    _, list, _ = s.run("list", "--json")
    JSON.parse(list).as_a.map(&.["name"]).should eq(["Green Room"])
    code, _, _ = s.run("apply", "Green Room")
    code.should eq(0)
    s.wol_log.should eq("theme set -- look-green-room\n")
  end

  it "imports a colors.toml" do
    s = Sandbox.new
    f = File.join(s.root, "night.toml")
    File.write(f, (0..15).map { |i| "color#{i} = \"##{i < 8 ? "1a1b26" : "c0caf5"}\"" }.join("\n").sub("color1 = \"#1a1b26\"", "color1 = \"#f7768e\"").sub("color7 = \"#1a1b26\"", "color7 = \"#c0caf5\""))
    code, res, err = s.run("import", "colors", f, "--no-apply", "--json")
    {code, err}.should eq({0, ""})
    j = JSON.parse(res)
    j["id"].should eq("night")
    j["variant"].should eq("dark")
    j["tokens"]["danger"].should eq("#f7768e")
  end

  it "fails clearly" do
    s = Sandbox.new
    s.run("from-wallpaper")[0].should eq(2)
    s.run("from-wallpaper", "/nope.png", "--no-apply")[2].should contain("no such file")
    s.run("from-wallpaper", SpecHelp.scene_files["ocean"], "--mode", "neon")[0].should eq(2)
    s.run("frobnicate")[0].should eq(2)
    s.run("from-wallpaper", SpecHelp.scene_files["ocean"], "--id", "../../etc")[0].should eq(1)
    s.run("adjust", "missing", "--vibrance", "10")[0].should eq(2)
  end
end

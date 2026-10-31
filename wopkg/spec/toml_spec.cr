require "spec"
require "../src/wopkg/toml"

describe Wopkg::Toml do
  it "reads the recipe subset" do
    t = Wopkg::Toml.parse(<<-T)
      name = "hvm" # trailing comment
      version = '2.0.22'
      jobs = 4
      cached = true
      needs = ["cargo",
               "cc"]  # across lines
      [[source]]
      url = "https://x/a#b"
      [[source]]
      url = "https://x/c"
      [build]
      run = ["cargo build --release \\"quoted\\""]
      T
    t["name"].should eq("hvm")
    t["version"].should eq("2.0.22")
    t["jobs"].should eq(4)
    t["cached"].should eq(true)
    t["needs"].should eq(["cargo", "cc"])
    t["source"].as(Array)[0].as(Hash)["url"].should eq("https://x/a#b")
    t["source"].as(Array).size.should eq(2)
    t["build"].as(Hash)["run"].should eq([%(cargo build --release "quoted")])
  end

  it "refuses what it does not understand, with the line" do
    {
      "a = 1\na = 2"      => /line 2: a is set twice/,
      "x"                 => /line 1: expected key = value/,
      "a = \"open"        => /unfinished string/,
      "a = 1.5e3"         => /unexpected text/,
      "[t]\n[t]"          => /appears twice/,
      "a = {inline = 1}"  => /unsupported value/,
      "a = [1 2]"         => /expected , or \]/,
    }.each do |text, err|
      expect_raises(Wopkg::Toml::ParseError, err) { Wopkg::Toml.parse(text) }
    end
  end
end

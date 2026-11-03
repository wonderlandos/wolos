# SPDX-License-Identifier: GPL-2.0-or-later
# The .wollook file and the imports: a round trip, hostile input, Base16 and colors.toml.
require "./spec_helper"

private BASE = %w(#101820 #d04a4a #4ad08a #d0b04a #4a7ad0 #b04ad0 #4ad0d0 #e8e8e8 #303840 #ff6a6a #6aff9a #ffd06a #6a9aff #d06aff #6affff #ffffff)

describe Looks::Wollook do
  it "round-trips a look and leaves the wallpaper path out" do
    l = Hue::Look.new("Dune day", BASE, "light", "pastel", "picture", "/p/dune.png", "#4a7ad0")
    l.adjustments["vibrance"] = 12.5
    text = Looks::Wollook.dump(l)
    text.should_not contain("/p/dune.png")
    JSON.parse(text)["format"].as_s.should eq "wollook"
    back = Looks::Wollook.parse(text)
    back.name.should eq "Dune day"
    back.colors.should eq BASE
    back.variant.should eq "light"
    back.mode.should eq "pastel"
    back.accent.should eq "#4a7ad0"
    back.adjustments.should eq({"vibrance" => 12.5})
    back.wallpaper.should be_nil
    back.source.should eq "imported"
  end

  it "refuses what is not a look, with a sentence" do
    expect_raises(Looks::Wollook::Invalid, /not a valid .wollook/) { Looks::Wollook.parse("{nope") }
    expect_raises(Looks::Wollook::Invalid, /not a valid .wollook/) { Looks::Wollook.parse("[1,2]") }
    expect_raises(Looks::Wollook::Invalid, /not a .wollook/) do
      Looks::Wollook.parse(%({"format":"other","version":1,"name":"x","variant":"dark","colors":[]}))
    end
    expect_raises(Looks::Wollook::Invalid, /newer version/) do
      Looks::Wollook.parse(%({"format":"wollook","version":9,"name":"x","variant":"dark","colors":[]}))
    end
    expect_raises(Looks::Wollook::Invalid, /cannot be used/) do
      Looks::Wollook.parse(%({"format":"wollook","version":1,"name":"x","variant":"dark","colors":["#000000"]}))
    end
    expect_raises(Looks::Wollook::Invalid, /too large/) { Looks::Wollook.parse(" " * 70_000) }
  end

  it "refuses a slider outside its range and a colour that is not one" do
    l = Hue::Look.new("x", BASE, "dark")
    doc = JSON.parse(Looks::Wollook.dump(l)).as_h
    doc["adjustments"] = JSON.parse(%({"vibrance": 900}))
    expect_raises(Looks::Wollook::Invalid, /outside/) { Looks::Wollook.parse(doc.to_json) }
    doc["adjustments"] = JSON.parse(%({}))
    doc["colors"] = JSON.parse(BASE.map { "red" }.to_json)
    expect_raises(Looks::Wollook::Invalid, /not colours/) { Looks::Wollook.parse(doc.to_json) }
  end

  it "cleans a hostile name into one the store accepts" do
    l = Hue::Look.new("../../etc/passwd", BASE, "dark")
    text = Looks::Wollook.dump(l)
    Looks::Wollook.parse(text).name.should eq "etc passwd"
    Looks::Names.clean("").should eq "Imported look"
    Looks::Names.clean("***").should eq "Imported look"
    Looks::Names.clean("a" * 100).size.should eq 64
    Looks::Names.clean("  Sea   glass  ").should eq "Sea glass"
  end

  it "picks the next free name" do
    Looks::Names.unique("Dune", ["Sea"]).should eq "Dune"
    Looks::Names.unique("Dune", ["dune"]).should eq "Dune 2"
    Looks::Names.unique("Dune", ["Dune", "Dune 2", "dune 3"]).should eq "Dune 4"
    Looks::Names.unique("x" * 64, ["x" * 64]).size.should eq 64
  end
end

describe Looks::Importer do
  base16 = <<-Y
    scheme: "Ocean"
    author: "Someone"
    base00: "2b303b"
    base01: "343d46"
    base02: "4f5b66"
    base03: "65737e"
    base04: "a7adba"
    base05: "c0c5ce"
    base06: "dfe1e8"
    base07: "eff1f5"
    base08: "bf616a"
    base09: "d08770"
    base0A: "ebcb8b"
    base0B: "a3be8c"
    base0C: "96b5b4"
    base0D: "8fa1b3"
    base0E: "b48ead"
    base0F: "ab7967"
    Y

  it "reads a Base16 scheme" do
    with_tmpdir do |dir|
      l = Looks::Importer.read("ocean.yaml", base16, File.join(dir, "scratch"))
      l.name.should eq "Ocean"
      l.colors.size.should eq 16
      l.variant.should eq "dark"
      l.source.should eq "base16"
      Dir.children(File.join(dir, "scratch")).should be_empty
    end
  end

  it "reads colors.toml" do
    with_tmpdir do |dir|
      toml = BASE.each_with_index.map { |c, i| "color#{i} = \"#{c}\"" }.join("\n") + "\nbackground = \"#101820\"\nforeground = \"#e8e8e8\"\n"
      l = Looks::Importer.read("sea-glass.toml", toml, File.join(dir, "s"))
      l.colors.size.should eq 16
      l.source.should eq "colors"
    end
  end

  it "reads a .wollook by its name and refuses other kinds" do
    with_tmpdir do |dir|
      text = Looks::Wollook.dump(Hue::Look.new("Via file", BASE, "dark"))
      Looks::Importer.read("x.wollook", text, dir).name.should eq "Via file"
      expect_raises(Looks::Wollook::Invalid, /reads .wollook/) { Looks::Importer.read("x.png", "..", dir) }
      expect_raises(Looks::Wollook::Invalid, /could not be read/) { Looks::Importer.read("x.yaml", "not: [valid", dir) }
      expect_raises(Looks::Wollook::Invalid, /could not be read/) { Looks::Importer.read("x.toml", "nothing", dir) }
      expect_raises(Looks::Wollook::Invalid, /too large/) { Looks::Importer.read("x.yaml", "a" * 70_000, dir) }
    end
  end
end

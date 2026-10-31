require "spec"
require "../src/wopkg/ui"

describe Wopkg::UI do
  it "names sources the way people know them" do
    s = ->(k : String, r : String) { JSON.parse(%({"kind":"#{k}","repo":"#{r}"})) }
    Wopkg::UI.source_label(s.call("pacman", "wolos")).should eq("WolOS")
    Wopkg::UI.source_label(s.call("pacman", "extra")).should eq("Arch (extra)")
    Wopkg::UI.source_label(s.call("pacman", "wolos-cachy")).should eq("WolOS optimized (wolos-cachy)")
    Wopkg::UI.source_label(s.call("aur", "aur")).should eq("AUR (unreviewed)")
  end

  it "draws a progress bar and sizes" do
    Wopkg::UI.bar(512_i64, 1024_i64, 4).should eq("[##..] 50% 512 B/1.0 KiB")
    Wopkg::UI.bytes(3 * 1024 * 1024).should eq("3.0 MiB")
  end

  it "asks once; Enter means yes" do
    Wopkg::UI.confirm("Go?", false, IO::Memory.new("\n")).should be_true
    Wopkg::UI.confirm("Go?", false, IO::Memory.new("n\n")).should be_false
    Wopkg::UI.confirm("Go?", false, IO::Memory.new("")).should be_false
    Wopkg::UI.confirm("Go?", true, IO::Memory.new("n\n")).should be_true
  end
end

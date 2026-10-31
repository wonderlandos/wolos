require "spec"
require "../src/wopkg/args"

describe Wopkg::Args do
  it "maps pacman's short forms and plain words to the same commands" do
    {
      %w(-S firefox)       => {"install", ["firefox"]},
      %w(install firefox)  => {"install", ["firefox"]},
      %w(-Ss web browser)  => {"search", ["web", "browser"]},
      %w(search web)       => {"search", ["web"]},
      %w(-Si vim)          => {"info", ["vim"]},
      %w(-Qi vim)          => {"info", ["vim"]},
      %w(-Syu)             => {"update", [] of String},
      %w(-Sy)              => {"sync", [] of String},
      %w(-Rns vim)         => {"remove", ["vim"]},
      %w(rm vim)           => {"remove", ["vim"]},
      %w(-Q)               => {"list", [] of String},
      %w(undo 4)           => {"rollback", ["4"]},
      [] of String         => {"help", [] of String},
    }.each do |argv, (verb, targets)|
      a = Wopkg::Args.parse(argv)
      {argv, a.verb, a.targets}.should eq({argv, verb, targets})
    end
  end

  it "reads flags anywhere" do
    a = Wopkg::Args.parse(%w(update --to 1.1 -y --json --only flatpak,appimage))
    {a.verb, a.to, a.yes, a.json, a.only}.should eq({"update", "1.1", true, true, ["flatpak", "appimage"]})
    Wopkg::Args.parse(%w(-Qu)).check.should be_true
    Wopkg::Args.parse(%w(install --from flatpak gimp)).from.should eq("flatpak")
    Wopkg::Args.parse(%w(-S -- -weird)).targets.should eq(["-weird"])
  end

  it "says what is wrong in one sentence" do
    expect_raises(Wopkg::UsageError, /needs a name/) { Wopkg::Args.parse(%w(install)) }
    expect_raises(Wopkg::UsageError, /not a wopkg command/) { Wopkg::Args.parse(%w(frobnicate)) }
    expect_raises(Wopkg::UsageError, /Unknown option/) { Wopkg::Args.parse(%w(-X)) }
    expect_raises(Wopkg::UsageError, /--to goes with/) { Wopkg::Args.parse(%w(search x --to 1.1)) }
    expect_raises(Wopkg::UsageError, /needs a value/) { Wopkg::Args.parse(%w(update --to)) }
  end
end

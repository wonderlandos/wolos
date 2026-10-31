require "spec"
require "../src/wopkg/ui"

describe Wopkg::UI, "deliberate answers" do
  it "needs the word yes, ignores -y, and treats Enter as no" do
    Wopkg::UI.confirm_strict("Go on?", IO::Memory.new("yes\n")).should be_true
    Wopkg::UI.confirm_strict("Go on?", IO::Memory.new("YES\n")).should be_true
    Wopkg::UI.confirm_strict("Go on?", IO::Memory.new("y\n")).should be_false
    Wopkg::UI.confirm_strict("Go on?", IO::Memory.new("\n")).should be_false
    Wopkg::UI.confirm_strict("Go on?", IO::Memory.new("")).should be_false
  end
end
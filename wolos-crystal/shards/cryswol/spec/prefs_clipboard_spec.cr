# SPDX-License-Identifier: BSD-2-Clause
# Preferences, the clipboard and sharing against the fake wold scripted by
# spec/fixtures/prefs_clipboard.json.
require "./spec_helper"

describe "Wol::Prefs, Wol::Clipboard and Wol::Share" do
  it "sets, reads, lists and removes preferences; an unset key is nil" do
    Wol::Spec.with_fake("prefs_clipboard") do |ctx, fake|
      prefs = Wol::Prefs.new(ctx)
      (prefs["width"] = "640").should eq "640"
      prefs["width"]?.should eq "640"
      prefs["height"]?.should be_nil
      expect_raises(KeyError) { prefs["height"] }
      prefs.keys.should eq ["height", "width"]
      prefs.delete("width")
      fake.call_tags.should eq ["prefs.set", "prefs.get", "prefs.get", "prefs.get", "prefs.keys", "prefs.remove"]
      fake.calls[0]["key"].as_s.should eq "width"
      fake.calls[0]["value"].as_s.should eq "640"
    end
  end

  it "refuses a bad key and an over-long value with InvalidArgument before asking wold" do
    Wol::Spec.with_fake("prefs_clipboard") do |ctx, fake|
      prefs = Wol::Prefs.new(ctx)
      expect_raises(Wol::InvalidArgument) { prefs["no spaces"] = "x" }
      expect_raises(Wol::InvalidArgument) { prefs["k"] = "x" * 4097 }
      expect_raises(Wol::InvalidArgument) { prefs["k"] = "a\0b" }
      fake.call_tags.should be_empty
    end
  end

  it "reads and writes the clipboard" do
    Wol::Spec.with_fake("prefs_clipboard") do |ctx, fake|
      Wol::Clipboard.set_text(ctx, "héllo")
      Wol::Clipboard.text(ctx).should eq "copied"
      fake.call_tags.should eq ["clipboard.write", "clipboard.read"]
      fake.calls[0]["text"].as_s.should eq "héllo"
      expect_raises(Wol::InvalidArgument) { Wol::Clipboard.set_text(ctx, "x" * (48 * 1024 + 1)) }
    end
  end

  it "shares text and maps a refusal" do
    Wol::Spec.with_fake("prefs_clipboard") do |ctx, fake|
      Wol::Share.text(ctx, "Note", "body").should eq "copied"
      expect_raises(Wol::PermissionDenied, "Sharing is turned off for this app.") do
        Wol::Share.text(ctx, "Note", "body")
      end
      fake.calls[0]["title"].as_s.should eq "Note"
    end
  end
end

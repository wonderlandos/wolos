# SPDX-License-Identifier: BSD-2-Clause
# The answer verbs (system, power, network, audio, account, devices, search query) against the fake
# wold scripted by spec/fixtures/answers.json.
require "./spec_helper"

describe "Wol answers" do
  it "reads the system information without the system service" do
    Wol::Spec.with_fake("answers") do |ctx, fake|
      info = Wol::System.info(ctx)
      info.os_name.should_not be_empty
      info.kernel.should_not be_empty
      info.accent_hex.should match(/\A#[0-9a-f]{6}\z/)
      (info.appearance.light? || info.appearance.dark?).should be_true
      fake.call_tags.should be_empty
    end
  end

  it "reads power, and a missing key is nil" do
    Wol::Spec.with_fake("answers") do |ctx, fake|
      info = Wol::Power.info(ctx)
      info["on_battery"].as_bool.should be_true
      info["low_power"].as_bool.should be_false
      info["battery"]["percent"].as_i?.should eq 71
      info["battery"]["minutes_left"]?.should be_nil
      info["nothing"]?.should be_nil
      fake.call_tags.should eq ["power.info"]
    end
  end

  it "keeps the computer awake and maps a refusal to PermissionDenied with libwol's sentence" do
    Wol::Spec.with_fake("answers") do |ctx, fake|
      Wol::Power.keep_awake(ctx, "Exporting the film") { :done }.should eq :done
      ex = expect_raises(Wol::PermissionDenied) { Wol::Power.keep_awake(ctx, "Again") }
      ex.message.should eq "Keeping the computer awake is turned off for this app."
      ex.code.should eq LibWol::WOL_ERR_PERMISSION_DENIED
      fake.calls.first["reason"].as_s.should eq "Exporting the film"
      fake.call_tags.should eq ["power.keep-awake", "power.keep-awake"]
    end
  end

  it "lists the network interfaces" do
    Wol::Spec.with_fake("answers") do |ctx, _fake|
      info = Wol::Network.info(ctx)
      info["interfaces"].size.should eq 2
      info["interfaces"].map(&.["kind"].as_s).should eq ["ethernet", "wifi"]
      info["interfaces"][1]["up"].as_bool.should be_false
      Wol::Network.online?(ctx).should be_true
    end
  end

  it "reads and sets the audio output" do
    Wol::Spec.with_fake("answers") do |ctx, fake|
      info = Wol::Audio.info(ctx)
      info["volume"].as_f.should eq 40.0
      info["now_playing"]["title"].as_s.should eq "Song"
      Wol::Audio.set_volume(ctx, 55)
      Wol::Audio.set_muted(ctx, true)
      fake.call_tags.should eq ["audio.info", "audio.set-volume", "audio.set-muted"]
      fake.calls[1]["percent"].as_i.should eq 55
      fake.calls[2]["muted"].as_bool.should be_true
    end
  end

  it "refuses a volume out of range in libwol, not in Crystal" do
    Wol::Spec.with_fake("answers") do |ctx, fake|
      expect_raises(Wol::InvalidArgument) { Wol::Audio.set_volume(ctx, 101) }
      expect_raises(Wol::InvalidArgument) { Wol::Audio.set_volume(ctx, -1) }
      fake.call_tags.should_not contain "audio.set-volume"
    end
  end

  it "reads the account and the device list" do
    Wol::Spec.with_fake("answers") do |ctx, _fake|
      Wol::Account.info(ctx)["full_name"].as_s.should eq "Alice"
      devices = Wol::Device.list(ctx)["devices"]
      devices.map(&.["name"].as_s).should eq ["Scanner", "Headphones"]
      devices[0]["vendor"].as_i?.should eq 1234
      devices[1]["can_open"].as_bool.should be_false
    end
  end

  it "searches, and a child answer stays usable after its parent goes out of scope" do
    Wol::Spec.with_fake("answers") do |ctx, fake|
      first = Wol::Search.query(ctx, "no")["results"][0]
      GC.collect
      first["title"].as_s.should eq "Notes"
      fake.calls.first["text"].as_s.should eq "no"
    end
  end

  it "refuses a child of a closed answer" do
    Wol::Spec.with_fake("answers") do |ctx, _fake|
      info = Wol::Account.info(ctx)
      name = info["name"]
      info.close
      expect_raises(Wol::UsageError) { name.as_s }
    end
  end
end

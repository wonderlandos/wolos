# SPDX-License-Identifier: BSD-2-Clause
# A player against the fake wold scripted by spec/fixtures/media.json: wold sends three remote
# controls after media.publish.
require "./spec_helper"

describe Wol::Player do
  it "publishes, sends the state whole and gets the remote controls" do
    Wol::Spec.with_fake("media") do |ctx, fake|
      seen = [] of Tuple(Wol::Player::Key, Time::Span)
      player = Wol::Media.publish(ctx) { |_p, key, by| seen << {key, by} }
      player.can = Wol::Player::Can::Next | Wol::Player::Can::Seek
      player.track("Song", artist: "Band", album: "Record", length: 215.5.seconds)
      player.art("/home/a/cover.png")
      player.state(:playing, 12.5.seconds)
      GC.collect
      Wol::Spec.run_until(ctx) { seen.size == 3 }.should be_true
      seen.should eq [
        {Wol::Player::Key::Seek, -5.seconds},
        {Wol::Player::Key::Toggle, 0.seconds},
        {Wol::Player::Key::SetPosition, 90.5.seconds},
      ]
      fake.call_tags.first.should eq "media.publish"
      fake.call_tags[1..].uniq!.should eq ["media.state"]
      last = fake.calls.select { |c| c["call"] == "media.state" }.last["state"]
      last["status"].as_s.should eq "playing"
      last["title"].as_s.should eq "Song"
      last["artist"].as_s.should eq "Band"
      last["art"].as_s.should eq "/home/a/cover.png"
      last["length_us"].as_i64.should eq 215_500_000
      last["position_us"].as_i64.should eq 12_500_000
      last["can"].as_i.should eq 5
      player.close
      ctx.rooted_count.should eq 0
    end
  end

  it "refuses a bad value before asking wold, and a second player" do
    Wol::Spec.with_fake("media") do |ctx, fake|
      player = Wol::Media.publish(ctx)
      expect_raises(Wol::InvalidArgument) { player.state(Wol::Player::Status::Playing, -1.second) }
      expect_raises(Wol::InvalidArgument) { player.track("two\nlines") }
      expect_raises(Wol::InvalidArgument) { player.art("cover.png") }
      expect_raises(Wol::InvalidArgument) { Wol::Media.publish(ctx) }
      fake.call_tags.should eq ["media.publish", "media.state"]
      player.close
      expect_raises(Wol::UsageError) { player.state(Wol::Player::Status::Paused) }
    end
  end

  it "says why a player was refused" do
    Wol::Spec.with_fake("media") do |ctx, _fake|
      first = Wol::Media.publish(ctx)
      first.close
      error = expect_raises(Wol::PermissionDenied) { Wol::Media.publish(ctx) }
      error.message.to_s.should contain "Allow it in System Settings"
    end
  end
end

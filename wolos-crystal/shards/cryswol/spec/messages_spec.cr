# SPDX-License-Identifier: BSD-2-Clause
# Messages between apps against the fake wold scripted by spec/fixtures/messages.json: a message
# arrives after ipc.listen, ipc.send answers "pong".
require "./spec_helper"

describe Wol::Endpoint do
  it "receives a message, keeps it and replies after the block returned" do
    Wol::Spec.with_fake("messages") do |ctx, fake|
      kept = [] of Wol::Message
      lent = [] of Wol::Message
      endpoint = Wol::Endpoint.new(ctx, "ping") do |m|
        lent << m
        kept << m.keep
      end
      GC.collect
      Wol::Spec.run_until(ctx) { !kept.empty? }.should be_true
      expect_raises(Wol::UsageError) { lent.first.body }
      message = kept.first
      message.from.should eq "/usr/bin/sender"
      message.body.should eq "hello"
      message.reply("pong")
      message.close
      endpoint.name.should eq "ping"
      endpoint.close
      fake.call_tags.should eq ["ipc.listen", "ipc.reply"]
      fake.calls[0]["name"].as_s.should eq "ping"
      fake.calls[1]["body"].as_s.should eq "pong"
    end
  end

  it "sends and publishes" do
    Wol::Spec.with_fake("messages") do |ctx, fake|
      Wol::Message.send(ctx, "/usr/bin/peer", "ping", "hi").should eq "pong"
      Wol::Message.publish(ctx, "news", "today")
      fake.call_tags.should eq ["ipc.send", "ipc.publish"]
      fake.calls[0]["peer"].as_s.should eq "/usr/bin/peer"
      fake.calls[1]["topic"].as_s.should eq "news"
    end
  end
end

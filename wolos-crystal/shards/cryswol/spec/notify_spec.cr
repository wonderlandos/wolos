# SPDX-License-Identifier: BSD-2-Clause
# Notifications against the fake wold (spec/fixtures/notify.json): the first post gets id 7, the
# second is refused; after a post the bus reports "open" for id 6 (someone else's), "later"
# (a key this notification does not have) and "open" for id 7. Only the last may reach a handler.
require "./spec_helper"

describe Wol::Notification do
  it "posts a notification without buttons" do
    Wol::Spec.with_fake("notify") do |ctx, fake|
      Wol::Notification.post(ctx, "Hello", "from the spec")
      fake.calls.should eq [JSON.parse(%({"call":"notification.post","title":"Hello","body":"from the spec"}))]
    end
  end

  it "raises PermissionDenied with wold's sentence" do
    Wol::Spec.with_fake("notify") do |ctx, _fake|
      Wol::Notification.post(ctx, "First")
      ex = expect_raises(Wol::PermissionDenied, "Notifications are off for this app.") do
        Wol::Notification.new(ctx, "Second", "refused").send
      end
      ex.code.should eq LibWol::WOL_ERR_PERMISSION_DENIED
    end
  end

  it "sends with buttons and runs the handler only for its own id and its own keys" do
    Wol::Spec.with_fake("notify") do |ctx, fake|
      keys = [] of String
      from = [] of Wol::Notification
      note = Wol::Notification.new(ctx, "Download done", "parts.iso")
      note.add_action("open", "Open").on_action do |pressed, key|
        from << pressed
        keys << key
      end
      note.send
      Wol::Spec.run_until(ctx) { !keys.empty? }.should be_true
      Wol::Spec.run_until(ctx, 300.milliseconds) { false }
      keys.should eq ["open"]
      from.should eq [note]
      fake.calls.should eq [JSON.parse(
        %({"call":"notification.post","title":"Download done","body":"parts.iso",) +
        %("actions":[{"key":"open","label":"Open"}]})
      )]
    end
  end

  it "replaces the text of a sent notification and withdraws it" do
    Wol::Spec.with_fake("notify") do |ctx, fake|
      note = Wol::Notification.new(ctx, "Copying", "1 of 3").add_action("open", "Open").send
      note.replace("Copied", "3 of 3").withdraw
      note.title.should eq "Copied"
      note.body.should eq "3 of 3"
      fake.call_tags.should eq %w(notification.post notification.replace notification.withdraw)
      replace = fake.calls[1]
      replace["id"].should eq 7
      replace["title"].should eq "Copied"
      replace["body"].should eq "3 of 3"
      replace["actions"].should eq JSON.parse(%([{"key":"open","label":"Open"}]))
      fake.calls[2].should eq JSON.parse(%({"call":"notification.withdraw","id":7}))
      # Withdrawn: a second withdraw has nothing to take back.
      note.withdraw
      fake.calls.size.should eq 3
    end
  end

  it "uses replaced text when sent later, and withdraws nothing that was never sent" do
    Wol::Spec.with_fake("notify") do |ctx, fake|
      note = Wol::Notification.new(ctx, "Draft")
      note.replace("Final", "body").withdraw
      fake.calls.should be_empty
      note.send
      fake.calls.should eq [JSON.parse(%({"call":"notification.post","title":"Final","body":"body"}))]
    end
  end

  it "refuses bad keys, repeated keys and a fourth button" do
    Wol::Spec.with_fake("notify") do |ctx, fake|
      note = Wol::Notification.new(ctx, "Choose")
      expect_raises(Wol::InvalidArgument) { note.add_action("Bad Key", "Bad") }
      note.add_action("one", "One")
      expect_raises(Wol::InvalidArgument, "Each button needs its own key.") { note.add_action("one", "Again") }
      note.add_action("two", "Two").add_action("default", "Click")
      expect_raises(Wol::InvalidArgument) { note.add_action("four", "Four") }
      expect_raises(Wol::InvalidArgument) { Wol::Notification.new(ctx, "x" * 100_000) }
      fake.calls.should be_empty
    end
  end

  it "hands an exception from the action handler to Loop.on_error" do
    errors = Wol::Spec.capture_errors do |seen|
      Wol::Spec.with_fake("notify") do |ctx, _fake|
        note = Wol::Notification.new(ctx, "Boom").add_action("open", "Open")
        note.on_action { raise "boom from an action" }
        note.send
        Wol::Spec.run_until(ctx) { !seen.empty? }.should be_true
      end
    end
    errors.map(&.message).should eq ["boom from an action"]
  end

  it "stops delivery after clear_action_handler" do
    Wol::Spec.with_fake("notify") do |ctx, _fake|
      keys = [] of String
      note = Wol::Notification.new(ctx, "Quiet").add_action("open", "Open")
      note.on_action { |_, key| keys << key }.clear_action_handler.send
      Wol::Spec.run_until(ctx, 500.milliseconds) { false }
      keys.should be_empty
    end
  end

  it "roots the notification while it has a handler and survives garbage collection" do
    Wol::Spec.with_fake("notify") do |ctx, _fake|
      keys = [] of String
      ctx.rooted_count.should eq 0
      Wol::Notification.new(ctx, "Kept").add_action("open", "Open").on_action { |_, key| keys << key }.send
      ctx.rooted_count.should eq 1
      3.times { GC.collect }
      Wol::Spec.run_until(ctx) { !keys.empty? }.should be_true
      keys.should eq ["open"]
    end
  end

  it "unroots on close and refuses use afterwards" do
    Wol::Spec.with_fake("notify") do |ctx, _fake|
      note = Wol::Notification.new(ctx, "Closing").on_action { }
      ctx.rooted_count.should eq 1
      note.close
      ctx.rooted_count.should eq 0
      expect_raises(Wol::UsageError) { note.send }
    end
  end
end

# SPDX-License-Identifier: BSD-2-Clause
# The loop and callbacks (plan-cr1 §6 "loop and callbacks"): a raise never unwinds into libwol,
# callbacks survive the GC, cancel stops delivery, NULL callbacks reach libwol as NULL.
require "./spec_helper"

private def post(ctx : Wol::Context) : Nil
  t, b = "Hello", "from loop"
  Wol.call { |err| LibWol.wol_notification_post(ctx, t, t.bytesize, b, b.bytesize, err) }
end

# Subscribe from a separate frame so no local of the spec refers to the subscription or its block.
private def subscribe_unreferenced(ctx : Wol::Context, seen : Array(String)) : Nil
  ctx.subscribe("theme") { |e| seen << JSON.parse(e.json)["id"].as_s }
  nil
end

# Many subscriptions whose only Crystal reference is the context's root set.
private def subscribe_many(ctx : Wol::Context, hits : Array(Int32), n : Int32) : Nil
  n.times { |i| ctx.subscribe("theme") { hits[i] += 1 } }
end

# Objects the GC really freed: a class whose finalizer counts, made where nothing keeps them.
class GcProbe
  class_property freed = 0

  def finalize
    GcProbe.freed += 1
  end
end

private def make_probes(n : Int32) : Nil
  n.times { GcProbe.new }
end

private def churn : Nil
  5.times do
    Array.new(2000) { |i| "x" * (i % 50) }
    GC.collect
  end
end

describe Wol::Loop do
  it "routes a raising callback to on_error, returns from dispatch, and keeps running" do
    seen = [] of String
    calls = 0
    errors = Wol::Spec.capture_errors do |errs|
      Wol::Spec.with_fake("core") do |ctx, _|
        ctx.subscribe("theme") do |e|
          id = JSON.parse(e.json)["id"].as_s
          calls += 1
          raise ArgumentError.new("boom #{id}") if calls == 1
          seen << id
        end
        Wol::Spec.run_until(ctx) { seen.size >= 2 }.should be_true
        errs.size.should eq 1
      end
    end
    calls.should eq 3
    errors.size.should eq 1
    errors[0].should be_a ArgumentError
    errors[0].message.should eq "boom wol-dark-blue"
    seen.should eq ["wol-light-blue", "wol-dark-green"]
  end

  it "survives an exception from inside a direct dispatch" do
    errors = Wol::Spec.capture_errors do
      Wol::Spec.with_fake("core") do |ctx, _|
        hits = 0
        ctx.subscribe("theme") { hits += 1; raise "boom" }
        deadline = Time.instant + 3.seconds
        while hits < 3 && Time.instant < deadline
          ctx.dispatch(100)
        end
        hits.should eq 3
      end
    end
    errors.size.should eq 3
  end

  it "keeps callbacks alive through repeated GC runs" do
    Wol::Spec.with_fake("core") do |ctx, _|
      seen = [] of String
      subscribe_unreferenced(ctx, seen)
      churn
      ctx.rooted_count.should eq 1
      Wol::Spec.run_until(ctx) { seen.size >= 3 }.should be_true
      churn
      post(ctx)
      Wol::Spec.run_until(ctx) { seen.includes?("wol-after-post") }.should be_true
      seen.should eq ["wol-dark-blue", "wol-light-blue", "wol-dark-green", "wol-after-post"]
    end
  end

  it "delivers to many unreferenced subscriptions after the GC freed other garbage" do
    Wol::Spec.with_fake("core") do |ctx, _|
      hits = Array.new(64, 0)
      subscribe_many(ctx, hits, 64)
      make_probes(1000)
      churn
      GcProbe.freed.should be > 0
      Wol::Spec.run_until(ctx) { hits.all? { |h| h >= 3 } }.should be_true
      hits.uniq.should eq [3]
    end
  end

  it "stops delivery on cancel and unroots the subscription" do
    Wol::Spec.with_fake("core") do |ctx, _|
      seen = 0
      sub = ctx.subscribe("theme") { seen += 1 }
      sub.cancel
      sub.closed?.should be_true
      ctx.rooted_count.should eq 0
      Wol::Loop.run(ctx, 300.milliseconds)
      seen.should eq 0
      sub.cancel
    end
  end

  it "passes nil as a NULL callback, which libwol refuses with its sentence" do
    Wol::Context.open("org.example.core") do |ctx|
      topic = "theme"
      ex = expect_raises(Wol::InvalidArgument) do
        Wol.out(LibWol::Subscription) do |slot, err|
          LibWol.wol_subscribe(ctx, Wol::Strings.ptr(topic), Wol::Strings.size(topic), nil, nil, slot, err)
        end
      end
      ex.message.should eq "A subscription needs a callback."
    end
  end

  it "refuses a dispatch from inside a callback and reports it" do
    errors = Wol::Spec.capture_errors do
      Wol::Spec.with_fake("core") do |ctx, _|
        hits = 0
        ctx.subscribe("theme") { hits += 1; ctx.dispatch(0) }
        Wol::Spec.run_until(ctx) { hits >= 1 }.should be_true
      end
    end
    errors.first.should be_a Wol::UsageError
  end

  it "expires an event after its callback, and keep holds one longer" do
    Wol::Spec.with_fake("core") do |ctx, _|
      lent = nil
      kept = nil
      ctx.subscribe("theme") do |e|
        lent ||= e
        kept ||= e.keep
      end
      Wol::Spec.run_until(ctx) { !kept.nil? }.should be_true
      old = lent.should be_a(Wol::Event)
      held = kept.should be_a(Wol::Event)
      old.valid?.should be_false
      expect_raises(Wol::UsageError) { old.json }
      held.topic.should eq "theme"
      held.json.should eq %({"event":"theme","id":"wol-dark-blue"})
      held.close
      expect_raises(Wol::UsageError) { held.json }
    end
  end

  it "stops from a callback and from another fiber" do
    Wol::Spec.with_fake("core") do |ctx, _|
      ctx.subscribe("theme") { Wol::Loop.stop(ctx) }
      Wol::Loop.run(ctx).should be_nil
    end
    Wol::Spec.with_fake("core") do |ctx, _|
      started = Time.instant
      spawn { sleep 100.milliseconds; Wol::Loop.stop(ctx) }
      Wol::Loop.run(ctx)
      (Time.instant - started).should be < 2.seconds
    end
  end

  it "returns false when the timeout passes first" do
    Wol::Context.open("org.example.core") do |ctx|
      started = Time.instant
      Wol::Loop.run(ctx, 100.milliseconds) { false }.should be_false
      (Time.instant - started).should be >= 100.milliseconds
    end
  end

  it "refuses a second run of the same context" do
    Wol::Spec.with_fake("core") do |ctx, _|
      refused = false
      ctx.subscribe("theme") do
        Wol::Loop.run(ctx, 10.milliseconds)
      rescue Wol::UsageError
        refused = true
        Wol::Loop.stop(ctx)
      end
      Wol::Loop.run(ctx, 2.seconds)
      refused.should be_true
    end
  end

  it "closes rooted objects when the context closes" do
    sub = nil
    Wol::Spec.with_fake("core") do |ctx, _|
      sub = ctx.subscribe("theme") { }
    end
    sub.should be_a(Wol::Subscription)
    sub.try(&.closed?).should be_true
  end

  it "works across contexts that reuse the same wake fd number" do
    3.times do
      Wol::Spec.with_fake("core") do |ctx, _|
        seen = 0
        ctx.subscribe("theme") { seen += 1 }
        Wol::Spec.run_until(ctx) { seen >= 3 }.should be_true
      end
    end
  end
end

describe "Wol.background" do
  it "runs on another thread with its own context and returns the value" do
    main = Thread.current
    Wol.background("org.example.core") { |bg| {bg.app_id, Thread.current == main} }
      .should eq({"org.example.core", false})
  end

  it "raises the block's exception in the caller" do
    expect_raises(Wol::InvalidArgument) do
      Wol.background("org.example.core") { |bg| Wol::Strings.round_trip(String.new(Bytes[0xff])); bg }
    end
  end
end

# SPDX-License-Identifier: BSD-2-Clause
# Core: versions, error mapping, strings, handles, contexts and answers (plan-cr1 §6 "core").
require "./spec_helper"

# A notification through the raw layer: the trigger for the fixture's "after notification.post".
private def post(ctx : Wol::Context) : Nil
  t, b = "Hello", "from core"
  Wol.call { |err| LibWol.wol_notification_post(ctx, t, t.bytesize, b, b.bytesize, err) }
end

private def power(ctx : Wol::Context) : Wol::Info
  Wol::Info.copy { |slot, err| LibWol.wol_power_copy_info(ctx, slot, err) }
end

describe "Wol versions" do
  it "reports the loaded libwol, at least the ABI the bindings came from" do
    Wol.version.should eq LibWol.wol_version
    major, minor, _ = Wol::ABI_VERSION
    Wol.at_least?(major, minor).should be_true
    Wol.at_least?(major + 1, 0).should be_false
    v = Wol.version
    Wol.version_string.should eq "#{v >> 16}.#{(v >> 8) & 0xff}.#{v & 0xff}"
  end
end

describe Wol::Error do
  it "maps every status to its class and keeps unknown codes as Internal" do
    Wol::Error.from(1, "a").should be_a Wol::InvalidArgument
    Wol::Error.from(2, "a").should be_a Wol::PermissionDenied
    Wol::Error.from(3, "a").should be_a Wol::Unavailable
    Wol::Error.from(4, "a").should be_a Wol::Internal
    unknown = Wol::Error.from(99, "a")
    unknown.should be_a Wol::Internal
    unknown.code.should eq 99
  end

  it "raises Internal with the fallback sentence when libwol gave no error object" do
    ex = expect_raises(Wol::Internal) { Wol.call { |_err| LibWol::WOL_ERR_INTERNAL } }
    ex.message.should eq Wol::FALLBACK_MESSAGE
    Wol.call { |_err| LibWol::WOL_OK }.should be_nil
  end

  it "carries libwol's sentence verbatim for a bad app id" do
    ex = expect_raises(Wol::InvalidArgument) { Wol::Context.open("not an id") }
    ex.code.should eq LibWol::WOL_ERR_INVALID_ARGUMENT
    ex.message.to_s.should start_with "An app id is reverse-DNS"
  end

  it "raises Unavailable when the system service is not there" do
    ENV["WOL_SOCKET"] = "/nonexistent/cryswol/wold.sock"
    Wol::Context.open("org.example.core") do |ctx|
      ex = expect_raises(Wol::Unavailable) { ctx.subscribe("theme") { } }
      ex.message.to_s.should_not be_empty
      ctx.rooted_count.should eq 0
    end
  end
end

describe Wol::Strings do
  it "round-trips text, the empty string and an embedded NUL through libwol" do
    Wol::Strings.round_trip("grüße").should eq "grüße"
    Wol::Strings.round_trip("").should eq ""
    Wol::Strings.round_trip("a\0b").should eq "a\0b"
  end

  it "passes NULL for nil and the empty string" do
    Wol::Strings.ptr(nil).null?.should be_true
    Wol::Strings.ptr("").null?.should be_true
    Wol::Strings.size("añ").should eq 3
    Wol::Strings.size(nil).should eq 0
  end

  it "lets libwol refuse invalid UTF-8" do
    expect_raises(Wol::InvalidArgument) { Wol::Strings.round_trip(String.new(Bytes[0x61, 0xff])) }
  end
end

describe Wol::Context do
  it "opens, refuses use after close, and closes twice" do
    ctx = Wol::Context.open("org.example.core")
    ctx.app_id.should eq "org.example.core"
    ctx.dispatch(0)
    ctx.close
    ctx.closed?.should be_true
    expect_raises(Wol::UsageError) { ctx.dispatch(0) }
    expect_raises(Wol::UsageError) { ctx.subscribe("theme") { } }
    ctx.close
  end

  it "gives the block form's value and closes afterwards" do
    kept = nil
    Wol::Context.open("org.example.core") { |ctx| kept = ctx; 42 }.should eq 42
    kept.should be_a(Wol::Context)
    kept.try(&.closed?).should be_true
  end

  it "refuses an unknown topic with libwol's sentence" do
    Wol::Context.open("org.example.core") do |ctx|
      expect_raises(Wol::InvalidArgument) { ctx.subscribe("no-such-topic") { } }
    end
  end

  it "sends calls through the fake on the real wire" do
    Wol::Spec.with_fake("core") do |ctx, fake|
      post(ctx)
      fake.call_tags.should eq ["notification.post"]
      fake.calls[0]["title"].should eq "Hello"
      fake.problems.should be_empty
    end
  end
end

describe "Wol.dup_fd" do
  it "gives a separate descriptor that Crystal owns, and refuses a bad one" do
    r, w = IO.pipe
    dup = Wol.dup_fd(r.fd)
    dup.fd.should_not eq r.fd
    w.write_byte 7u8
    dup.read_byte.should eq 7u8
    dup.close
    r.closed?.should be_false
    expect_raises(Wol::Internal) { Wol.dup_fd(-1) }
    r.close
    w.close
  end
end

describe Wol::Info do
  it "reads typed values, with nil for what is missing or of another type" do
    Wol::Spec.with_fake("core") do |ctx, _|
      info = power(ctx)
      info["on_battery"].as_bool.should be_true
      info["low_power"].as_bool.should be_false
      info["battery"]["percent"].as_f.should eq 71.0
      info["battery"]["percent"].as_i?.should eq 71
      info["name"].as_s.should eq "Main"
      info["name"].as_f?.should be_nil
      info["nothing"]?.should be_nil
      expect_raises(KeyError) { info["nothing"] }
      cells = info["cells"]
      cells.size.should eq 3
      cells[1].as_f.should eq 2.5
      cells[1].as_i?.should be_nil
      cells[5]?.should be_nil
      cells[-1]?.should be_nil
      cells.map(&.json).should eq ["1", "2.5", "\"x\""]
      cells.strings.should eq ["x"]
      JSON.parse(info["battery"].json)["charging"].as_bool.should be_false
      info["battery"].decode(Hash(String, Float64 | Bool))["percent"].should eq 71.0
      info.size.should eq 0
      info.close
    end
  end

  it "keeps the answer alive for a child after the root is dropped" do
    Wol::Spec.with_fake("core") do |ctx, _|
      child = power(ctx)["battery"]
      5.times do
        Array.new(1000) { "garbage" * 10 }
        GC.collect
      end
      child["percent"].as_f.should eq 71.0
    end
  end

  it "refuses a child once its root is closed" do
    Wol::Spec.with_fake("core") do |ctx, _|
      root = power(ctx)
      child = root["battery"]
      root.close
      root.closed?.should be_true
      child.closed?.should be_true
      expect_raises(Wol::UsageError) { child["percent"] }
      root.close
    end
  end
end

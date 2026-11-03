# SPDX-License-Identifier: BSD-2-Clause
# Specs for the patch encoder: the bytes are checked against the layout in patch.rs and read back
# with Patch.describe.
require "./spec_helper"

describe Wolkit::PatchWriter do
  it "writes the 24-byte header, the ops and the string pool" do
    w = Wolkit::PatchWriter.new
    w.create(1_u32, Wolkit::Wire::Kind::BOX)
    w.insert(0_u32, 1_u32)
    w.set_text(1_u32, "Hi")
    bytes = w.finish(7_u32)
    String.new(bytes[0, 4]).should eq "WKP1"
    IO::ByteFormat::LittleEndian.decode(UInt16, bytes[4, 2]).should eq 1
    IO::ByteFormat::LittleEndian.decode(UInt32, bytes[8, 4]).should eq 7
    IO::ByteFormat::LittleEndian.decode(UInt32, bytes[12, 4]).should eq 3
    IO::ByteFormat::LittleEndian.decode(UInt32, bytes[16, 4]).should eq 8
    (bytes.size % 4).should eq 0
    # CREATE: opcode 1, MUST_UNDERSTAND | 3 words, node, kind
    IO::ByteFormat::LittleEndian.decode(UInt16, bytes[24, 2]).should eq 1
    IO::ByteFormat::LittleEndian.decode(UInt16, bytes[26, 2]).should eq 0x8003
    bytes[bytes.size - 4, 2].should eq "Hi".to_slice
  end

  it "describes every op it writes" do
    w = Wolkit::PatchWriter.new
    w.create(1_u32, Wolkit::Wire::Kind::BOX)
    w.create(2_u32, Wolkit::Wire::Kind::TEXT)
    w.set_f32(1_u32, Wolkit::Wire::Prop::GAP, 12.0_f32)
    w.set_u32(1_u32, Wolkit::Wire::Prop::FLEX_DIRECTION, 1_u32)
    w.set_bool(2_u32, Wolkit::Wire::Prop::FOCUSABLE, true)
    w.set_dim(1_u32, Wolkit::Wire::Prop::WIDTH, 1_u32, 420.0_f32)
    w.set_text(2_u32, "1 left")
    w.set_style(1_u32, [3_u32, 4_u32])
    w.set_state(1_u32, 16_u32)
    w.set_key(1_u32, 0x1_0000_0002_u64)
    w.bind_event(1_u32, 1_u32)
    w.set_a11y(2_u32, Wolkit::Wire::Role::LABEL, "count", "")
    w.insert(0_u32, 1_u32)
    w.insert(1_u32, 2_u32, 0_u32)
    w.remove(2_u32)
    w.free(2_u32)
    Wolkit::Patch.describe(w.finish(1_u32)).should eq [
      "PATCH seq=1 ops=16 pool=24",
      "CREATE 1 box",
      "CREATE 2 text",
      "SET_PROP 1 gap 12.0",
      "SET_PROP 1 flex-direction column",
      "SET_PROP 2 focusable true",
      "SET_PROP 1 width px:420.0",
      "SET_TEXT 2 \"1 left\"",
      "SET_STYLE 1 3 4",
      "SET_STATE 1 16",
      "SET_KEY 1 4294967298",
      "BIND_EVENT 1 1",
      "SET_A11Y 2 label \"count\" \"\"",
      "INSERT 0 1 append",
      "INSERT 1 2 0",
      "REMOVE 2",
      "FREE 2",
    ]
  end

  it "reuses its buffers: a second patch has the same bytes as the first" do
    w = Wolkit::PatchWriter.new
    build = ->(writer : Wolkit::PatchWriter) do
      writer.create(1_u32, Wolkit::Wire::Kind::BOX)
      writer.set_text(1_u32, "same")
    end
    build.call(w)
    first = w.finish(1_u32).dup
    w.clear
    build.call(w)
    w.finish(1_u32).should eq first
  end

  it "grows for a patch bigger than its first buffers" do
    w = Wolkit::PatchWriter.new(64)
    200.times { |i| w.create((i + 1).to_u32, Wolkit::Wire::Kind::BOX) }
    w.set_text(1_u32, "x" * 5000)
    lines = Wolkit::Patch.describe(w.finish(2_u32))
    lines.size.should eq 202
    w.full?.should be_false
  end

  it "refuses a text over the engine limit" do
    w = Wolkit::PatchWriter.new
    expect_raises(Wolkit::LimitError, /at most 65536 bytes/) { w.set_text(1_u32, "x" * 70_000) }
  end
end

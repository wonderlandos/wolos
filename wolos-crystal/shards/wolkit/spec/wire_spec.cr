# SPDX-License-Identifier: BSD-2-Clause
# The wire ids in src/wolkit/patch/wire.cr are a copy of the engine's append-only tables. This spec
# compares the copy with the generated binding of wolkit.h (`LibWolkit`) and with the property
# snapshot `abi/wolkit-1.0.props`, so an id that drifts fails here, not on a user's screen.
require "./spec_helper"
require "../src/wolkit/lib_wolkit"

describe Wolkit::Wire do
  it "agrees with the constants generated from wolkit.h" do
    {% for pair in [{"Op", "OP"}, {"Kind", "KIND"}, {"Role", "ROLE"}, {"State", "STATE"}, {"Event", "EVENT"}, {"Prop", "PROP"}] %}
      {% mod = pair[0].id %}
      {% for name in Wolkit::Wire.constant(pair[0]).constants %}
        Wolkit::Wire::{{ mod }}::{{ name }}.to_i64.should eq(LibWolkit::WOLKIT_{{ pair[1].id }}_{{ name }}.to_i64), "Wire::{{ mod }}::{{ name }} differs from WOLKIT_{{ pair[1].id }}_{{ name }}"
      {% end %}
    {% end %}
    Wolkit::Wire::MAGIC.should eq LibWolkit::WOLKIT_PATCH_MAGIC.to_u32
    Wolkit::Wire::FORMAT.to_i.should eq LibWolkit::WOLKIT_PATCH_FORMAT
    Wolkit::Wire::HEADER.should eq LibWolkit::WOLKIT_PATCH_HEADER
    Wolkit::Wire::APPEND.should eq LibWolkit::WOLKIT_APPEND.to_u32
    Wolkit::Wire::NO_STR.should eq LibWolkit::WOLKIT_NO_STR.to_u32
    Wolkit::Wire::MUST_UNDERSTAND.to_u32.should eq LibWolkit::WOLKIT_PATCH_MUST_UNDERSTAND.to_u32
  end

  it "lists every property of the engine's snapshot with the same id and type" do
    path = File.join(__DIR__, "../../../../crates/wolkit/abi/wolkit-1.0.props")
    next unless File.exists?(path)
    lines = File.read_lines(path).reject(&.empty?)
    lines.size.should eq Wolkit::Wire::PROPS.size
    lines.each do |line|
      id, _ident, name, type = line.split(' ', 4)
      info = Wolkit::Wire::PROPS[name]?
      info.should_not be_nil, "the property #{name} is missing from Wire::PROPS"
      next unless info
      info[:id].should eq id.to_i
      info[:ty].should eq type
    end
  end

  it "has a table for every enum property" do
    Wolkit::Wire::PROPS.each_value do |info|
      table = info[:en]
      if info[:ty] == "enum"
        table.should_not be_nil
        Wolkit::Wire::ENUMS.has_key?(table.to_s).should be_true
      else
        table.should be_nil
      end
    end
  end
end

describe Wolkit::DSL do
  it "maps every property to an engine property" do
    Wolkit::DSL::PROPS.each do |name, spec|
      Wolkit::Wire::PROPS.has_key?(spec[:wire]).should be_true, "#{name} names the unknown engine property #{spec[:wire]}"
    end
  end

  it "documents every element and property in docs/modules/wolkit.md" do
    path = File.join(__DIR__, "../../../../docs/modules/wolkit.md")
    next unless File.exists?(path)
    text = File.read(path)
    Wolkit::DSL::ELEMENTS.each_key { |name| text.should contain("`#{name}`"), "wolkit.md does not mention the element #{name}" }
    Wolkit::DSL::COMMON.each { |name| text.should contain(name), "wolkit.md does not list the property #{name}" }
  end
end

# SPDX-License-Identifier: GPL-2.0-or-later
# The schedule rule: which phase it is, from the mode, the clock and the Night Shift flag.
require "./spec_helper"

private def at(h : Int32, m : Int32 = 0) : Time
  Time.local(2026, 11, 3, h, m, 0)
end

describe Looks::ScheduleConfig do
  it "is off by default and gives no phase" do
    c = Looks::ScheduleConfig.new
    c.off?.should be_true
    c.phase(at(12), true).should be_nil
  end

  it "follows the Night Shift flag" do
    c = Looks::ScheduleConfig.new
    c.mode = "night-shift"
    c.phase(at(12), true).should eq Looks::Phase::Night
    c.phase(at(3), false).should eq Looks::Phase::Day
  end

  it "uses a window that crosses midnight" do
    c = Looks::ScheduleConfig.new
    c.mode = "times"
    c.phase(at(18, 59), false).should eq Looks::Phase::Day
    c.phase(at(19, 0), false).should eq Looks::Phase::Night
    c.phase(at(23, 59), false).should eq Looks::Phase::Night
    c.phase(at(0, 0), false).should eq Looks::Phase::Night
    c.phase(at(6, 59), false).should eq Looks::Phase::Night
    c.phase(at(7, 0), false).should eq Looks::Phase::Day
  end

  it "uses a window inside one day, and an empty one is always day" do
    c = Looks::ScheduleConfig.new
    c.mode = "times"
    c.from = "01:00"
    c.to = "05:30"
    c.phase(at(2), false).should eq Looks::Phase::Night
    c.phase(at(5, 30), false).should eq Looks::Phase::Day
    c.from = "08:00"
    c.to = "08:00"
    c.phase(at(8), false).should eq Looks::Phase::Day
  end

  it "reads times strictly" do
    Looks::ScheduleConfig.minutes("19:00").should eq 1140
    Looks::ScheduleConfig.minutes("7:05").should eq 425
    Looks::ScheduleConfig.minutes("24:00").should be_nil
    Looks::ScheduleConfig.minutes("19:60").should be_nil
    Looks::ScheduleConfig.minutes("seven").should be_nil
    c = Looks::ScheduleConfig.new
    c.valid?.should be_true
    c.from = "later"
    c.valid?.should be_false
    c.mode = "sometimes"
    c.valid?.should be_false
  end

  it "reads the Night Shift flag file" do
    with_tmpdir do |dir|
      flag = File.join(dir, "org.wolos.nightlight")
      Looks::NightShift.on?(flag).should be_false
      File.write(flag, "")
      Looks::NightShift.on?(flag).should be_true
    end
  end
end

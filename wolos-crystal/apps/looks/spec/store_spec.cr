# SPDX-License-Identifier: GPL-2.0-or-later
# The store: saving, renaming, deleting and favouriting looks, the history of twenty, a state file
# that is damaged or from nowhere, and compatibility with the files `wol-hue save` writes.
require "./spec_helper"

private BASE = %w(#101820 #d04a4a #4ad08a #d0b04a #4a7ad0 #b04ad0 #4ad0d0 #e8e8e8 #303840 #ff6a6a #6aff9a #ffd06a #6a9aff #d06aff #6affff #ffffff)

private def look(name : String) : Hue::Look
  Hue::Look.new(name, BASE, "dark", "normal", "picture", "/p/a.png")
end

private def store(dir : String) : Looks::Store
  Looks::Store.new(File.join(dir, "data", "state.json"), File.join(dir, "looks"))
end

describe Looks::Store do
  it "saves a look as the file `wol-hue list` reads and finds it again" do
    with_tmpdir do |dir|
      s = store(dir)
      s.save(look("Desert day")).should be_nil
      File.file?(File.join(dir, "looks", "Desert day.json")).should be_true
      s.names.should eq ["Desert day"]
      must(s.find("Desert day")).colors.should eq BASE
      Hue::Look.list.size # the engine's own lister type-checks against the same files
    end
  end

  it "reads a look written by the command line tool" do
    with_tmpdir do |dir|
      FileUtils.mkdir_p(File.join(dir, "looks"))
      l = look("From cli")
      l.adjustments["vibrance"] = 10.0
      File.write(File.join(dir, "looks", "From cli.json"), l.to_pretty_json + "\n")
      must(store(dir).find("From cli")).adjustments.should eq({"vibrance" => 10.0})
    end
  end

  it "refuses a name the engine would refuse, and a look that fails its rules" do
    with_tmpdir do |dir|
      s = store(dir)
      said(s.save(look("../escape"))).should contain("A name has letters")
      s.find("../escape").should be_nil
      bad = look("Bad")
      bad.colors = ["#000000"]
      said(s.save(bad)).should contain("cannot be saved")
      Dir.exists?(File.join(dir, "looks")).should be_false
    end
  end

  it "puts favourites first, then by name" do
    with_tmpdir do |dir|
      s = store(dir)
      %w(Bravo Alpha Charlie).each { |n| s.save(look(n)) }
      s.names.should eq %w(Alpha Bravo Charlie)
      s.toggle_favourite("Charlie").should be_true
      s.names.should eq %w(Charlie Alpha Bravo)
      s.favourite?("charlie").should be_true
      s.toggle_favourite("Charlie").should be_false
      s.names.should eq %w(Alpha Bravo Charlie)
    end
  end

  it "renames a look and its favourite and schedule entries follow" do
    with_tmpdir do |dir|
      s = store(dir)
      s.save(look("Old name"))
      s.toggle_favourite("Old name")
      st = s.state
      st.schedule.mode = "times"
      st.schedule.day = "Old name"
      s.save_state(st)
      s.rename("Old name", "New name").should be_nil
      s.names.should eq ["New name"]
      s.find("Old name").should be_nil
      must(s.find("New name")).name.should eq "New name"
      s.favourite?("New name").should be_true
      s.state.schedule.day.should eq "New name"
    end
  end

  it "does not rename onto another look or to a bad name" do
    with_tmpdir do |dir|
      s = store(dir)
      s.save(look("One"))
      s.save(look("Two"))
      said(s.rename("One", "Two")).should contain("already a look")
      said(s.rename("One", "a/b")).should contain("A name has letters")
      said(s.rename("Nope", "Three")).should contain("There is no look")
      s.names.should eq %w(One Two)
      s.rename("One", "One").should be_nil
    end
  end

  it "renames only the case of a name" do
    with_tmpdir do |dir|
      s = store(dir)
      s.save(look("dune"))
      s.rename("dune", "Dune").should be_nil
      s.names.should eq ["Dune"]
    end
  end

  it "deletes a look and forgets its favourite and schedule entries" do
    with_tmpdir do |dir|
      s = store(dir)
      s.save(look("Gone"))
      s.toggle_favourite("Gone")
      st = s.state
      st.schedule.night = "Gone"
      s.save_state(st)
      s.delete("Gone").should be_nil
      s.names.should be_empty
      s.state.favourites.should be_empty
      s.state.schedule.night.should be_nil
      said(s.delete("Gone")).should contain("There is no look")
    end
  end

  it "keeps the last twenty applied looks, newest first" do
    with_tmpdir do |dir|
      s = store(dir)
      25.times { |i| s.record(look("Look #{i}"), "looks-look-#{i}", Time.utc(2026, 11, 3, 10, i, 0)).should be_nil }
      h = s.history
      h.size.should eq 20
      h.first.look.name.should eq "Look 24"
      h.last.look.name.should eq "Look 5"
      h.first.id.should eq "looks-look-24"
    end
  end

  it "survives a damaged or oversized state file and drops bad history entries" do
    with_tmpdir do |dir|
      s = store(dir)
      FileUtils.mkdir_p(File.dirname(s.state_file))
      File.write(s.state_file, "{not json")
      s.state.history.should be_empty
      File.write(s.state_file, "x" * 2_000_000)
      s.state.favourites.should be_empty
      good = look("Good")
      File.write(s.state_file, {"history" => [
        {"at" => 1, "id" => "looks-good", "look" => JSON.parse(good.to_json)},
        {"at" => 2, "id" => "../bad", "look" => JSON.parse(good.to_json)},
      ]}.to_json)
      s.state.history.map(&.id).should eq ["looks-good"]
    end
  end

  it "writes private files and leaves no temporary file behind" do
    with_tmpdir do |dir|
      s = store(dir)
      s.save(look("Quiet"))
      s.record(look("Quiet"), "looks-quiet")
      File.info(s.state_file).permissions.value.should eq 0o600
      Dir.glob(File.join(dir, "**", "*.tmp.*")).should be_empty
    end
  end
end

describe Looks::Tick do
  it "applies the look of the phase once, and again only when the phase changes" do
    with_tmpdir do |dir|
      s = store(dir)
      s.save(look("Sun"))
      s.save(look("Moon"))
      st = s.state
      st.schedule.mode = "times"
      st.schedule.day = "Sun"
      st.schedule.night = "Moon"
      s.save_state(st)
      fake = FakeApplier.new
      noon = Time.local(2026, 11, 3, 12, 0, 0)
      late = Time.local(2026, 11, 3, 22, 0, 0)
      Looks::Tick.run(s, fake, noon, false).applied.should eq "Sun"
      Looks::Tick.run(s, fake, noon + 5.minutes, false).applied.should be_nil
      fake.applied.map(&.[0]).should eq ["Sun"]
      # a look chosen by hand stays until the phase changes
      Looks::Tick.run(s, fake, noon + 30.minutes, false).applied.should be_nil
      Looks::Tick.run(s, fake, late, false).applied.should eq "Moon"
      fake.applied.map(&.[1]).should eq ["looks-sun", "looks-moon"]
      s.history.map(&.look.name).should eq ["Moon", "Sun"]
    end
  end

  it "follows Night Shift when asked to" do
    with_tmpdir do |dir|
      s = store(dir)
      s.save(look("Sun"))
      s.save(look("Moon"))
      st = s.state
      st.schedule.mode = "night-shift"
      st.schedule.day = "Sun"
      st.schedule.night = "Moon"
      s.save_state(st)
      fake = FakeApplier.new
      Looks::Tick.run(s, fake, Time.local, true).applied.should eq "Moon"
      Looks::Tick.run(s, fake, Time.local, false).applied.should eq "Sun"
    end
  end

  it "does nothing when off, when the look is gone, or when the desktop refuses" do
    with_tmpdir do |dir|
      s = store(dir)
      fake = FakeApplier.new
      Looks::Tick.run(s, fake, Time.local, true).phase.should be_nil
      s.save(look("Sun"))
      st = s.state
      st.schedule.mode = "night-shift"
      st.schedule.day = "Sun"
      st.schedule.night = "Missing"
      s.save_state(st)
      r = Looks::Tick.run(s, fake, Time.local, true)
      said(r.notice).should contain("not saved any more")
      fake.refuse = "no"
      Looks::Tick.run(s, fake, Time.local, false).notice.should eq "no"
      fake.applied.should be_empty
      s.state.phase.should be_nil
    end
  end
end

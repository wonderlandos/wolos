# SPDX-License-Identifier: GPL-2.0-or-later
# Every state of Looks through the headless engine, with golden screenshots and accessibility trees,
# the keyboard map driven with key events, and the pictures it writes read back and checked.
require "./engine_helper_spec"

include LooksSpec

alias K = Looks::Keys

describe "Looks" do
  it "invites you to pick a picture when there is none" do
    LooksSpec.run(Env.new) do |t, app|
      app.reading.value.idle?.should be_true
      t.text(app.status).should start_with("Pick a picture to start")
      LooksSpec.pin(t, "make-empty")
    end
  end

  it "says it is reading while the picture is read" do
    env = Env.new
    LooksSpec.run(env, LooksSpec.scenes(env), hold: true) do |t, app|
      app.reading.value.reading?.should be_true
      t.text("Reading the picture…").should eq "Reading the picture…"
      app.card_src.each { |c| File.file?(c.value).should be_true } # the neutral pictures are there
      LooksSpec.pin(t, "make-reading")
    end
  end

  it "keeps the newest choice when a slow picture finishes last" do
    env = Env.new
    pics = LooksSpec.scenes(env)
    LooksSpec.run(env, pics, hold: true) do |t, app|
      # start_up asked for the first picture; ask for the third, then let the third finish first
      app.use_picture(pics[2])
      App.held.size.should eq 2
      App.held[1].call
      t.settle
      app.reading.value.ready?.should be_true
      must(app.analysis).path.should eq pics[2]
      App.held[0].call
      t.settle
      must(app.analysis).path.should eq pics[2]
      app.picture_name.value.should eq "ocean"
    end
  end

  it "shows a picture it cannot read with one sentence and a way on" do
    env = Env.new
    bad = File.join(env.dir, "bad.png")
    File.write(bad, "not a picture")
    LooksSpec.run(env, [bad]) do |t, app|
      app.reading.value.failed?.should be_true
      app.error.value.should contain("could not be read")
      t.text(app.status).should end_with("Choose another picture.")
      LooksSpec.pin(t, "make-failed")
    end
  end

  it "reads the first picture and shows the eight modes, the sliders and the app previews" do
    env = Env.new
    LooksSpec.run(env, LooksSpec.scenes(env)) do |t, app|
      app.reading.value.ready?.should be_true
      app.picture_name.value.should eq "sunset"
      Looks::MODES.each { |m| t.find(Looks::MODE_LABELS[m]) }
      Looks::SLIDERS.each { |s| t.find(s.label) }
      %w(Terminal Settings Document Browser).each { |n| t.find("#{n} preview") }
      app.card_src.each { |s| File.file?(s.value).should be_true }
      app.app_src.each { |s| File.file?(s.value).should be_true }
      LooksSpec.fits_kit?(t).should be_true, "parts #{LooksSpec.parts(t)}, depth #{LooksSpec.depth(t)}"
      LooksSpec.pin(t, "make")
    end
  end

  it "fits a 900 x 600 window and a narrow one" do
    env = Env.new
    LooksSpec.run(env, LooksSpec.scenes(env), size: {900, 600}) do |t, _app|
      LooksSpec.pin(t, "make-900")
    end
    env2 = Env.new
    LooksSpec.run(env2, LooksSpec.scenes(env2), size: {480, 360}) do |t, _app|
      LooksSpec.fits_kit?(t).should be_true, "parts #{LooksSpec.parts(t)}, depth #{LooksSpec.depth(t)}"
      LooksSpec.pin(t, "make-narrow")
    end
  end

  it "has a one-column layout for the Kit backend, within its limits on every page" do
    env = Env.new
    pal = Looks::Analysis.read(env.picture("forest")).palette(false, "normal")
    20.times { |i| env.store.save(Hue::Look.new("Look #{i}", pal, "dark")) }
    LooksSpec.run(env, LooksSpec.scenes(env), compact: true) do |t, app|
      app.compact?.should be_true
      LooksSpec.fits_kit?(t).should be_true, "make: parts #{LooksSpec.parts(t)}, depth #{LooksSpec.depth(t)}"
      LooksSpec.pin(t, "make-kit")
      Looks::Page.values.each do |page|
        app.go(page)
        t.settle
        LooksSpec.fits_kit?(t).should be_true, "#{page}: parts #{LooksSpec.parts(t)}, depth #{LooksSpec.depth(t)}"
      end
    end
  end

  it "follows dark mode and reduce motion" do
    env = Env.new
    LooksSpec.run(env, LooksSpec.scenes(env)) do |t, _app|
      t.engine.set_environment(true, 0x7c5cff_u32)
      t.settle
      LooksSpec.pin(t, "make-dark")
      t.engine.set_environment(true, 0x7c5cff_u32, LibWolkit::WOLKIT_ENV_REDUCE_MOTION.to_u32)
      t.settle
      Wolkit::Env.reduce_motion.value.should be_true
      t.press K::RIGHT
    end
  end

  it "moves between modes with the arrows and redraws the previews" do
    env = Env.new
    LooksSpec.run(env, LooksSpec.scenes(env)) do |t, app|
      before = app.app_src.map(&.value)
      t.press K::RIGHT
      t.press K::RIGHT
      app.mode.value.should eq 2
      app.app_src.map(&.value).should_not eq before
      t.press K::DOWN
      app.mode.value.should eq 6
      t.press K::DOWN
      app.mode.value.should eq 7
      t.press K::HOME
      app.mode.value.should eq 0
      t.press K::END_
      app.mode.value.should eq 7
      t.click "Pastel"
      app.mode.value.should eq 3
      must(app.selected).mode.should eq "pastel"
    end
  end

  it "switches variant with the keys and the tabs" do
    env = Env.new
    LooksSpec.run(env, LooksSpec.scenes(env)) do |t, app|
      app.light.value.should be_false
      t.press K::L
      app.light.value.should be_true
      must(app.selected).variant.should eq "light"
      t.press K::D
      app.light.value.should be_false
      t.click "Light"
      app.light.value.should be_true
      t.click "Dark"
      app.light.value.should be_false
    end
  end

  it "writes pictures whose pixels are the candidate's tokens" do
    env = Env.new
    LooksSpec.run(env, LooksSpec.scenes(env)) do |t, app|
      t.press K::RIGHT
      a = must(app.analysis)
      dark = Looks::Candidate.new(a.palette(false, "monochromatic"), false, "monochromatic")
      light = Looks::Candidate.new(a.palette(true, "monochromatic"), true, "monochromatic")
      Hue::Picture.load(app.card_src[1].value).rgba.should eq Looks::Raster.card(dark, light).rgba
      Hue::Picture.load(app.app_src[0].value).rgba.should eq Looks::Raster.terminal(dark).rgba
      Hue::Picture.load(app.app_src[3].value).rgba.should eq Looks::Raster.browser(dark).rgba
    end
  end

  it "re-colours every card at once when a slider moves, and keeps only two generations of files" do
    env = Env.new
    LooksSpec.run(env, LooksSpec.scenes(env)) do |t, app|
      first = app.card_src.map(&.value)
      t.press K::TWO # the second slider (contrast)
      app.slider_index.value.should eq 1
      t.press K::EQUAL
      t.press K::EQUAL
      app.sliders[1].value.should eq 10.0
      app.card_src.map(&.value).should_not eq first
      app.adjustments.should eq({"contrast" => 10.0})
      t.press K::MINUS
      t.press K::MINUS
      t.press K::MINUS
      app.sliders[1].value.should eq -5.0
      files = Dir.children(File.join(env.dir, "cache", "previews"))
      generations = files.compact_map { |f| f.match(/-(\d+)\.png\z/).try(&.[1].to_i) }.uniq!
      generations.size.should be <= 2
      t.press K::ZERO
      app.sliders.map(&.value).uniq!.should eq [0.0]
      app.adjustments.should be_empty
      first.each_with_index { |_, i| File.file?(app.card_src[i].value).should be_true }
    end
  end

  it "keeps a slider inside its range" do
    env = Env.new
    LooksSpec.run(env, LooksSpec.scenes(env)) do |t, app|
      40.times { t.press K::EQUAL }
      app.sliders[0].value.should eq 50.0
      app.set_slider(0, -900.0)
      app.sliders[0].value.should eq -50.0
    end
  end

  it "moves through the pictures with Page Down and Page Up" do
    env = Env.new
    LooksSpec.run(env, LooksSpec.scenes(env)) do |t, app|
      t.press K::PAGE_DOWN
      app.picture_name.value.should eq "forest"
      t.press K::PAGE_DOWN
      t.press K::PAGE_DOWN
      app.picture_name.value.should eq "ocean"
      t.press K::PAGE_UP
      app.picture_name.value.should eq "forest"
      t.click "ocean"
      app.picture_name.value.should eq "ocean"
    end
  end

  it "applies the candidate to the desktop with Return, records it and offers Undo" do
    env = Env.new
    LooksSpec.run(env, LooksSpec.scenes(env)) do |t, app|
      t.press K::RIGHT # Monochrome
      t.press K::L
      t.press K::RETURN
      applier = App.applier
      applier.applied.size.should eq 1
      name, id = applier.applied.first
      name.should eq "sunset Monochrome light"
      id.should eq "looks-sunset-monochrome-light"
      app.notice.value.should eq "“sunset Monochrome light” is on."
      app.history.value.map(&.look.name).should eq ["sunset Monochrome light"]
      app.history.value.first.look.variant.should eq "light"
      app.active_id.value.should eq "looks-sunset-monochrome-light"
      LooksSpec.pin(t, "applied")
      t.click "Undo"
      applier.switched.should eq ["wol-dark"]
      app.notice.value.should eq "Back to the previous theme."
    end
  end

  it "applies with the Apply button, with the sliders in the look" do
    env = Env.new
    LooksSpec.run(env, LooksSpec.scenes(env)) do |t, app|
      app.set_slider(2, 20.0)
      t.click "Apply"
      app.history.value.first.look.adjustments.should eq({"temperature" => 20.0})
    end
  end

  it "says why when the desktop refuses" do
    env = Env.new
    LooksSpec.run(env, LooksSpec.scenes(env)) do |t, app|
      App.applier.refuse = "The desktop did not switch to the new look. Try again, or open Settings > Appearance."
      t.press K::RETURN
      app.notice.value.should contain("did not switch")
      app.history.value.should be_empty
      app.active_id.value.should eq "wol-dark" # still the theme it was
    end
  end

  it "asks for a name with Ctrl+S, saves under it, and refuses a bad one" do
    env = Env.new
    LooksSpec.run(env, LooksSpec.scenes(env)) do |t, app|
      t.press K::S, K::CTRL
      app.form.value.save?.should be_true
      app.draft.value.should eq "sunset Natural"
      LooksSpec.pin(t, "make-save")
      t.click "Name of the look"
      app.draft.value = "bad/name"
      t.click "Save"
      app.notice.value.should contain("A name has letters")
      app.form.value.save?.should be_true
      app.draft.value = "Dusk"
      t.click "Save"
      app.form.value.none?.should be_true
      env.store.names.should eq ["Dusk"]
      must(env.store.find("Dusk")).wallpaper.should eq env.picture("sunset")
      app.notice.value.should eq "Saved “Dusk”."
      # the same candidate again gets the next free name
      t.press K::S, K::CTRL
      app.draft.value.should eq "sunset Natural"
      t.press K::ESCAPE
      app.form.value.none?.should be_true
    end
  end

  it "does not act on keys while a name is being typed" do
    env = Env.new
    LooksSpec.run(env, LooksSpec.scenes(env)) do |t, app|
      t.press K::S, K::CTRL
      t.press K::L
      t.press K::RIGHT
      app.light.value.should be_false
      app.mode.value.should eq 0
    end
  end

  it "lists the saved looks, favourites first, and applies one" do
    env = Env.new
    s = env.store
    %w(Bravo Alpha Charlie).each { |n| s.save(Hue::Look.new(n, Looks::Analysis.read(env.picture("forest")).palette(false, "normal"), "dark", "normal", "picture", env.picture("forest"))) }
    s.toggle_favourite("Charlie")
    LooksSpec.run(env, LooksSpec.scenes(env)) do |t, app|
      t.press K::TWO, K::CTRL
      app.page.value.saved?.should be_true
      app.shown_looks.map(&.name).should eq %w(Charlie Alpha Bravo)
      LooksSpec.fits_kit?(t).should be_true, "parts #{LooksSpec.parts(t)}, depth #{LooksSpec.depth(t)}"
      LooksSpec.pin(t, "saved")
      t.press K::DOWN
      t.press K::RETURN
      App.applier.applied.map(&.[0]).should eq ["Alpha"]
      t.click "Favourites"
      app.shown_looks.map(&.name).should eq ["Charlie"]
      t.click "All"
      app.shown_looks.size.should eq 3
    end
  end

  it "marks a favourite, renames, deletes with Undo" do
    env = Env.new
    s = env.store
    pal = Looks::Analysis.read(env.picture("forest")).palette(false, "normal")
    %w(One Two).each { |n| s.save(Hue::Look.new(n, pal, "dark")) }
    LooksSpec.run(env, LooksSpec.scenes(env)) do |t, app|
      t.press K::TWO, K::CTRL
      t.press K::DOWN
      t.press K::F
      app.notice.value.should eq "“Two” is a favourite."
      app.shown_looks.map(&.name).should eq %w(Two One)
      app.look_index.value.should eq 1 # the selection stays where it was
      t.press K::UP
      t.press K::F2
      app.form.value.rename?.should be_true
      app.draft.value.should eq "Two"
      app.draft.value = "Dos"
      t.click "Save"
      env.store.names.should eq %w(Dos One)
      app.notice.value.should eq "Renamed to “Dos”."
      t.press K::DELETE
      env.store.names.should eq ["One"]
      app.notice.value.should eq "“Dos” is deleted."
      LooksSpec.pin(t, "saved-deleted")
      t.click "Undo"
      env.store.names.should eq %w(Dos One)
      env.store.favourite?("Dos").should be_true
    end
  end

  it "shows the empty Saved page" do
    env = Env.new
    LooksSpec.run(env, LooksSpec.scenes(env)) do |t, _app|
      t.press K::TWO, K::CTRL
      t.text("No saved looks yet").should eq "No saved looks yet"
      LooksSpec.pin(t, "saved-empty")
    end
  end

  it "imports a .wollook, Base16 and colors file, and exports a look" do
    env = Env.new
    pal = Looks::Analysis.read(env.picture("forest")).palette(false, "normal")
    wl = Looks::Wollook.dump(Hue::Look.new("Shared", pal, "dark", "muted"))
    LooksSpec.run(env, LooksSpec.scenes(env)) do |t, app|
      App.system.handed = Looks::Handed.new("shared.wollook", wl.to_slice)
      t.press K::I, K::CTRL
      App.system.opened.should eq ["Import a look or a palette"]
      app.page.value.saved?.should be_true
      app.notice.value.should eq "Imported “Shared”."
      env.store.names.should eq ["Shared"]
      # the same file again: the next free name
      t.press K::I, K::CTRL
      env.store.names.should eq ["Shared", "Shared 2"]
      App.system.handed = Looks::Handed.new("x.txt", "hello".to_slice)
      t.press K::I, K::CTRL
      app.notice.value.should contain("reads .wollook")
      App.system.handed = nil
      App.system.refusal = "Allow Looks to open files in System Settings."
      t.press K::I, K::CTRL
      app.notice.value.should contain("Allow Looks")
      App.system.refusal = nil
      t.press K::I, K::CTRL
      env.store.names.size.should eq 2
      # export the selected one
      t.press K::E, K::CTRL
      App.system.saved.size.should eq 1
      App.system.saved.first[0].should eq "Shared 2.wollook"
      Looks::Wollook.parse(App.system.saved.first[1]).name.should eq "Shared 2"
      app.notice.value.should eq "Exported “Shared 2”."
      App.system.save_result = "The look could not be written: disk full"
      t.press K::E, K::CTRL
      app.notice.value.should contain("disk full")
    end
  end

  it "keeps the picture the file panel hands over, and uses it" do
    env = Env.new
    bytes = File.read(env.picture("desert")).to_slice
    LooksSpec.run(env, [] of String) do |t, app|
      App.system.handed = Looks::Handed.new("My Desert.png", bytes)
      t.press K::O, K::CTRL
      app.reading.value.ready?.should be_true
      app.picture.value.should match(/org\.wolos\.looks\/pictures\/[0-9a-f]{12}-My-Desert\.png\z/)
      app.picture.value.should start_with(File.join(env.dir, "data"))
    end
  end

  it "lists the last twenty applied looks and applies one again" do
    env = Env.new
    LooksSpec.run(env, LooksSpec.scenes(env)) do |t, app|
      Looks::MODES.each_with_index do |_, i|
        app.select_mode(i)
        t.press K::RETURN
      end
      t.press K::THREE, K::CTRL
      app.page.value.history?.should be_true
      app.history.value.size.should eq 8
      app.history.value.first.look.name.should eq "sunset Bright"
      LooksSpec.fits_kit?(t).should be_true, "parts #{LooksSpec.parts(t)}, depth #{LooksSpec.depth(t)}"
      LooksSpec.pin(t, "history")
      t.press K::DOWN
      t.press K::RETURN
      App.applier.applied.last[0].should eq "sunset Muted"
      app.history.value.first.look.name.should eq "sunset Muted"
    end
  end

  it "shows the empty History page" do
    env = Env.new
    LooksSpec.run(env, LooksSpec.scenes(env)) do |t, _app|
      t.press K::THREE, K::CTRL
      t.text("Nothing applied yet").should eq "Nothing applied yet"
      LooksSpec.pin(t, "history-empty")
    end
  end

  it "keeps a day look and a night look, and switches them with the clock" do
    env = Env.new
    pal = Looks::Analysis.read(env.picture("forest")).palette(false, "normal")
    %w(Sun Moon).each { |n| env.store.save(Hue::Look.new(n, pal, "dark")) }
    LooksSpec.run(env, LooksSpec.scenes(env)) do |t, app|
      t.press K::FOUR, K::CTRL
      app.page.value.schedule?.should be_true
      LooksSpec.pin(t, "schedule")
      t.press K::THREE
      app.schedule_mode.value.should eq 2
      t.press K::D # the next saved look is the day look
      app.day_name.value.should eq "Moon"
      t.press K::D
      app.day_name.value.should eq "Sun"
      t.press K::L
      app.night_name.value.should eq "Moon"
      LooksSpec.pin(t, "schedule-hours")
      App.applier.timer_on?.should be_true # the user timer is on while a schedule is
      st = env.store.state
      st.schedule.mode.should eq "times"
      st.schedule.day.should eq "Sun"
      st.schedule.night.should eq "Moon"
      # 15:00 is day
      app.schedule_tick
      App.applier.applied.map(&.[0]).should eq ["Sun"]
      app.notice.value.should eq "The day look “Sun” is on."
      app.schedule_tick
      App.applier.applied.size.should eq 1
    end
  end

  it "checks the hours before keeping them" do
    env = Env.new
    LooksSpec.run(env, LooksSpec.scenes(env)) do |t, app|
      t.press K::FOUR, K::CTRL
      t.press K::THREE
      app.from_draft.value = "later"
      app.commit_hours
      app.notice.value.should eq "Write the hours as 19:00 and 07:00."
      app.from_draft.value = "20:30"
      app.to_draft.value = "6:15"
      app.commit_hours
      env.store.state.schedule.from.should eq "20:30"
      env.store.state.schedule.to.should eq "6:15"
      t.press K::ONE
      app.schedule_mode.value.should eq 0
      App.applier.timer_on?.should be_false
    end
  end

  it "dismisses a notice with Escape" do
    env = Env.new
    LooksSpec.run(env, LooksSpec.scenes(env)) do |t, app|
      app.say("Something happened.")
      t.settle
      t.find("Something happened.")
      t.press K::ESCAPE
      app.notice.value.should eq ""
    end
  end

  it "keeps every page under the Kit backend's 200 parts with a full library" do
    env = Env.new
    pal = Looks::Analysis.read(env.picture("forest")).palette(false, "normal")
    30.times { |i| env.store.save(Hue::Look.new("Look #{i}", pal, "dark")) }
    pics = (0...12).map { |i| p = File.join(env.dir, "pics", "p#{i}.png"); FileUtils.mkdir_p(File.dirname(p)); File.copy(env.picture("sunset"), p); p }
    LooksSpec.run(env, pics) do |t, app|
      LooksSpec.fits_kit?(t).should be_true, "parts #{LooksSpec.parts(t)}, depth #{LooksSpec.depth(t)}"
      t.press K::TWO, K::CTRL
      app.shown_looks.size.should eq Looks::App::MAX_ROWS
      LooksSpec.fits_kit?(t).should be_true, "parts #{LooksSpec.parts(t)}, depth #{LooksSpec.depth(t)}"
      20.times { app.history.value = app.history.value + [Looks::HistoryEntry.new(1_i64, "looks-x", app.looks.value.first)] }
      t.press K::THREE, K::CTRL
      LooksSpec.fits_kit?(t).should be_true, "parts #{LooksSpec.parts(t)}, depth #{LooksSpec.depth(t)}"
    end
  end
end

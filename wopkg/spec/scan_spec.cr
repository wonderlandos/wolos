require "spec"
require "file_utils"
require "../src/wopkg/scan"

include Wopkg

# A stand-in for clamscan: prints what the real one prints and exits like it.
def fake_clamscan(dir : String, body : String) : String
  path = "#{dir}/clamscan"
  File.write(path, "#!/bin/sh\nfor a; do LAST=$a; done\n#{body}\n")
  File.chmod(path, 0o755)
  path
end

describe Wopkg::Scan do
  it "scans third-party installs and skips what a signed repository already vouches for" do
    Scan.needed?("aur").should be_true
    Scan.needed?("appimage").should be_true
    Scan.needed?("wol", third_party: true).should be_true
    Scan.needed?("wol", third_party: false).should be_false
    Scan.needed?("pacman").should be_false
    Scan.needed?("flatpak").should be_false
    Scan.needed?("recipe").should be_true
  end

  it "reports a clean file" do
    d = File.tempname("wopkg-scan")
    Dir.mkdir_p(d)
    bin = fake_clamscan(d, %(echo "$LAST: OK"; echo; echo "Infected files: 0"; exit 0))
    File.write("#{d}/f", "x")
    File.write("#{d}/daily.cvd", "db")
    v = Scan.run(["#{d}/f"], clamscan: bin, db: d, now: Time.utc)
    v.clean?.should be_true
    v.message.should contain("No known malware")
    FileUtils.rm_rf(d)
  end

  it "stops on a hit with one plain sentence naming the signature" do
    d = File.tempname("wopkg-scan")
    Dir.mkdir_p(d)
    bin = fake_clamscan(d, %(echo "$LAST: Unix.Trojan.Chaos-1 FOUND"; echo "Infected files: 1"; exit 1))
    File.write("#{d}/f", "x")
    File.write("#{d}/daily.cvd", "db")
    v = Scan.run(["#{d}/f"], clamscan: bin, db: d, now: Time.utc)
    v.infected?.should be_true
    v.hits.should eq(["Unix.Trojan.Chaos-1"])
    v.message.should eq("ClamAV found Unix.Trojan.Chaos-1 in the download, so nothing was installed.")
    v.message.should_not contain("\n")
    FileUtils.rm_rf(d)
  end

  it "says so when ClamAV is not installed, instead of implying a clean result" do
    v = Scan.run(["/nonexistent"], clamscan: nil, db: "/nonexistent", now: Time.utc)
    v.unavailable?.should be_true
    v.clean?.should be_false
    v.message.should contain("not installed")
    v.message.should contain("not checked")
  end

  it "says so when the signature database is missing or old" do
    d = File.tempname("wopkg-scan")
    Dir.mkdir_p(d)
    bin = fake_clamscan(d, %(echo "$LAST: OK"; exit 0))
    File.write("#{d}/f", "x")
    missing = Scan.run(["#{d}/f"], clamscan: bin, db: "#{d}/none", now: Time.utc)
    missing.unavailable?.should be_true
    missing.message.should contain("signature")
    File.write("#{d}/daily.cvd", "db")
    File.utime(Time.utc - 30.days, Time.utc - 30.days, "#{d}/daily.cvd")
    old = Scan.run(["#{d}/f"], clamscan: bin, db: d, now: Time.utc)
    old.stale?.should be_true
    old.clean?.should be_false
    old.message.should contain("30 days")
    old.message.should contain("not a clean result")
    File.utime(Time.utc, Time.utc, "#{d}/daily.cvd")
    Scan.run(["#{d}/f"], clamscan: bin, db: d, now: Time.utc).clean?.should be_true
    FileUtils.rm_rf(d)
  end

  it "treats a scanner error as not checked, never as clean" do
    d = File.tempname("wopkg-scan")
    Dir.mkdir_p(d)
    bin = fake_clamscan(d, %(echo "ERROR: Can't open file" >&2; exit 2))
    File.write("#{d}/f", "x")
    File.write("#{d}/daily.cld", "db")
    v = Scan.run(["#{d}/f"], clamscan: bin, db: d, now: Time.utc)
    v.clean?.should be_false
    v.unavailable?.should be_true
    FileUtils.rm_rf(d)
  end

  it "needs a deliberate override to go on after a hit, and none after a clean scan" do
    hit = Scan::Verdict.new(Scan::Kind::Infected, ["X"], "m")
    clean = Scan::Verdict.new(Scan::Kind::Clean, [] of String, "m")
    Scan.may_continue?(clean, override: false).should be_true
    Scan.may_continue?(hit, override: false).should be_false
    Scan.may_continue?(hit, override: true).should be_true
  end

  it "never lets the scan run an option-looking path as a flag" do
    argv = Scan.argv("clamscan", ["-evil", "/tmp/a"], "/db")
    argv.index("--").should_not be_nil
    argv.last(2).should eq(["-evil", "/tmp/a"])
  end
end

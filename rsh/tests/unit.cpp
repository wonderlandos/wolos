// SPDX-License-Identifier: GPL-2.0-or-later
// rsh unit tests: the reader (statement starts, the command-line rule, the command-line grammar),
// patterns and argument safety. Plain checks, no framework: `make test` runs build/unit.
#include <unistd.h>

#include <cstdio>
#include <cstdlib>
#include <string>

#include "exec.hpp"
#include "glob.hpp"
#include "transpile.hpp"

using namespace rsh;

static int failures = 0, checks = 0;
#define CHECK(cond)                                                                \
    do {                                                                           \
        ++checks;                                                                  \
        if (!(cond)) {                                                             \
            ++failures;                                                            \
            std::fprintf(stderr, "%s:%d: FAILED: %s\n", __FILE__, __LINE__, #cond); \
        }                                                                          \
    } while (0)

static Result tx(const std::string& src) {
    Known k;
    collect_defs(src, "", k.methods);
    return transpile(src, "t", k);
}
// Kinds of each line: "RC" = line 1 Ruby, line 2 command.
static std::string kinds(const std::string& src) {
    Result r = tx(src);
    size_t n = 1;
    for (char c : src) n += c == '\n';
    std::string k(n, 'R');
    for (auto& m : r.marks)
        if (m.kind == 'C' && m.line >= 1 && size_t(m.line) <= n) k[size_t(m.line) - 1] = 'C';
    return r.ok() ? k : "ERR";
}
static bool is_cmd(const std::string& line) { return kinds(line) == "C"; }
static std::string err(const std::string& src) {
    Result r = tx(src);
    for (auto& d : r.diags)
        if (!d.warning) return d.msg;
    return "";
}

struct NoHost : Host {
    std::vector<std::string> a{"x", "y z"};
    bool stage(int, const std::string& l, std::string& o, bool& k) override {
        o = l;
        k = true;
        return true;
    }
    int source(const std::vector<std::string>&) override { return 0; }
    const std::vector<std::string>& args() override { return a; }
    void history(int) override {}
};

static std::vector<std::string> argv_of(const std::string& text, std::vector<Value> vals = {}) {
    std::vector<std::string> v, out;
    std::string e;
    int idx = parse_command(text, "t", 1, v, e);
    if (idx < 0) return {"ERR: " + e};
    NoHost h;
    for (auto& w : table()[size_t(idx)].items[0].pipeline[0].words)
        if (!expand(w, vals, h, out, e)) return {"ERR: " + e};
    return out;
}

int main() {
    // -- the rule: what is a command line
    CHECK(is_cmd("git pull --ff-only"));
    CHECK(is_cmd("ls -l"));
    CHECK(is_cmd("./deploy.sh now"));
    CHECK(is_cmd("../bin/x"));
    CHECK(is_cmd("/usr/bin/env true"));
    CHECK(is_cmd("~/bin/tool"));
    CHECK(is_cmd("apt-get install x"));
    CHECK(is_cmd("make"));
    CHECK(is_cmd("$ puts hello"));
    CHECK(is_cmd("LANG=C sort file"));
    CHECK(is_cmd("sleep 5 &"));
    CHECK(!is_cmd("puts 1"));
    CHECK(!is_cmd("printf \"%d\", 1"));
    CHECK(!is_cmd("x = 1"));
    CHECK(!is_cmd("x += 1"));
    CHECK(!is_cmd("foo(1)"));
    CHECK(!is_cmd("foo.bar"));
    CHECK(!is_cmd("Foo.new"));
    CHECK(!is_cmd("FOO=1"));
    CHECK(!is_cmd("if x then y end"));
    CHECK(!is_cmd("[1, 2].each { |v| p v }"));
    CHECK(!is_cmd("loop do"));
    CHECK(!is_cmd("ok?"));
    CHECK(!is_cmd("save!"));
    CHECK(!is_cmd("/regex/.match(x)"));
    CHECK(!is_cmd("foo, bar = 1, 2"));
    CHECK(!is_cmd("each_line do |l|"));
    CHECK(kinds("x = 1\nx -1") == "RR");                  // a local wins (Ruby's own rule)
    CHECK(kinds("def deploy(t)\nend\ndeploy 1") == "RRR"); // a method in the file wins
    CHECK(kinds("ls = 3\nls") == "RR");
    CHECK(kinds("if grep -q x f\n  echo yes\nend") == "CCR");
    CHECK(kinds("[1].each do |f|\n  cp #{f} /b\nend") == "RCR");
    CHECK(kinds("x = [1].map { |f| f }\nls") == "RC");
    CHECK(kinds("s = <<~T\n  ls -l\nT\nls") == "RRRC"); // here-doc bodies are text
    CHECK(kinds("s = \"\nls\n\"\nls") == "RRRC");       // so are strings
    CHECK(kinds("x = 1 +\n  y") == "RR");               // a continued expression
    CHECK(kinds("=begin\nls\n=end\nls") == "RRRC");
    CHECK(kinds("ok = (grep -q a b)") == "C");
    CHECK(kinds("a; ls") == "C");

    // -- the generated Ruby keeps every line where it was
    {
        Result r = tx("echo a \\\n  b\nputs 2\n");
        CHECK(r.ok());
        int nl = 0;
        for (char c : r.ruby) nl += c == '\n';
        CHECK(nl == 3);
        CHECK(r.ruby.find("puts 2") != std::string::npos);
    }
    {
        Result r = tx("x = `date +%s`");
        CHECK(r.ok() && r.ruby.find("__rsh_cap(") != std::string::npos);
        Result p = tx("x = %x(date)");
        CHECK(p.ok() && p.ruby.find("__rsh_cap(") != std::string::npos);
    }

    // -- mistakes the reader refuses
    CHECK(err("echo $1").find("ARGV") != std::string::npos);
    CHECK(err("echo $(date)").find("capture") != std::string::npos);
    CHECK(err("rm x if dry").find("quote it") != std::string::npos);
    CHECK(err("echo 'open").find("not closed") != std::string::npos);
    CHECK(err("a &&") != "" || err("a && ") != "");
    CHECK(err("cat <<EOF\nno end").find("end word") != std::string::npos);
    CHECK(err("( ls )") == "");  // a Ruby group around a command line
    CHECK(err("ls ( x )").find("group") != std::string::npos);

    // -- arguments: one value = one argument, never split, never a pattern
    {
        std::vector<Value> v(1);
        v[0].items = {"a b; rm -rf / *"};
        auto a = argv_of("echo #{x}", v);
        CHECK(a.size() == 2 && a[1] == "a b; rm -rf / *");
        auto b = argv_of("echo pre-#{x}-post", v);
        CHECK(b.size() == 2 && b[1] == "pre-a b; rm -rf / *-post");
    }
    {
        std::vector<Value> v(1);
        v[0].items = {"one", "two words"};
        v[0].array = true;
        auto a = argv_of("echo #{x}", v);
        CHECK(a.size() == 3 && a[2] == "two words");
        auto b = argv_of("echo -x=#{x}", v);
        CHECK(b.size() == 1 && b[0].find("ERR") == 0);
    }
    CHECK(argv_of("echo 'a b' \"c d\" e\\ f") == (std::vector<std::string>{"echo", "a b", "c d", "e f"}));
    CHECK(argv_of("echo x{1,2}y") == (std::vector<std::string>{"echo", "x1y", "x2y"}));
    CHECK(argv_of("echo '{1,2}'") == (std::vector<std::string>{"echo", "{1,2}"}));
    CHECK(argv_of("echo $*") == (std::vector<std::string>{"echo", "x", "y z"}));
    setenv("RSH_UNIT", "v w", 1);
    CHECK(argv_of("echo $RSH_UNIT ${RSH_UNIT}x") == (std::vector<std::string>{"echo", "v w", "v wx"}));
    unsetenv("RSH_UNSET_X");
    CHECK(argv_of("echo $RSH_UNSET_X")[0].find("ERR") == 0);
    setenv("HOME", "/home/u", 1);
    CHECK(argv_of("echo ~/x ~") == (std::vector<std::string>{"echo", "/home/u/x", "/home/u"}));
    CHECK(argv_of("echo '~'") == (std::vector<std::string>{"echo", "~"}));

    // -- the grammar: pipelines, lists, redirects
    {
        std::vector<std::string> v;
        std::string e;
        int i = parse_command("a | b |& c && d || e > f 2>&1 < g &", "t", 1, v, e);
        CHECK(i >= 0);
        const Line& L = table()[size_t(i)];
        CHECK(L.items.size() == 3);
        CHECK(L.items[0].pipeline.size() == 3 && L.items[0].op == Op::And);
        CHECK(L.items[0].pipeline[1].pipe_err);
        CHECK(L.items[1].op == Op::Or);
        CHECK(L.items[2].op == Op::Bg);
        CHECK(L.items[2].pipeline[0].redirs.size() == 3);
        CHECK(L.items[2].pipeline[0].redirs[1].k == Redir::Dup && L.items[2].pipeline[0].redirs[1].dupfd == 1);
    }

    // -- patterns
    char dir[] = "/tmp/rsh-unit-XXXXXX";
    CHECK(mkdtemp(dir) != nullptr);
    std::string d = dir;
    for (const char* f : {"a.log", "b.log", ".hidden.log", "c.txt", "sub/d.log", "sub/deep/e.log"}) {
        std::string p = d + "/" + f;
        std::string parent = p.substr(0, p.rfind('/'));
        std::string mk = "mkdir -p '" + parent + "' && touch '" + p + "'";
        CHECK(std::system(mk.c_str()) == 0);
    }
    CHECK(glob(glob_escape(d) + "/*.log").size() == 2);
    CHECK(glob(glob_escape(d) + "/.*.log").size() == 1);
    CHECK(glob(glob_escape(d) + "/**/*.log").size() == 4);
    CHECK(glob(glob_escape(d) + "/*.none").empty());
    CHECK(glob_has_meta("a*") && !glob_has_meta("a\\*") && !glob_has_meta("plain"));
    CHECK(brace_expand("{a,b{c,d}}").size() == 3);
    CHECK(brace_expand("{a}").size() == 1);
    std::string rm = "rm -rf '" + d + "'";
    CHECK(std::system(rm.c_str()) == 0);

    std::printf("%d checks, %d failed\n", checks, failures);
    return failures ? 1 : 0;
}

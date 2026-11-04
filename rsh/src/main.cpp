// SPDX-License-Identifier: GPL-2.0-or-later
// rsh: Ruby with command lines. `rsh file [args]` runs a script (also through `#!/usr/bin/rsh`),
// `rsh` alone on a terminal is interactive. See rsh(1) and docs/modules/rsh.md.
#include <dirent.h>
#include <sys/stat.h>
#include <unistd.h>

#include <cctype>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <set>

#include "edit.hpp"
#include "exec.hpp"
#include "rt.hpp"

using namespace rsh;

namespace {

const char* USAGE =
    "usage: rsh [options] [file [arguments]]\n"
    "  -c TEXT     run TEXT (-e is the same)\n"
    "  -s          read the script from standard input (arguments follow, after --)\n"
    "  -n          check the script: read and compile it, run nothing\n"
    "  --lint      -n, plus warnings (unknown commands, risky forms)\n"
    "  --explain   show how each line is read: C command line, R Ruby, X a capture inside Ruby\n"
    "  --ruby      show the Ruby text rsh runs\n"
    "  -i          interactive (-l, a login window, is the same)\n"
    "  -v          version\n";

// ---- interactive: colours and completion ---------------------------------------------------------

const char* GREEN = "\x1b[32m";
const char* RED = "\x1b[31m";
const char* YELLOW = "\x1b[33m";
const char* BLUE = "\x1b[34m";
const char* MAGENTA = "\x1b[35m";
const char* CYAN = "\x1b[36m";
const char* GREY = "\x1b[90m";
const char* RESET = "\x1b[0m";

Known* repl_known = nullptr;

std::string paint_ruby(const std::string& s, size_t from) {
    std::string o = s.substr(0, from);
    size_t i = from;
    while (i < s.size()) {
        char c = s[i];
        if (c == '#' && !(i + 1 < s.size() && s[i + 1] == '{')) {
            o += GREY + s.substr(i) + RESET;
            break;
        }
        if (c == '"' || c == '\'' || c == '`') {
            size_t e = i + 1;
            while (e < s.size() && s[e] != c) e += s[e] == '\\' ? 2 : 1;
            e = std::min(e + 1, s.size());
            o += (c == '`' ? MAGENTA : YELLOW) + s.substr(i, e - i) + RESET;
            i = e;
            continue;
        }
        if (std::isdigit((unsigned char)c)) {
            size_t e = i;
            while (e < s.size() && (std::isalnum((unsigned char)s[e]) || s[e] == '_' || s[e] == '.')) ++e;
            o += CYAN + s.substr(i, e - i) + RESET;
            i = e;
            continue;
        }
        if (std::isalpha((unsigned char)c) || c == '_') {
            size_t e = i;
            while (e < s.size() && (std::isalnum((unsigned char)s[e]) || s[e] == '_' || s[e] == '?' || s[e] == '!')) ++e;
            std::string w = s.substr(i, e - i);
            if (is_keyword(w)) o += BLUE + w + RESET;
            else if (std::isupper((unsigned char)w[0])) o += CYAN + w + RESET;
            else o += w;
            i = e;
            continue;
        }
        if (c == ':' && i + 1 < s.size() && std::isalpha((unsigned char)s[i + 1])) {
            size_t e = i + 1;
            while (e < s.size() && (std::isalnum((unsigned char)s[e]) || s[e] == '_')) ++e;
            o += CYAN + s.substr(i, e - i) + RESET;
            i = e;
            continue;
        }
        o += c;
        ++i;
    }
    return o;
}

std::string paint_cmd(const std::string& s, size_t from) {
    std::string o = s.substr(0, from);
    size_t i = from;
    bool first = true;
    while (i < s.size()) {
        char c = s[i];
        if (c == ' ' || c == '\t') {
            o += c;
            ++i;
            continue;
        }
        if (c == '|' || c == '&' || c == ';' || c == '>' || c == '<') {
            size_t e = i;
            while (e < s.size() && std::strchr("|&;<>", s[e])) ++e;
            o += BLUE + s.substr(i, e - i) + RESET;
            if (s.compare(i, 2, "||") == 0 || s.compare(i, 2, "&&") == 0 || s[i] == '|' || s[i] == ';') first = true;
            i = e;
            continue;
        }
        if (c == '#' && !(i + 1 < s.size() && s[i + 1] == '{')) {
            o += GREY + s.substr(i) + RESET;
            break;
        }
        size_t e = i;
        std::string w;
        std::string painted;
        while (e < s.size() && s[e] != ' ' && s[e] != '\t' && !std::strchr("|&;<>", s[e])) {
            if (s[e] == '\'' || s[e] == '"') {
                size_t q = e + 1;
                while (q < s.size() && s[q] != s[e]) q += s[q] == '\\' ? 2 : 1;
                q = std::min(q + 1, s.size());
                painted += YELLOW + s.substr(e, q - e) + RESET;
                w += s.substr(e, q - e);
                e = q;
            } else if (s[e] == '#' && e + 1 < s.size() && s[e + 1] == '{') {
                size_t q = s.find('}', e);
                q = q == std::string::npos ? s.size() : q + 1;
                painted += MAGENTA + s.substr(e, q - e) + RESET;
                w += s.substr(e, q - e);
                e = q;
            } else if (s[e] == '$') {
                size_t q = e + 1;
                while (q < s.size() && (std::isalnum((unsigned char)s[q]) || s[q] == '_' || s[q] == '{' || s[q] == '}' || s[q] == '?')) ++q;
                painted += CYAN + s.substr(e, q - e) + RESET;
                w += s.substr(e, q - e);
                e = q;
            } else {
                painted += s[e];
                w += s[e];
                ++e;
            }
        }
        if (first) {
            std::string name = w;
            if (name.compare(0, 2, "$ ") == 0) name = name.substr(2);
            bool found = is_builtin(name) || !find_program(name).empty();
            o += std::string(found ? GREEN : RED) + w + RESET;
            first = false;
        } else if (!w.empty() && w[0] == '-') o += CYAN + w + RESET;
        else o += painted;
        i = e;
    }
    return o;
}

std::string paint(const std::string& line) {
    if (!repl_known) return line;
    size_t before = table().size();
    Known k = *repl_known;
    Result r = transpile(line, "(rsh)", k);
    table().resize(before);
    size_t lead = 0;
    while (lead < line.size() && (line[lead] == ' ' || line[lead] == '\t')) ++lead;
    bool command = false;
    for (auto& m : r.marks)
        if (m.kind == 'C' && m.line == 1) command = true;
    if (command) {
        // Ruby words before the command (if, unless...) stay Ruby-coloured.
        size_t start = lead;
        for (const char* kw : {"if ", "unless ", "while ", "until ", "elsif ", "$ "}) {
            size_t n = std::strlen(kw);
            if (line.compare(start, n, kw) == 0) {
                std::string pre = line.substr(0, start + n);
                return paint_ruby(pre, 0) + paint_cmd(line.substr(start + n), 0);
            }
        }
        return paint_cmd(line, lead);
    }
    return paint_ruby(line, 0);
}

std::vector<std::string> complete(const std::string& line, size_t start) {
    std::string word = line.substr(start);
    std::set<std::string> c;
    bool first = true;
    for (size_t k = 0; k < start; ++k)
        if (line[k] != ' ' && line[k] != '\t') first = false;
    if (first && word.find('/') == std::string::npos) {
        for (const char* b : {"cd", "pushd", "popd", "dirs", "export", "unset", "exec", "umask", "jobs", "fg", "bg", "wait",
                              "source", "echo", "type", "pwd", "history"})
            if (!std::strncmp(b, word.c_str(), word.size())) c.insert(b);
        for (const char* const* w = RUBY_WORDS; *w; ++w)
            if (!std::strncmp(*w, word.c_str(), word.size()) && std::islower((unsigned char)**w)) c.insert(*w);
        if (repl_known)
            for (auto& m : repl_known->methods)
                if (!m.compare(0, word.size(), word)) c.insert(m);
        const char* p = getenv("PATH");
        std::string path = p ? p : "/usr/bin";
        size_t i = 0;
        while (i <= path.size() && !word.empty()) {
            size_t e = path.find(':', i);
            if (e == std::string::npos) e = path.size();
            std::string dir = path.substr(i, e - i);
            if (DIR* d = opendir(dir.c_str())) {
                while (dirent* x = readdir(d))
                    if (!std::strncmp(x->d_name, word.c_str(), word.size()) && x->d_name[0] != '.') c.insert(x->d_name);
                closedir(d);
            }
            i = e + 1;
        }
        if (!c.empty()) return std::vector<std::string>(c.begin(), c.end());
    }
    if (repl_known && !first)
        for (auto& l : repl_known->locals)
            if (!word.empty() && !l.compare(0, word.size(), word)) c.insert(l);
    std::string dir = ".", base = word, shown_dir;
    size_t slash = word.rfind('/');
    if (slash != std::string::npos) {
        shown_dir = word.substr(0, slash + 1);
        dir = slash == 0 ? "/" : word.substr(0, slash);
        base = word.substr(slash + 1);
        if (dir[0] == '~') dir = home_of("") + dir.substr(1);
    }
    if (DIR* d = opendir(dir.c_str())) {
        while (dirent* x = readdir(d)) {
            if (std::strncmp(x->d_name, base.c_str(), base.size())) continue;
            if (x->d_name[0] == '.' && (base.empty() || base[0] != '.')) continue;
            if (!std::strcmp(x->d_name, ".") || !std::strcmp(x->d_name, "..")) continue;
            std::string full = (dir == "/" ? "/" : dir + "/") + x->d_name;
            struct stat st;
            bool isdir = stat(full.c_str(), &st) == 0 && S_ISDIR(st.st_mode);
            c.insert(shown_dir + x->d_name + (isdir ? "/" : ""));
        }
        closedir(d);
    }
    return std::vector<std::string>(c.begin(), c.end());
}

std::string short_cwd() {
    char buf[4096];
    if (!getcwd(buf, sizeof buf)) return "?";
    std::string c = buf, h = home_of("");
    if (!h.empty() && h != "/" && c.compare(0, h.size(), h) == 0 && (c.size() == h.size() || c[h.size()] == '/'))
        c = "~" + c.substr(h.size());
    return c;
}

int interactive() {
    setup_interactive();
    mrb_state* mrb = rt_open("rsh", {});
    if (!mrb) return 70;
    std::string home = home_of("");
    const char* xdg = getenv("XDG_CONFIG_HOME");
    std::string init = (xdg && *xdg ? std::string(xdg) : home + "/.config") + "/rsh/init.rsh";
    if (access(init.c_str(), R_OK) == 0) {
        bool ok;
        std::string src = read_file(init, ok);
        if (ok) rt_run(mrb, src, init, true);
    }
    const char* xs = getenv("XDG_STATE_HOME");
    std::string state = xs && *xs ? std::string(xs) : home + "/.local/state";
    if (!xs || !*xs) {
        mkdir((home + "/.local").c_str(), 0700);
    }
    mkdir(state.c_str(), 0700);
    mkdir((state + "/rsh").c_str(), 0700);
    Editor ed;
    ed.history_file = state + "/rsh/history";
    ed.load();
    ed.paint = paint;
    ed.complete = complete;
    Known known;
    repl_known = &known;
    std::string buf;
    int status = 0;
    for (;;) {
        report_jobs();
        std::string prompt;
        if (!buf.empty()) prompt = std::string(GREY) + "  ... " + RESET;
        else if (!rt_prompt(mrb, prompt))
            prompt = std::string("\x1b[1;34m") + short_cwd() + RESET + (status ? std::string(RED) + " " + std::to_string(status) + RESET : "") +
                     " \x1b[1;35m>\x1b[0m ";
        std::string line;
        if (!ed.read(prompt, line)) {
            if (!buf.empty()) {
                buf.clear();
                continue;
            }
            break;
        }
        buf += line + "\n";
        bool exited, incomplete;
        int st = rt_eval(mrb, buf, known, exited, incomplete);
        if (incomplete) continue;
        std::string entry = buf.substr(0, buf.size() - 1);
        ed.add(entry);
        rt_add_history(entry);
        buf.clear();
        status = st;
        if (exited) {
            rt_close(mrb);
            return st;
        }
    }
    rt_close(mrb);
    return status;
}

}  // namespace

int main(int argc, char** argv) {
    std::string code, file;
    bool have_code = false, from_stdin = false, check = false, lint = false, explain = false, show_ruby = false, inter = false;
    int i = 1;
    for (; i < argc; ++i) {
        std::string a = argv[i];
        if (a == "--") {
            ++i;
            break;
        }
        if (a.empty() || a[0] != '-' || a == "-") break;
        if ((a == "-c" || a == "-e") && i + 1 < argc) code = argv[++i], have_code = true;
        else if (a == "-s") from_stdin = true;
        else if (a == "-n") check = true;
        else if (a == "--lint") check = lint = true;
        else if (a == "--explain") explain = true;
        else if (a == "--ruby") show_ruby = true;
        else if (a == "-i" || a == "-l") inter = true;
        else if (a == "-v" || a == "--version") {
            std::printf("rsh %s\n", RSH_VERSION);
            return 0;
        } else if (a == "-h" || a == "--help") {
            std::fputs(USAGE, stdout);
            return 0;
        } else {
            std::fprintf(stderr, "rsh: unknown option %s\n%s", a.c_str(), USAGE);
            return 2;
        }
    }
    std::vector<std::string> args;
    std::string src;
    if (have_code) {
        file = "-c";
        src = code;
    } else if (from_stdin) {
        file = "-";
        bool ok;
        src = read_file("-", ok);
    } else if (i < argc) {
        file = argv[i++];
        bool ok;
        src = read_file(file, ok);
        if (!ok) {
            std::fprintf(stderr, "rsh: %s: %s\n", file.c_str(), std::strerror(errno));
            return 127;
        }
    } else if (inter || isatty(0)) {
        return interactive();
    } else {
        file = "-";
        bool ok;
        src = read_file("-", ok);
    }
    for (; i < argc; ++i) args.push_back(argv[i]);

    if (explain || show_ruby) {
        Known k;
        size_t slash = file.rfind('/');
        collect_defs(src, slash == std::string::npos ? "." : file.substr(0, slash), k.methods);
        Result r = transpile(src, file, k);
        for (auto& d : r.diags) std::fprintf(stderr, "%s:%d: %s\n", file.c_str(), d.line, d.msg.c_str());
        if (show_ruby) std::fputs(r.ruby.c_str(), stdout);
        else {
            std::vector<char> kind;
            size_t n = 1;
            for (char c : src) n += c == '\n';
            kind.assign(n + 1, 'R');
            for (auto& m : r.marks)
                if (m.line > 0 && size_t(m.line) <= n && kind[size_t(m.line)] != 'C') kind[size_t(m.line)] = m.kind;
            size_t ln = 1, b = 0;
            while (b <= src.size()) {
                size_t e = src.find('\n', b);
                if (e == std::string::npos) e = src.size();
                std::string text = src.substr(b, e - b);
                bool blank = text.find_first_not_of(" \t") == std::string::npos;
                std::printf("%4zu %c  %s\n", ln, blank ? ' ' : kind[ln], text.c_str());
                ++ln;
                b = e + 1;
                if (e == src.size()) break;
            }
        }
        return r.ok() ? 0 : 2;
    }
    mrb_state* mrb = rt_open(file, args);
    if (!mrb) return 70;
    int rc = check ? rt_check(mrb, src, file, lint) : rt_run(mrb, src, file, true);
    std::fflush(stdout);
    rt_close(mrb);
    return rc;
}

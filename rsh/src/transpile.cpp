// SPDX-License-Identifier: GPL-2.0-or-later
// rsh: the reader. A small Ruby lexer finds statement starts; at each one the first word decides
// between Ruby and a command line (docs/modules/rsh.md, "How rsh reads a line"). The rule looks
// only at the text, never at PATH, so a file means the same thing on every computer.
#include "transpile.hpp"

#include <cctype>
#include <cstdio>
#include <cstring>

namespace rsh {

std::vector<Line>& table() {
    static std::vector<Line> t;
    return t;
}

const char* const RUBY_WORDS[] = {
    "puts", "print", "p", "pp", "printf", "sprintf", "format", "require", "require_relative", "load",
    "raise", "fail", "loop", "exit", "abort", "at_exit", "catch", "throw", "lambda", "proc",
    "rand", "srand", "gets", "warn", "binding", "caller", "Integer", "Float", "String", "Array",
    "Hash", "attr_reader", "attr_writer", "attr_accessor", "private", "public", "protected",
    "module_function", "include", "extend", "define_method", "system", "trap", "freeze",
    "__method__", nullptr};

static const char* const KEYWORDS[] = {
    "alias", "and", "begin", "BEGIN", "break", "case", "class", "def", "defined?", "do", "else",
    "elsif", "end", "END", "ensure", "false", "for", "if", "in", "module", "next", "nil", "not",
    "or", "redo", "rescue", "retry", "return", "self", "super", "then", "true", "undef", "unless",
    "until", "when", "while", "yield", "__FILE__", "__LINE__", "__ENCODING__", nullptr};

static bool in_list(const char* const* l, const std::string& w) {
    for (; *l; ++l)
        if (w == *l) return true;
    return false;
}
bool is_keyword(const std::string& w) { return in_list(KEYWORDS, w); }
bool is_ruby_word(const std::string& w) { return in_list(RUBY_WORDS, w); }

bool Result::ok() const {
    for (auto& d : diags)
        if (!d.warning) return false;
    return true;
}

namespace {

bool ident_start(char c) { return std::isalpha((unsigned char)c) || c == '_' || (unsigned char)c >= 0x80; }
bool ident_char(char c) { return std::isalnum((unsigned char)c) || c == '_' || (unsigned char)c >= 0x80; }
bool blank(char c) { return c == ' ' || c == '\t' || c == '\r'; }

int count_nl(const std::string& s) {
    int n = 0;
    for (char c : s) n += c == '\n';
    return n;
}

// From s[i] (just inside a '{'), the index of the matching '}', or npos. Knows strings, nested
// braces and comments; enough for #{...} and Ruby blocks inside command lines.
size_t match_brace(const std::string& s, size_t i, size_t lim) {
    int depth = 0;
    while (i < lim) {
        char c = s[i];
        if (c == '\\') {
            i += 2;
            continue;
        }
        if (c == '"' || c == '\'' || c == '`') {
            char q = c;
            ++i;
            while (i < lim && s[i] != q) {
                if (s[i] == '\\') ++i;
                else if (q != '\'' && s[i] == '#' && i + 1 < lim && s[i + 1] == '{') {
                    size_t e = match_brace(s, i + 2, lim);
                    if (e == std::string::npos) return e;
                    i = e;
                }
                ++i;
            }
            ++i;
            continue;
        }
        if (c == '#' && !(i + 1 < lim && s[i + 1] == '{')) {
            while (i < lim && s[i] != '\n') ++i;
            continue;
        }
        if (c == '{') ++depth;
        else if (c == '}') {
            if (depth == 0) return i;
            --depth;
        }
        ++i;
    }
    return std::string::npos;
}

std::string trim(const std::string& s) {
    size_t a = 0, b = s.size();
    while (a < b && std::isspace((unsigned char)s[a])) ++a;
    while (b > a && std::isspace((unsigned char)s[b - 1])) --b;
    return s.substr(a, b - a);
}

std::string first_line(const std::string& s) { return trim(s.substr(0, s.find('\n'))); }

std::string fragment(const std::string& src, const std::string& file, int line, Known& known, Result& res);

// ---- command lines -------------------------------------------------------------------------

struct CmdParser {
    const std::string& s;
    size_t i, lim;
    char close;  // ')' or '}': an unquoted close at a word start ends the line
    std::string file;
    int line0;
    Known& known;
    Result& res;
    std::vector<std::string> vals;
    std::string err;
    int nl = 0;
    bool capture = false;
    bool do_block = false;
    struct Pending {
        size_t item, cmd, redir;
        std::string delim;
        bool squiggly, literal;
    };
    std::vector<Pending> here;
    size_t n_items = 0, n_cmds = 0;  // where the command being read will sit (here-doc fix-up)
    void push_item(Line& L, Item& it) {
        L.items.push_back(std::move(it));
        it = Item();
        ++n_items;
        n_cmds = 0;
    }

    CmdParser(const std::string& s_, size_t i_, size_t lim_, char close_, const std::string& f, int l, Known& k,
              Result& r)
        : s(s_), i(i_), lim(lim_), close(close_), file(f), line0(l), known(k), res(r) {}

    bool fail(const std::string& m) {
        if (err.empty()) err = m;
        return false;
    }
    int value(const std::string& ruby_src) {
        vals.push_back(fragment(ruby_src, file, line0 + nl, known, res));
        return int(vals.size()) - 1;
    }
    void skip_blank() {
        while (i < lim) {
            if (blank(s[i])) ++i;
            else if (s[i] == '\\' && i + 1 < lim && s[i + 1] == '\n') {
                i += 2;
                ++nl;
            } else break;
        }
    }
    void skip_newlines() {  // after | && || a line may go on
        for (;;) {
            skip_blank();
            if (i < lim && s[i] == '\n') {
                ++i;
                ++nl;
            } else if (i < lim && s[i] == '#' && !(i + 1 < lim && s[i + 1] == '{')) {
                while (i < lim && s[i] != '\n') ++i;
            } else break;
        }
    }
    bool line_end() const {
        if (i >= lim) return true;
        char c = s[i];
        return c == '\n' || c == ';' || (close && c == close);
    }
    static void add(Word& w, Part::Kind k, const std::string& t) {
        if (!w.parts.empty() && w.parts.back().k == k && (k == Part::Lit || k == Part::Pat)) w.parts.back().s += t;
        else {
            Part p;
            p.k = k;
            p.s = t;
            w.parts.push_back(p);
        }
    }
    static void add_val(Word& w, Part::Kind k, int idx, const std::string& name = "") {
        Part p;
        p.k = k;
        p.idx = idx;
        p.s = name;
        w.parts.push_back(p);
    }
    bool interp(Word& w) {  // at "#{"
        size_t e = match_brace(s, i + 2, lim);
        if (e == std::string::npos) return fail("#{ without a closing }");
        std::string expr = s.substr(i + 2, e - i - 2);
        nl += count_nl(expr);
        add_val(w, Part::Val, value("(" + expr + ")"));
        i = e + 1;
        return true;
    }
    bool dollar(Word& w) {  // at '$'
        char c = i + 1 < lim ? s[i + 1] : 0;
        if (c == '?') add_val(w, Part::Status, -1), i += 2;
        else if (c == '*') add_val(w, Part::Args, -1), i += 2;
        else if (c == '!') add_val(w, Part::BgPid, -1), i += 2;
        else if (c == '(') return fail("$(...) is not rsh: capture with #{`command`} or a Ruby variable");
        else if (c == '{') {
            size_t e = s.find('}', i + 2);
            if (e == std::string::npos || e >= lim) return fail("${ without a closing }");
            add_val(w, Part::Env, -1, s.substr(i + 2, e - i - 2));
            i = e + 1;
        } else if (std::isdigit((unsigned char)c) || c == '@' || c == '#')
            return fail(std::string("$") + c + " is sh: write #{ARGV[0]}, #{ARGV.size} or $*");
        else if (ident_start(c)) {
            size_t e = i + 1;
            while (e < lim && ident_char(s[e])) ++e;
            add_val(w, Part::Env, -1, s.substr(i + 1, e - i - 1));
            i = e;
        } else {
            add(w, Part::Lit, "$");
            ++i;
        }
        return true;
    }
    bool dquote(Word& w) {  // at '"'
        ++i;
        add(w, Part::Lit, "");
        while (i < lim && s[i] != '"') {
            char c = s[i];
            if (c == '\\' && i + 1 < lim) {
                char d = s[i + 1];
                if (d == '\n') ++nl;
                else if (std::strchr("\"\\$#`", d)) add(w, Part::Lit, std::string(1, d));
                else add(w, Part::Lit, std::string("\\") + d);
                i += 2;
            } else if (c == '#' && i + 1 < lim && s[i + 1] == '{') {
                if (!interp(w)) return false;
            } else if (c == '$') {
                if (!dollar(w)) return false;
            } else {
                if (c == '\n') ++nl;
                add(w, Part::Lit, std::string(1, c));
                ++i;
            }
        }
        if (i >= lim) return fail("a \" string is not closed");
        ++i;
        return true;
    }
    static bool word_stop(char c) { return blank(c) || c == '\n' || std::strchr(";|&<>()", c); }
    bool word(Word& w) {
        while (i < lim && !word_stop(s[i])) {
            char c = s[i];
            if (c == '#' && i + 1 < lim && s[i + 1] == '{') {
                if (!interp(w)) return false;
            } else if (c == '\'') {
                size_t e = s.find('\'', i + 1);
                if (e == std::string::npos || e >= lim) return fail("a ' string is not closed");
                std::string t = s.substr(i + 1, e - i - 1);
                nl += count_nl(t);
                add(w, Part::Lit, t);
                i = e + 1;
            } else if (c == '"') {
                if (!dquote(w)) return false;
            } else if (c == '\\' && i + 1 < lim) {
                if (s[i + 1] == '\n') ++nl;
                else add(w, Part::Lit, std::string(1, s[i + 1]));
                i += 2;
            } else if (c == '$') {
                if (!dollar(w)) return false;
            } else if (c == '`') {
                return fail("a capture inside a command line: put it in a variable first");
            } else {
                add(w, Part::Pat, std::string(1, c));
                ++i;
            }
        }
        return true;
    }
    static bool plain(const Word& w, const char* t) {
        return w.parts.size() == 1 && w.parts[0].k == Part::Pat && w.parts[0].s == t;
    }
    bool finish_cmd(Item& it, Cmd& c) {
        if (c.words.empty() && c.block < 0) {
            if (!c.env.empty()) return fail("NAME=value needs a command after it (for the shell's own variables use export or ENV)");
            return fail("a command is missing");
        }
        it.pipeline.push_back(std::move(c));
        c = Cmd();
        ++n_cmds;
        return true;
    }
    bool redirect(Cmd& c, int fd) {
        Redir r;
        if (s[i] == '&') {  // &> &>>
            i += 2;
            r.k = Redir::Both;
            if (i < lim && s[i] == '>') r.k = Redir::BothAppend, ++i;
        } else if (s[i] == '>') {
            r.fd = fd < 0 ? 1 : fd;
            if (i + 1 < lim && s[i + 1] == '>') r.k = Redir::Append, i += 2;
            else if (i + 1 < lim && s[i + 1] == '&') {
                i += 2;
                if (i >= lim || !std::isdigit((unsigned char)s[i])) return fail(">& needs a file descriptor number");
                r.k = Redir::Dup;
                r.dupfd = s[i++] - '0';
                c.redirs.push_back(r);
                return true;
            } else r.k = Redir::Out, ++i;
        } else {  // '<'
            r.fd = fd < 0 ? 0 : fd;
            if (s.compare(i, 3, "<<<") == 0) {
                i += 3;
                skip_blank();
                r.k = Redir::Here;
                if (!word(r.target) || r.target.parts.empty()) return fail("<<< needs a word");
                add(r.target, Part::Lit, "\n");
                c.redirs.push_back(r);
                return true;
            }
            if (s.compare(i, 2, "<<") == 0) {
                i += 2;
                Pending p;
                p.squiggly = i < lim && (s[i] == '~' || s[i] == '-');
                if (p.squiggly) ++i;
                p.literal = i < lim && (s[i] == '\'' || s[i] == '"');
                char q = p.literal ? s[i++] : 0;
                size_t b = i;
                while (i < lim && (q ? s[i] != q : ident_char(s[i]))) ++i;
                p.delim = s.substr(b, i - b);
                if (q) ++i;
                p.literal = q == '\'';
                if (p.delim.empty()) return fail("<< needs an end word (<<EOF)");
                if (capture) return fail("here-documents do not go in a capture");
                r.k = Redir::Here;
                c.redirs.push_back(r);
                p.redir = c.redirs.size() - 1;
                p.item = n_items;
                p.cmd = n_cmds;
                here.push_back(p);
                return true;
            }
            r.k = Redir::In;
            ++i;
        }
        skip_blank();
        if (!word(r.target) || r.target.parts.empty()) return fail("a redirect needs a file name");
        c.redirs.push_back(r);
        return true;
    }
    bool env_prefix(Cmd& c, Word& w) {
        if (!c.words.empty() || w.parts.empty() || w.parts[0].k != Part::Pat) return false;
        const std::string& t = w.parts[0].s;
        size_t eq = t.find('=');
        if (eq == 0 || eq == std::string::npos || !ident_start(t[0])) return false;
        for (size_t k = 0; k < eq; ++k)
            if (!ident_char(t[k])) return false;
        Word v;
        if (eq + 1 < t.size()) add(v, Part::Pat, t.substr(eq + 1));
        for (size_t k = 1; k < w.parts.size(); ++k) v.parts.push_back(w.parts[k]);
        c.env.emplace_back(t.substr(0, eq), v);
        return true;
    }

    bool parse(Line& L) {
        Item it;
        Cmd c;
        bool ruby_after = false;
        for (;;) {
            skip_blank();
            if (line_end()) break;
            char ch = s[i];
            char nx = i + 1 < lim ? s[i + 1] : 0;
            if (ch == '#' && nx != '{') {
                while (i < lim && s[i] != '\n') ++i;
                break;
            }
            if (ch == '}' && close == '}') break;
            if (ch == '|') {
                ruby_after = false;
                if (nx == '|') {
                    if (!finish_cmd(it, c)) return false;
                    it.op = Op::Or;
                    push_item(L, it);
                    i += 2;
                    skip_newlines();
                    continue;
                }
                if (nx == '&') c.pipe_err = true, ++i;
                if (!finish_cmd(it, c)) return false;
                ++i;
                skip_newlines();
                if (i < lim && s[i] == '{') {
                    size_t e = match_brace(s, i + 1, lim);
                    if (e == std::string::npos) return fail("a Ruby stage { |line| ... } is not closed");
                    std::string body = s.substr(i, e + 1 - i);
                    nl += count_nl(body);
                    Cmd rb;
                    rb.block = value("proc " + body);
                    i = e + 1;
                    c = std::move(rb);
                    ruby_after = true;
                }
                continue;
            }
            if (ch == '&' && nx == '&') {
                ruby_after = false;
                if (!finish_cmd(it, c)) return false;
                it.op = Op::And;
                push_item(L, it);
                i += 2;
                skip_newlines();
                continue;
            }
            if (ch == '&' && nx != '>') {
                if (!finish_cmd(it, c)) return false;
                it.op = Op::Bg;
                push_item(L, it);
                ++i;
                continue;
            }
            if (ruby_after) return fail("after a Ruby stage only | may follow");
            if (ch == '>' || ch == '<' || (ch == '&' && nx == '>')) {
                if (!redirect(c, -1)) return false;
                continue;
            }
            if (std::isdigit((unsigned char)ch) && (nx == '>' || nx == '<') && c.words.size() + c.env.size() > 0) {
                ++i;
                if (!redirect(c, ch - '0')) return false;
                continue;
            }
            if (ch == '(' || ch == ')') return fail("( ) do not group commands in rsh: use Ruby (if, begin ... end)");
            if (ch == '!' && c.words.empty() && (nx == ' ' || nx == '\t')) {
                it.bang = true;
                ++i;
                continue;
            }
            Word w;
            if (!word(w)) return false;
            if (w.parts.empty()) return fail(std::string("unexpected ") + s[i]);
            if (env_prefix(c, w)) continue;
            if (!c.words.empty()) {
                if (plain(w, "do")) {
                    skip_blank();
                    if (plain(c.words[0], "cd") && line_end() && L.items.empty()) {
                        do_block = true;
                        break;
                    }
                    return fail("`do` in a command line: only `cd dir do ... end` takes a block; quote it ('do') to pass the word");
                }
                for (const char* kw : {"if", "unless", "while", "until", "rescue", "then"})
                    if (plain(w, kw))
                        return fail(std::string("`") + kw + "` in a command line is passed as a word; quote it ('" + kw +
                                    "') or put the Ruby on its own line");
            }
            c.words.push_back(std::move(w));
        }
        if (!c.words.empty() || c.block >= 0 || !c.env.empty() || !c.redirs.empty()) {
            if (!finish_cmd(it, c)) return false;
        }
        if (!it.pipeline.empty()) push_item(L, it);
        else if (!L.items.empty() && L.items.back().op != Op::Bg) return fail("a command is missing after && or ||");
        if (L.items.empty()) return fail("an empty command line");
        if (L.items.back().op == Op::Bg) {
        } else L.items.back().op = Op::End;
        if (!here.empty()) return bodies(L);
        return true;
    }
    // Here-document bodies: the lines after this one, up to each end word.
    bool bodies(Line& L) {
        if (i >= lim || s[i] != '\n') return fail("a here-document must end its line");
        ++i;
        ++nl;
        for (auto& p : here) {
            std::vector<std::string> lines;
            bool found = false;
            while (i < lim) {
                size_t e = s.find('\n', i);
                if (e == std::string::npos || e > lim) e = lim;
                std::string ln = s.substr(i, e - i);
                i = e < lim ? e + 1 : lim;
                ++nl;
                if ((p.squiggly ? trim(ln) : ln) == p.delim) {
                    found = true;
                    break;
                }
                lines.push_back(ln);
            }
            if (!found) return fail("here-document without its end word " + p.delim);
            if (p.squiggly) {  // drop the common indentation
                size_t ind = std::string::npos;
                for (auto& ln : lines) {
                    if (trim(ln).empty()) continue;
                    size_t k = 0;
                    while (k < ln.size() && blank(ln[k])) ++k;
                    if (k < ind) ind = k;
                }
                for (auto& ln : lines) ln = ln.size() >= ind && ind != std::string::npos ? ln.substr(ind) : trim(ln);
            }
            std::string text;
            for (auto& ln : lines) text += ln + "\n";
            Word w;
            if (p.literal) add(w, Part::Lit, text);
            else {
                // #{...} and $NAME work; a backslash keeps the next character.
                size_t k = 0;
                std::string lit;
                auto flush = [&] {
                    if (!lit.empty()) add(w, Part::Lit, lit);
                    lit.clear();
                };
                while (k < text.size()) {
                    char ch = text[k];
                    if (ch == '\\' && k + 1 < text.size()) {
                        lit += text[k + 1];
                        k += 2;
                    } else if (ch == '#' && k + 1 < text.size() && text[k + 1] == '{') {
                        size_t e = match_brace(text, k + 2, text.size());
                        if (e == std::string::npos) return fail("#{ without } in a here-document");
                        flush();
                        add_val(w, Part::Val, value("(" + text.substr(k + 2, e - k - 2) + ")"));
                        k = e + 1;
                    } else if (ch == '$' && k + 1 < text.size() && (ident_start(text[k + 1]) || text[k + 1] == '{')) {
                        flush();
                        size_t b = k + 1, e;
                        if (text[b] == '{') {
                            e = text.find('}', b);
                            if (e == std::string::npos) return fail("${ without }");
                            add_val(w, Part::Env, -1, text.substr(b + 1, e - b - 1));
                            k = e + 1;
                        } else {
                            e = b;
                            while (e < text.size() && ident_char(text[e])) ++e;
                            add_val(w, Part::Env, -1, text.substr(b, e - b));
                            k = e;
                        }
                    } else {
                        lit += ch;
                        ++k;
                    }
                }
                flush();
                if (w.parts.empty()) add(w, Part::Lit, "");
            }
            L.items[p.item].pipeline[p.cmd].redirs[p.redir].target = w;
        }
        return true;
    }
};

// ---- the Ruby side -------------------------------------------------------------------------

struct Tx {
    const std::string& s;
    size_t n, i = 0;
    std::string out;
    std::string file;
    int line;
    Known& known;
    Result& res;
    enum Prev { Start, Value, Op } prev = Start;
    bool stmt = true, cond = false, space = true, after_dot = false, lambda = false, rescue_line = false;
    std::vector<char> frames{'T'};
    struct Heredoc {
        std::string id;
        bool dash;
    };
    std::vector<Heredoc> heredocs;

    Tx(const std::string& src, const std::string& f, int l, Known& k, Result& r)
        : s(src), n(src.size()), file(f), line(l), known(k), res(r) {}

    bool at(const char* t) const { return s.compare(i, std::strlen(t), t) == 0; }
    void copy(size_t e) {
        line += count_nl(s.substr(i, e - i));
        out.append(s, i, e - i);
        i = e;
    }
    void error(int ln, const std::string& m) { res.diags.push_back({ln, m, false}); }

    void flush_heredocs() {
        for (auto& h : heredocs) {
            while (i < n) {
                size_t e = s.find('\n', i);
                size_t end = e == std::string::npos ? n : e + 1;
                std::string ln = s.substr(i, (e == std::string::npos ? n : e) - i);
                copy(end);
                if ((h.dash ? trim(ln) : ln) == h.id) break;
            }
        }
        heredocs.clear();
    }

    // Strings with #{...}: the code inside is read as Ruby (so captures in it are rewritten).
    void dstring(char close, char open) {
        int depth = 0;
        while (i < n) {
            char c = s[i];
            if (c == '\\' && i + 1 < n) {
                copy(i + 2);
                continue;
            }
            if (c == '#' && i + 1 < n && s[i + 1] == '{') {
                out += "#{";
                i += 2;
                frames.push_back('I');
                Prev p0 = prev;
                bool st0 = stmt;
                prev = Start;
                stmt = false;
                code(true);
                prev = p0;
                stmt = st0;
                frames.pop_back();
                if (i < n) out += '}', ++i;
                continue;
            }
            if (open && c == open) ++depth;
            else if (c == close) {
                if (depth == 0) {
                    out += c;
                    ++i;
                    return;
                }
                --depth;
            }
            if (c == '\n') ++line;
            out += c;
            ++i;
        }
    }

    void block_params() {
        size_t k = i;
        while (k < n && blank(s[k])) ++k;
        if (k < n && s[k] == '|' && !(k + 1 < n && s[k + 1] == '|')) {
            size_t e = s.find('|', k + 1);
            if (e == std::string::npos) return;
            std::string p = s.substr(k + 1, e - k - 1);
            record_idents(p);
            copy(e + 1);
        }
    }
    void record_idents(const std::string& p) {
        size_t k = 0;
        bool in_default = false;
        int depth = 0;
        while (k < p.size()) {
            char c = p[k];
            if (c == '(' || c == '[' || c == '{') ++depth;
            if (c == ')' || c == ']' || c == '}') --depth;
            if (c == '=' && depth == 0) in_default = true;
            if (c == ',' && depth == 0) in_default = false;
            if (ident_start(c)) {
                size_t e = k;
                while (e < p.size() && ident_char(p[e])) ++e;
                if (!in_default) known.locals.insert(p.substr(k, e - k));
                k = e;
                continue;
            }
            ++k;
        }
    }

    // Is the identifier at i..j followed by an assignment?
    bool assigned(size_t j) const {
        size_t k = j;
        while (k < n && blank(s[k])) ++k;
        if (k >= n) return false;
        if (s[k] == '=' && !(k + 1 < n && (s[k + 1] == '=' || s[k + 1] == '~' || s[k + 1] == '>'))) return true;
        for (const char* op : {"+=", "-=", "*=", "/=", "%=", "||=", "&&=", "|=", "&=", "<<=", ">>=", "**=", "^="})
            if (s.compare(k, std::strlen(op), op) == 0) return true;
        return false;
    }

    char close_char() const {
        char f = frames.back();
        return f == 'G' ? ')' : f == 'B' ? '}' : 0;
    }

    void command(size_t from, bool value_mode) {
        int ln = line;
        CmdParser p(s, from, n, close_char(), file, ln, known, res);
        Line L;
        L.file = file;
        L.line = ln;
        // here-doc fix-up needs item/cmd numbers: they are known when parse() ends, recorded below.
        bool ok = p.parse(L);
        if (!ok) {
            error(ln + p.nl, p.err);
            size_t e = s.find('\n', p.i);
            i = e == std::string::npos ? n : e;
            line += p.nl;
            out += "nil";
            return;
        }
        L.text = first_line(s.substr(from, p.i - from));
        int idx = int(table().size());
        table().push_back(std::move(L));
        std::string call = std::string(p.do_block ? "__rsh_cd_do(" : "__rsh_cmd(") + std::to_string(idx) + ", [";
        for (size_t k = 0; k < p.vals.size(); ++k) call += (k ? ", " : "") + p.vals[k];
        call += "]";
        if (value_mode) call += ", true";
        call += ")";
        if (p.do_block) call += " do";
        out += call;
        int pad = p.nl - count_nl(call);
        for (int k = 0; k < pad; ++k) out += '\n';
        line += p.nl;
        res.marks.push_back({ln, 'C', table()[idx].text});
        i = p.i;
        if (p.do_block || (i > 0 && s[i - 1] == '\n')) {
            stmt = true;
            prev = Start;
        } else {
            stmt = false;
            prev = Value;
        }
    }

    // `...` or %x{...}: one command line, captured.
    void capture(char close, char open) {
        int ln = line;
        size_t k = i;
        int depth = 0;
        while (k < n) {
            if (s[k] == '\\') {
                k += 2;
                continue;
            }
            if (s[k] == '#' && k + 1 < n && s[k + 1] == '{') {
                size_t e = match_brace(s, k + 2, n);
                if (e == std::string::npos) break;
                k = e + 1;
                continue;
            }
            if (open && s[k] == open) ++depth;
            else if (s[k] == close) {
                if (depth == 0) break;
                --depth;
            }
            ++k;
        }
        if (k >= n) {
            error(ln, "a capture is not closed");
            i = n;
            return;
        }
        CmdParser p(s, i, k, 0, file, ln, known, res);
        p.capture = true;
        Line L;
        L.file = file;
        L.line = ln;
        bool ok = p.parse(L);
        if (ok && p.i < k) ok = p.fail("a capture holds one command line");
        if (!ok) {
            error(ln, p.err);
            out += "nil";
        } else {
            L.text = first_line(s.substr(i, k - i));
            int idx = int(table().size());
            table().push_back(std::move(L));
            std::string call = "__rsh_cap(" + std::to_string(idx) + ", [";
            for (size_t m = 0; m < p.vals.size(); ++m) call += (m ? ", " : "") + p.vals[m];
            call += "])";
            out += call;
            int pad = count_nl(s.substr(i, k - i)) - count_nl(call);
            for (int m = 0; m < pad; ++m) out += '\n';
            res.marks.push_back({ln, 'X', table()[idx].text});
        }
        line = ln + count_nl(s.substr(i, k - i));
        i = k + 1;
        prev = Value;
    }

    bool try_command() {
        bool value_mode = cond || frames.back() == 'G';
        if (s[i] == '$' && i + 1 < n && blank(s[i + 1])) {
            i += 2;
            while (i < n && blank(s[i])) ++i;
            command(i, value_mode);
            return true;
        }
        if (at("./") || at("../") || (s[i] == '~' && i + 1 < n && (s[i + 1] == '/' || ident_start(s[i + 1])))) {
            command(i, value_mode);
            return true;
        }
        if (s[i] == '/' && i + 1 < n && (ident_char(s[i + 1]) || s[i + 1] == '.')) {
            size_t e = i;
            while (e < n && !blank(s[e]) && s[e] != '\n') ++e;
            std::string w = s.substr(i, e - i);
            if (w.back() != '/' && w.find("/.") == std::string::npos && w.find("=~") == std::string::npos) {
                command(i, value_mode);
                return true;
            }
            return false;
        }
        if (!ident_start(s[i])) return false;
        size_t j = i;
        while (j < n && ident_char(s[j])) ++j;
        std::string w = s.substr(i, j - i);
        if (j < n && (s[j] == '?' || s[j] == '!')) return false;
        if (is_keyword(w)) return false;
        if (std::isupper((unsigned char)w[0])) {
            // NAME=value command ...
            if (j < n && s[j] == '=' && j + 1 < n && !std::strchr("=~ \t\n", s[j + 1])) {
                size_t k = j;
                while (k < n && !blank(s[k]) && s[k] != '\n') ++k;
                while (k < n && blank(s[k])) ++k;
                if (k < n && s[k] != '\n' && s[k] != '#' && s[k] != ';') {
                    command(i, value_mode);
                    return true;
                }
            }
            return false;
        }
        if (known.locals.count(w) || known.methods.count(w) || is_ruby_word(w)) return false;
        if (j < n && std::strchr(".([,=:", s[j])) return false;
        if (assigned(j)) return false;
        size_t k = j;
        while (k < n && blank(s[k])) ++k;
        if (s.compare(k, 2, "do") == 0 && (k + 2 >= n || !ident_char(s[k + 2]))) return false;
        if (k < n && s[k] == '{') return false;
        command(i, value_mode);
        return true;
    }

    bool regex_ok(bool space_before) const {
        if (prev != Value) return true;
        return space_before && i + 1 < n && s[i + 1] != ' ' && s[i + 1] != '=';
    }

    void code(bool interp) {
        size_t depth0 = frames.size();
        while (i < n) {
            char c = s[i];
            bool bol = i == 0 || s[i - 1] == '\n';
            if (bol && !interp) {
                if (at("=begin") && (i + 6 >= n || std::isspace((unsigned char)s[i + 6]))) {
                    size_t e = s.find("\n=end", i);
                    e = e == std::string::npos ? n : s.find('\n', e + 1);
                    copy(e == std::string::npos ? n : e);
                    continue;
                }
                if (at("__END__") && (i + 7 == n || s[i + 7] == '\n')) {
                    copy(n);
                    return;
                }
            }
            if (blank(c)) {
                out += c;
                ++i;
                space = true;
                continue;
            }
            if (c == '\\' && i + 1 < n && s[i + 1] == '\n') {
                copy(i + 2);
                space = true;
                continue;
            }
            if (c == '\n') {
                out += '\n';
                ++i;
                ++line;
                space = true;
                flush_heredocs();
                char f = frames.back();
                if ((f == 'T' || f == 'B' || f == 'G') && prev != Op) {
                    stmt = true;
                    cond = false;
                }
                rescue_line = false;
                continue;
            }
            bool sp = space;
            space = false;
            if (c == '#' && !(i + 1 < n && s[i + 1] == '{')) {
                size_t e = s.find('\n', i);
                copy(e == std::string::npos ? n : e);
                space = true;
                continue;
            }
            if (c == ';') {
                out += ';';
                ++i;
                stmt = true;
                cond = false;
                prev = Start;
                continue;
            }
            if ((stmt || cond) && !interp && try_command()) {
                cond = false;
                continue;
            }
            stmt = false;
            cond = false;
            bool dot = after_dot;
            after_dot = false;
            bool lam = lambda;
            lambda = false;

            if (c == '"') {
                out += c;
                ++i;
                dstring('"', 0);
                prev = Value;
                continue;
            }
            if (c == '\'') {
                size_t k = i + 1;
                while (k < n && s[k] != '\'') k += s[k] == '\\' ? 2 : 1;
                copy(k < n ? k + 1 : n);
                prev = Value;
                continue;
            }
            if (c == '`') {
                ++i;
                capture('`', 0);
                continue;
            }
            if (ident_start(c)) {
                size_t j = i;
                while (j < n && ident_char(s[j])) ++j;
                if (j < n && (s[j] == '?' || s[j] == '!') && !(j + 1 < n && s[j + 1] == '=' && !(j + 2 < n && s[j + 2] == '=')))
                    ++j;
                std::string w = s.substr(i, j - i);
                if (dot) {
                    copy(j);
                    prev = Value;
                    continue;
                }
                if (j < n && s[j] == ':' && !(j + 1 < n && s[j + 1] == ':')) {  // label
                    copy(j + 1);
                    prev = Op;
                    continue;
                }
                if (is_keyword(w)) {
                    copy(j);
                    if (w == "def") {
                        size_t k = i;
                        while (k < n && blank(s[k])) ++k;
                        if (s.compare(k, 5, "self.") == 0) k += 5;
                        size_t b = k;
                        while (k < n && (ident_char(s[k]) || s[k] == '?' || s[k] == '!' || s[k] == '=')) ++k;
                        if (k > b) known.methods.insert(s.substr(b, k - b));
                        size_t e = s.find('\n', k);
                        record_idents(s.substr(k, (e == std::string::npos ? n : e) - k));
                        copy(k);
                        prev = Value;
                    } else if (w == "if" || w == "unless" || w == "while" || w == "until" || w == "elsif") {
                        prev = Op;
                        cond = true;
                    } else if (w == "then" || w == "else" || w == "begin" || w == "ensure") {
                        prev = Start;
                        stmt = true;
                    } else if (w == "do") {
                        block_params();
                        prev = Start;
                        stmt = true;
                    } else if (w == "for") {
                        size_t k = i;
                        while (k < n && blank(s[k])) ++k;
                        size_t b = k;
                        while (k < n && ident_char(s[k])) ++k;
                        known.locals.insert(s.substr(b, k - b));
                        prev = Op;
                    } else if (w == "rescue") {
                        rescue_line = true;
                        prev = Value;
                    } else if (w == "and" || w == "or" || w == "not" || w == "in" || w == "when" || w == "case" ||
                               w == "class" || w == "module" || w == "alias" || w == "undef") {
                        prev = Op;
                    } else prev = Value;
                    continue;
                }
                if (assigned(j)) known.locals.insert(w);
                if (rescue_line && out.size() >= 2) {
                    size_t k = out.size();
                    while (k > 0 && blank(out[k - 1])) --k;
                    if (k >= 2 && out.compare(k - 2, 2, "=>") == 0) known.locals.insert(w);
                }
                copy(j);
                prev = Value;
                continue;
            }
            if (std::isdigit((unsigned char)c)) {
                size_t j = i;
                while (j < n && (ident_char(s[j]) || (s[j] == '.' && j + 1 < n && std::isdigit((unsigned char)s[j + 1])))) ++j;
                copy(j);
                prev = Value;
                continue;
            }
            if (c == '$' && i + 1 < n) {
                size_t j = i + 1;
                if (ident_start(s[j]) || std::isdigit((unsigned char)s[j]))
                    while (j < n && ident_char(s[j])) ++j;
                else ++j;
                copy(j);
                prev = Value;
                continue;
            }
            if (c == '@') {
                size_t j = i + 1;
                if (j < n && s[j] == '@') ++j;
                while (j < n && ident_char(s[j])) ++j;
                copy(j);
                prev = Value;
                continue;
            }
            if (c == ':') {
                if (i + 1 < n && s[i + 1] == ':') {
                    copy(i + 2);
                    after_dot = true;
                    prev = Op;
                } else if (i + 1 < n && s[i + 1] == '"') {
                    out += ":\"";
                    i += 2;
                    dstring('"', 0);
                    prev = Value;
                } else if (i + 1 < n && ident_start(s[i + 1])) {
                    size_t j = i + 1;
                    while (j < n && ident_char(s[j])) ++j;
                    if (j < n && (s[j] == '?' || s[j] == '!' || s[j] == '=')) ++j;
                    copy(j);
                    prev = Value;
                } else {
                    copy(i + 1);
                    prev = Op;
                }
                continue;
            }
            if (c == '%' && i + 1 < n && (prev != Value || (sp && s[i + 1] != ' ' && s[i + 1] != '='))) {
                size_t j = i + 1;
                char type = 0;
                if (j < n && std::strchr("qQwWiIrsx", s[j]) && j + 1 < n && !std::isalnum((unsigned char)s[j + 1]) &&
                    !std::isspace((unsigned char)s[j + 1]))
                    type = s[j++];
                char open = j < n ? s[j] : 0;
                if (open && !std::isalnum((unsigned char)open) && !std::isspace((unsigned char)open)) {
                    char close = open == '(' ? ')' : open == '[' ? ']' : open == '{' ? '}' : open == '<' ? '>' : open;
                    char nest = close == open ? 0 : open;
                    if (type == 'x') {
                        i = j + 1;
                        capture(close, nest);
                        continue;
                    }
                    copy(j + 1);
                    dstring(close, nest);
                    if (type == 'r')
                        while (i < n && std::isalpha((unsigned char)s[i])) copy(i + 1);
                    prev = Value;
                    continue;
                }
            }
            if (c == '/' && regex_ok(sp)) {
                out += '/';
                ++i;
                dstring('/', 0);
                while (i < n && std::isalpha((unsigned char)s[i])) copy(i + 1);
                prev = Value;
                continue;
            }
            if (c == '?' && prev != Value && i + 1 < n && !std::isspace((unsigned char)s[i + 1])) {
                size_t j = i + 1;
                if (s[j] == '\\') j += 2;
                else ++j;
                if (j >= n || !ident_char(s[j])) {
                    copy(j);
                    prev = Value;
                    continue;
                }
            }
            if (c == '<' && i + 2 < n && s[i + 1] == '<' && (prev != Value || (sp && s[i + 2] != ' '))) {
                size_t k = i + 2;
                bool dash = s[k] == '~' || s[k] == '-';
                if (dash) ++k;
                char q = (s[k] == '\'' || s[k] == '"' || s[k] == '`') ? s[k] : 0;
                size_t b = q ? k + 1 : k;
                size_t e = b;
                while (e < n && (q ? s[e] != q : ident_char(s[e]))) ++e;
                if (e > b && (q || ident_start(s[b]))) {
                    heredocs.push_back({s.substr(b, e - b), dash});
                    copy(q ? e + 1 : e);
                    prev = Value;
                    continue;
                }
            }
            if (c == '{') {
                bool block = prev == Value || lam;
                frames.push_back(block ? 'B' : 'H');
                copy(i + 1);
                if (block) {
                    block_params();
                    prev = Start;
                    stmt = true;
                } else prev = Op;
                continue;
            }
            if (c == '}') {
                if (interp && frames.size() == depth0) return;
                if (frames.size() > 1) frames.pop_back();
                copy(i + 1);
                prev = Value;
                continue;
            }
            if (c == '(') {
                bool call = prev == Value;
                frames.push_back(call ? 'P' : 'G');
                copy(i + 1);
                if (!call) {
                    stmt = true;
                    prev = Start;
                } else prev = Op;
                continue;
            }
            if (c == '[') {
                frames.push_back('P');
                copy(i + 1);
                prev = Op;
                continue;
            }
            if (c == ')' || c == ']') {
                if (frames.size() > 1) frames.pop_back();
                copy(i + 1);
                prev = Value;
                continue;
            }
            if (c == '-' && i + 1 < n && s[i + 1] == '>') {
                copy(i + 2);
                prev = Op;
                lambda = true;
                continue;
            }
            if (c == '.' || (c == '&' && i + 1 < n && s[i + 1] == '.')) {
                if (c == '.' && i + 1 < n && s[i + 1] == '.') {  // range
                    copy(i + 2);
                    prev = Op;
                    continue;
                }
                copy(i + (c == '&' ? 2 : 1));
                after_dot = true;
                prev = Op;
                continue;
            }
            copy(i + 1);
            prev = Op;
        }
    }
};

std::string fragment(const std::string& src, const std::string& file, int line, Known& known, Result& res) {
    Tx t(src, file, line, known, res);
    t.stmt = false;
    t.prev = Tx::Op;
    t.code(false);
    return t.out;
}

}  // namespace

void collect_defs(const std::string& src, const std::string& dir, std::set<std::string>& out, int depth) {
    size_t i = 0;
    while (i < src.size()) {
        size_t e = src.find('\n', i);
        if (e == std::string::npos) e = src.size();
        std::string ln = trim(src.substr(i, e - i));
        i = e + 1;
        if (ln.compare(0, 4, "def ") == 0) {
            size_t k = 4;
            while (k < ln.size() && blank(ln[k])) ++k;
            if (ln.compare(k, 5, "self.") == 0) k += 5;
            size_t b = k;
            while (k < ln.size() && (ident_char(ln[k]) || ln[k] == '?' || ln[k] == '!')) ++k;
            if (k > b) out.insert(ln.substr(b, k - b));
        } else if (!dir.empty() && depth < 8 && ln.compare(0, 17, "require_relative ") == 0) {
            size_t q = ln.find_first_of("\"'", 17);
            if (q == std::string::npos) continue;
            size_t qe = ln.find(ln[q], q + 1);
            if (qe == std::string::npos) continue;
            std::string rel = ln.substr(q + 1, qe - q - 1);
            for (const char* ext : {"", ".rsh", ".rb"}) {
                std::string path = (rel[0] == '/' ? "" : dir + "/") + rel + ext;
                FILE* f = std::fopen(path.c_str(), "rb");
                if (!f) continue;
                std::string body;
                char buf[8192];
                size_t r;
                while ((r = std::fread(buf, 1, sizeof buf, f)) > 0) body.append(buf, r);
                std::fclose(f);
                size_t slash = path.rfind('/');
                collect_defs(body, slash == std::string::npos ? "." : path.substr(0, slash), out, depth + 1);
                break;
            }
        }
    }
}

Result transpile(const std::string& src, const std::string& file, Known& known) {
    Result res;
    Tx t(src, file, 1, known, res);
    t.code(false);
    res.ruby = std::move(t.out);
    return res;
}

int parse_command(const std::string& text, const std::string& file, int line, std::vector<std::string>& values,
                  std::string& err) {
    Known k;
    Result r;
    CmdParser p(text, 0, text.size(), 0, file, line, k, r);
    p.capture = true;
    Line L;
    L.file = file;
    L.line = line;
    if (!p.parse(L) || p.i < text.size()) {
        err = p.err.empty() ? "one command line expected" : p.err;
        return -1;
    }
    L.text = first_line(text);
    values = p.vals;
    table().push_back(std::move(L));
    return int(table().size()) - 1;
}

}  // namespace rsh

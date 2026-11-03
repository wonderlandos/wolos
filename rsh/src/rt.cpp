// SPDX-License-Identifier: GPL-2.0-or-later
// rsh: bindings. Rule for every function the interpreter calls: no C++ object with a destructor
// may be alive when the interpreter raises (it unwinds with longjmp). Work happens in an inner
// scope; the raise comes after it.
#include "rt.hpp"

#include <mruby.h>
#include <mruby/array.h>
#include <mruby/class.h>
#include <mruby/compile.h>
#include <mruby/data.h>
#include <mruby/error.h>
#include <mruby/hash.h>
#include <mruby/irep.h>
#include <mruby/proc.h>
#include <mruby/string.h>
#include <mruby/variable.h>
#include <pcre2.h>
#include <signal.h>
#include <unistd.h>

#include <cerrno>
#include <cstdio>
#include <cstdlib>
#include <cstring>

#include "exec.hpp"
#include "glob.hpp"

extern char** environ;
extern "C" const uint8_t rsh_prelude[];

namespace rsh {

std::string read_file(const std::string& path, bool& ok) {
    std::string s;
    FILE* f = path == "-" ? stdin : std::fopen(path.c_str(), "rb");
    ok = f != nullptr;
    if (!f) return s;
    char buf[65536];
    size_t n;
    while ((n = std::fread(buf, 1, sizeof buf, f)) > 0) s.append(buf, n);
    if (f != stdin) std::fclose(f);
    return s;
}

namespace {

struct RtHost : Host {
    mrb_state* mrb = nullptr;
    mrb_value vals = mrb_nil_value();  // the Ruby values of the running line (procs inside)
    mrb_value exc = mrb_nil_value();
    std::vector<std::string> argv;
    std::vector<std::string> files;  // being loaded, innermost last (require_relative)
    std::vector<std::string> hist;

    bool stage(int val, const std::string& line, std::string& out, bool& keep) override;
    int source(const std::vector<std::string>& a) override;
    const std::vector<std::string>& args() override { return argv; }
    void history(int fd) override {
        for (size_t k = 0; k < hist.size(); ++k) {
            std::string l = std::to_string(k + 1) + "  " + hist[k] + "\n";
            if (write(fd, l.data(), l.size()) < 0) return;
        }
    }
};
RtHost host;

struct StageCall {
    mrb_value proc, arg;
};
mrb_value stage_body(mrb_state* mrb, void* ud) {
    auto* c = static_cast<StageCall*>(ud);
    return mrb_yield(mrb, c->proc, c->arg);
}

bool RtHost::stage(int val, const std::string& line, std::string& out, bool& keep) {
    int ai = mrb_gc_arena_save(mrb);
    StageCall c{mrb_ary_ref(mrb, vals, val), mrb_str_new(mrb, line.data(), line.size())};
    mrb_bool err = FALSE;
    mrb_value r = mrb_protect_error(mrb, stage_body, &c, &err);
    if (err) {
        exc = r;
        mrb_gc_register(mrb, exc);
        mrb_gc_arena_restore(mrb, ai);
        return false;
    }
    if (mrb_string_p(r)) {
        keep = true;
        out.assign(RSTRING_PTR(r), size_t(RSTRING_LEN(r)));
    } else if (mrb_test(r)) {
        keep = true;
        out = line;
    } else keep = false;
    mrb_gc_arena_restore(mrb, ai);
    return true;
}

RClass* rsh_mod(mrb_state* mrb) { return mrb_module_get(mrb, "Rsh"); }

mrb_value str(mrb_state* mrb, const std::string& s) { return mrb_str_new(mrb, s.data(), s.size()); }
std::string cxx(mrb_value v) { return std::string(RSTRING_PTR(v), size_t(RSTRING_LEN(v))); }

void raise_with(mrb_state* mrb, RClass* cls, mrb_value msg) { mrb_exc_raise(mrb, mrb_exc_new_str(mrb, cls, msg)); }

// [code, statuses, signal, raise?, argv, tail, interrupted?, output]
mrb_value outcome(mrb_state* mrb, const Outcome& oc) {
    mrb_value r = mrb_ary_new_capa(mrb, 8);
    mrb_ary_push(mrb, r, mrb_fixnum_value(oc.code));
    mrb_value st = mrb_ary_new(mrb);
    for (int s : oc.statuses) mrb_ary_push(mrb, st, mrb_fixnum_value(s));
    mrb_ary_push(mrb, r, st);
    mrb_ary_push(mrb, r, oc.sig ? mrb_fixnum_value(oc.sig) : mrb_nil_value());
    mrb_ary_push(mrb, r, mrb_bool_value(oc.raise));
    mrb_value av = mrb_ary_new(mrb);
    for (auto& a : oc.failed_argv) mrb_ary_push(mrb, av, str(mrb, a));
    mrb_ary_push(mrb, r, av);
    mrb_ary_push(mrb, r, str(mrb, oc.tail));
    mrb_ary_push(mrb, r, mrb_bool_value(oc.interrupted));
    mrb_ary_push(mrb, r, str(mrb, oc.out));
    return r;
}

// Rsh.__exec(line, values, flags): flags 1 condition, 2 capture, 4 relaxed.
mrb_value m_exec(mrb_state* mrb, mrb_value) {
    mrb_int idx, flags;
    mrb_value vals;
    mrb_get_args(mrb, "iAi", &idx, &vals, &flags);
    mrb_value result = mrb_nil_value(), errmsg = mrb_nil_value(), exc = mrb_nil_value();
    if (idx < 0 || size_t(idx) >= table().size()) mrb_raise(mrb, E_ARGUMENT_ERROR, "no such command line");
    mrb_value saved = host.vals;
    {
        std::vector<Value> vs;
        for (mrb_int k = 0; k < RARRAY_LEN(vals); ++k) {
            mrb_value x = mrb_ary_ref(mrb, vals, k);
            Value v;
            if (mrb_string_p(x)) v.items.push_back(cxx(x));
            else if (mrb_array_p(x)) {
                v.array = true;
                for (mrb_int j = 0; j < RARRAY_LEN(x); ++j) {
                    mrb_value e = mrb_ary_ref(mrb, x, j);
                    if (mrb_string_p(e)) v.items.push_back(cxx(e));
                }
            } else v.proc = true;
            vs.push_back(std::move(v));
        }
        Options o;
        o.cond = flags & 1;
        o.capture = flags & 2;
        o.relaxed = flags & 4;
        host.vals = vals;
        host.exc = mrb_nil_value();
        Outcome oc = run(int(idx), vs, host, o);
        host.vals = saved;
        if (oc.ruby_error) exc = host.exc;
        else if (!oc.error.empty()) errmsg = str(mrb, table()[size_t(idx)].text.empty() ? oc.error : oc.error);
        else result = outcome(mrb, oc);
    }
    if (!mrb_nil_p(exc)) {
        mrb_gc_unregister(mrb, exc);
        mrb_exc_raise(mrb, exc);
    }
    if (!mrb_nil_p(errmsg)) raise_with(mrb, mrb_class_get_under(mrb, rsh_mod(mrb), "Error"), errmsg);
    return result;
}

// Rsh.__argv(argv, flags): one program, these arguments.
mrb_value m_argv(mrb_state* mrb, mrb_value) {
    mrb_value av;
    mrb_int flags;
    mrb_get_args(mrb, "Ai", &av, &flags);
    mrb_value result = mrb_nil_value(), errmsg = mrb_nil_value();
    if (RARRAY_LEN(av) == 0) mrb_raise(mrb, E_ARGUMENT_ERROR, "no program given");
    {
        std::vector<std::string> a;
        for (mrb_int k = 0; k < RARRAY_LEN(av); ++k) {
            mrb_value x = mrb_ary_ref(mrb, av, k);
            if (mrb_string_p(x)) a.push_back(cxx(x));
        }
        Options o;
        o.cond = flags & 1;
        Outcome oc = run_argv(a, host, o);
        if (!oc.error.empty()) errmsg = str(mrb, oc.error);
        else result = outcome(mrb, oc);
    }
    if (!mrb_nil_p(errmsg)) raise_with(mrb, mrb_class_get_under(mrb, rsh_mod(mrb), "Error"), errmsg);
    return result;
}

mrb_value m_set_status(mrb_state* mrb, mrb_value) {
    mrb_value st;
    mrb_get_args(mrb, "o", &st);
    mrb_gv_set(mrb, mrb_intern_lit(mrb, "$?"), st);
    return st;
}

mrb_value m_set_match(mrb_state* mrb, mrb_value) {
    mrb_value md, caps;
    mrb_get_args(mrb, "oA", &md, &caps);
    mrb_gv_set(mrb, mrb_intern_lit(mrb, "$~"), md);
    char name[3] = {'$', '1', 0};
    for (int k = 0; k < 9; ++k) {
        name[1] = char('1' + k);
        mrb_gv_set(mrb, mrb_intern_cstr(mrb, name), k < RARRAY_LEN(caps) ? mrb_ary_ref(mrb, caps, k) : mrb_nil_value());
    }
    return md;
}

mrb_value m_chdir(mrb_state* mrb, mrb_value) {
    const char* d;
    mrb_get_args(mrb, "z", &d);
    if (chdir(d) != 0) mrb_raisef(mrb, E_RUNTIME_ERROR, "cd %s: %s", d, std::strerror(errno));
    char now[4096];
    if (getcwd(now, sizeof now)) setenv("PWD", now, 1);
    return mrb_nil_value();
}

mrb_value m_nullglob(mrb_state* mrb, mrb_value) {
    mrb_bool on;
    mrb_get_args(mrb, "b", &on);
    shell().nullglob = on;
    return mrb_bool_value(on);
}

mrb_value m_getenv(mrb_state* mrb, mrb_value) {
    const char* k;
    mrb_get_args(mrb, "z", &k);
    const char* v = getenv(k);
    return v ? mrb_str_new_cstr(mrb, v) : mrb_nil_value();
}

mrb_value m_setenv(mrb_state* mrb, mrb_value) {
    const char* k;
    mrb_value v;
    mrb_get_args(mrb, "zo", &k, &v);
    if (!*k || std::strchr(k, '=')) mrb_raise(mrb, E_ARGUMENT_ERROR, "not an environment variable name");
    if (mrb_nil_p(v)) unsetenv(k);
    else {
        const char* s = mrb_string_value_cstr(mrb, &v);
        setenv(k, s, 1);
    }
    return v;
}

mrb_value m_env_hash(mrb_state* mrb, mrb_value) {
    mrb_value h = mrb_hash_new(mrb);
    for (char** p = environ; *p; ++p) {
        const char* eq = std::strchr(*p, '=');
        if (!eq) continue;
        mrb_hash_set(mrb, h, mrb_str_new(mrb, *p, eq - *p), mrb_str_new_cstr(mrb, eq + 1));
    }
    return h;
}

mrb_value m_pid(mrb_state*, mrb_value) { return mrb_fixnum_value(getpid()); }
mrb_value m_nproc(mrb_state*, mrb_value) { return mrb_fixnum_value(sysconf(_SC_NPROCESSORS_ONLN)); }

mrb_value m_kill(mrb_state* mrb, mrb_value) {
    const char* s;
    mrb_int pid;
    mrb_get_args(mrb, "zi", &s, &pid);
    int sig = std::atoi(s);
    struct {
        const char* n;
        int v;
    } names[] = {{"TERM", SIGTERM}, {"KILL", SIGKILL}, {"INT", SIGINT}, {"HUP", SIGHUP}, {"USR1", SIGUSR1},
                 {"USR2", SIGUSR2}, {"STOP", SIGSTOP}, {"CONT", SIGCONT}, {"QUIT", SIGQUIT}, {"0", 0}};
    const char* n = std::strncmp(s, "SIG", 3) == 0 ? s + 3 : s;
    for (auto& x : names)
        if (!std::strcmp(n, x.n)) sig = x.v;
    if (kill(pid_t(pid), sig) != 0) mrb_raisef(mrb, E_RUNTIME_ERROR, "kill %d: %s", (int)pid, std::strerror(errno));
    return mrb_true_value();
}

mrb_value m_glob(mrb_state* mrb, mrb_value) {
    const char* p;
    mrb_get_args(mrb, "z", &p);
    mrb_value r = mrb_ary_new(mrb);
    {
        std::vector<std::string> all;
        for (auto& b : brace_expand(p))
            for (auto& m : glob(b)) all.push_back(m);
        for (auto& m : all) mrb_ary_push(mrb, r, str(mrb, m));
    }
    return r;
}

// ---- require_relative / load / source ---------------------------------------------------------

std::string dir_of(const std::string& f) {
    size_t s = f.rfind('/');
    return s == std::string::npos ? "." : s == 0 ? "/" : f.substr(0, s);
}

bool ends_with(const std::string& s, const char* t) {
    size_t n = std::strlen(t);
    return s.size() >= n && s.compare(s.size() - n, n, t) == 0;
}

// Compiles and runs `src` as file `file` at the top level; leaves an exception in mrb->exc.
void load_text(mrb_state* mrb, const std::string& src, const std::string& file, bool rsh_syntax, bool& syntax_error) {
    syntax_error = false;
    mrb_value code = mrb_nil_value();
    {
        Known k;
        if (rsh_syntax) {
            collect_defs(src, dir_of(file), k.methods);
            Result r = transpile(src, file, k);
            if (!r.ok()) {
                for (auto& d : r.diags)
                    if (!d.warning) std::fprintf(stderr, "rsh: %s:%d: %s\n", file.c_str(), d.line, d.msg.c_str());
                syntax_error = true;
                return;
            }
            code = str(mrb, r.ruby);
        } else code = str(mrb, src);
    }
    mrb_ccontext* c = mrb_ccontext_new(mrb);
    mrb_ccontext_filename(mrb, c, file.c_str());
    c->capture_errors = TRUE;
    int ai = mrb_gc_arena_save(mrb);
    host.files.push_back(file);
    mrb_load_nstring_cxt(mrb, RSTRING_PTR(code), RSTRING_LEN(code), c);
    host.files.pop_back();
    mrb_gc_arena_restore(mrb, ai);
    mrb_ccontext_free(mrb, c);
}

mrb_value m_require(mrb_state* mrb, mrb_value) {
    const char* rel;
    mrb_bool relative;
    mrb_get_args(mrb, "zb", &rel, &relative);
    mrb_value path = mrb_nil_value(), src = mrb_nil_value();
    bool once = relative;
    {
        std::string base = relative && !host.files.empty() ? dir_of(host.files.back()) : ".";
        std::string r = rel;
        std::string found;
        for (const char* ext : {"", ".rsh", ".rb"}) {
            std::string p = (r[0] == '/' ? "" : base + "/") + r + ext;
            if (access(p.c_str(), R_OK) == 0 && !ends_with(p, "/")) {
                found = p;
                break;
            }
        }
        if (!found.empty()) {
            char real[4096];
            if (realpath(found.c_str(), real)) found = real;
            bool ok;
            std::string body = read_file(found, ok);
            path = str(mrb, found);
            if (ok) src = str(mrb, body);
        }
    }
    if (mrb_nil_p(path)) mrb_raisef(mrb, mrb_class_get(mrb, "LoadError"), "cannot load such file -- %s", rel);
    if (mrb_nil_p(src)) mrb_raisef(mrb, mrb_class_get(mrb, "LoadError"), "cannot read %v", path);
    mrb_sym loaded = mrb_intern_lit(mrb, "$__rsh_loaded");
    mrb_value list = mrb_gv_get(mrb, loaded);
    if (!mrb_hash_p(list)) {
        list = mrb_hash_new(mrb);
        mrb_gv_set(mrb, loaded, list);
    }
    if (once && mrb_test(mrb_hash_get(mrb, list, path))) return mrb_false_value();
    mrb_hash_set(mrb, list, path, mrb_true_value());
    bool syn;
    std::string p = cxx(path);
    std::string s = cxx(src);
    bool rsh_syntax = !ends_with(p, ".rb");
    load_text(mrb, s, p, rsh_syntax, syn);
    std::string().swap(p);
    std::string().swap(s);
    if (syn) mrb_raisef(mrb, mrb_class_get(mrb, "ScriptError"), "%v has mistakes (see above)", path);
    if (mrb->exc) {
        mrb_value e = mrb_obj_value(mrb->exc);
        mrb->exc = nullptr;
        mrb_exc_raise(mrb, e);
    }
    return mrb_true_value();
}

int RtHost::source(const std::vector<std::string>& a) {
    if (a.size() < 2) {
        std::fprintf(stderr, "rsh: source needs a file\n");
        return 2;
    }
    bool ok;
    std::string body = read_file(a[1], ok);
    if (!ok) {
        std::fprintf(stderr, "rsh: source: %s: %s\n", a[1].c_str(), std::strerror(errno));
        return 1;
    }
    bool syn;
    load_text(mrb, body, a[1], !ends_with(a[1], ".rb"), syn);
    if (syn) return 2;
    if (mrb->exc) {  // reported here; the command line itself only fails
        mrb_print_error(mrb);
        mrb->exc = nullptr;
        return 1;
    }
    return 0;
}

// ---- Regexp over the system's regex library ------------------------------------------------------

struct Re {
    pcre2_code* code = nullptr;
    pcre2_match_data* md = nullptr;
};
void re_free(mrb_state* mrb, void* p) {
    auto* r = static_cast<Re*>(p);
    if (!r) return;
    if (r->md) pcre2_match_data_free(r->md);
    if (r->code) pcre2_code_free(r->code);
    mrb_free(mrb, r);
}
const mrb_data_type re_type = {"Regexp", re_free};

// The few places where Ruby's pattern language and the engine's differ.
std::string translate(const std::string& s) {
    std::string o;
    for (size_t i = 0; i < s.size(); ++i) {
        if (s[i] == '\\' && i + 1 < s.size()) {
            char c = s[i + 1];
            if (c == 'h') o += "[0-9a-fA-F]";
            else if (c == 'H') o += "[^0-9a-fA-F]";
            else {
                o += s[i];
                o += c;
            }
            ++i;
            continue;
        }
        o += s[i];
    }
    return o;
}

mrb_value m_re_init(mrb_state* mrb, mrb_value self) {
    mrb_value src, opt = mrb_nil_value();
    mrb_get_args(mrb, "o|o", &src, &opt);
    if (mrb_obj_is_kind_of(mrb, src, mrb_class_get(mrb, "Regexp"))) {
        opt = mrb_iv_get(mrb, src, mrb_intern_lit(mrb, "@options"));
        src = mrb_iv_get(mrb, src, mrb_intern_lit(mrb, "@source"));
    }
    if (!mrb_string_p(src)) mrb_raise(mrb, E_TYPE_ERROR, "a Regexp needs a String");
    mrb_int flags = 0;
    if (mrb_integer_p(opt)) flags = mrb_integer(opt);
    else if (mrb_string_p(opt)) {
        for (char c : cxx(opt)) flags |= c == 'i' ? 1 : c == 'x' ? 2 : c == 'm' ? 4 : 0;
    } else if (mrb_test(opt)) flags = 1;
    mrb_value err = mrb_nil_value();
    Re* re = static_cast<Re*>(mrb_malloc(mrb, sizeof(Re)));
    re->code = nullptr;
    re->md = nullptr;
    if (DATA_PTR(self)) re_free(mrb, DATA_PTR(self));
    DATA_PTR(self) = re;
    DATA_TYPE(self) = &re_type;
    {
        std::string pat = translate(cxx(src));
        uint32_t o = PCRE2_UTF | PCRE2_MULTILINE | PCRE2_MATCH_INVALID_UTF;
        if (flags & 1) o |= PCRE2_CASELESS;
        if (flags & 2) o |= PCRE2_EXTENDED;
        if (flags & 4) o |= PCRE2_DOTALL;
        int ec;
        PCRE2_SIZE eo;
        re->code = pcre2_compile(reinterpret_cast<PCRE2_SPTR>(pat.data()), pat.size(), o, &ec, &eo, nullptr);
        if (!re->code) {
            PCRE2_UCHAR buf[256];
            pcre2_get_error_message(ec, buf, sizeof buf);
            err = str(mrb, std::string(reinterpret_cast<char*>(buf)) + " at " + std::to_string(eo) + ": /" + cxx(src) + "/");
        } else re->md = pcre2_match_data_create_from_pattern(re->code, nullptr);
    }
    if (!mrb_nil_p(err)) raise_with(mrb, mrb_class_get(mrb, "RegexpError"), err);
    mrb_iv_set(mrb, self, mrb_intern_lit(mrb, "@source"), mrb_str_dup(mrb, src));
    mrb_iv_set(mrb, self, mrb_intern_lit(mrb, "@options"), mrb_fixnum_value(flags));
    return self;
}

Re* get_re(mrb_state* mrb, mrb_value self) {
    Re* re = static_cast<Re*>(mrb_data_get_ptr(mrb, self, &re_type));
    if (!re || !re->code) mrb_raise(mrb, E_TYPE_ERROR, "uninitialized Regexp");
    return re;
}

// __match(str, byte_pos) -> [b0, e0, b1, e1, ...] (-1 for groups that took no part) or nil
mrb_value m_re_match(mrb_state* mrb, mrb_value self) {
    mrb_value s;
    mrb_int pos;
    mrb_get_args(mrb, "Si", &s, &pos);
    Re* re = get_re(mrb, self);
    if (pos < 0 || pos > RSTRING_LEN(s)) return mrb_nil_value();
    int rc = pcre2_match(re->code, reinterpret_cast<PCRE2_SPTR>(RSTRING_PTR(s)), size_t(RSTRING_LEN(s)), size_t(pos), 0,
                         re->md, nullptr);
    if (rc < 0) {
        if (rc == PCRE2_ERROR_NOMATCH) return mrb_nil_value();
        PCRE2_UCHAR buf[256];
        pcre2_get_error_message(rc, buf, sizeof buf);
        mrb_raisef(mrb, E_RUNTIME_ERROR, "pattern match failed: %s", reinterpret_cast<char*>(buf));
    }
    uint32_t groups;
    pcre2_pattern_info(re->code, PCRE2_INFO_CAPTURECOUNT, &groups);
    PCRE2_SIZE* ov = pcre2_get_ovector_pointer(re->md);
    mrb_value r = mrb_ary_new_capa(mrb, 2 * (groups + 1));
    for (uint32_t g = 0; g <= groups; ++g) {
        bool set = int(g) < rc && ov[2 * g] != PCRE2_UNSET;
        mrb_ary_push(mrb, r, mrb_fixnum_value(set ? (mrb_int)ov[2 * g] : -1));
        mrb_ary_push(mrb, r, mrb_fixnum_value(set ? (mrb_int)ov[2 * g + 1] : -1));
    }
    return r;
}

mrb_value names_of(mrb_state* mrb, Re* re, const char* want, mrb_int* index) {
    uint32_t count, size;
    PCRE2_SPTR table;
    pcre2_pattern_info(re->code, PCRE2_INFO_NAMECOUNT, &count);
    pcre2_pattern_info(re->code, PCRE2_INFO_NAMEENTRYSIZE, &size);
    pcre2_pattern_info(re->code, PCRE2_INFO_NAMETABLE, &table);
    mrb_value r = mrb_ary_new(mrb);
    for (uint32_t k = 0; k < count; ++k) {
        PCRE2_SPTR e = table + k * size;
        int n = (e[0] << 8) | e[1];
        const char* name = reinterpret_cast<const char*>(e + 2);
        if (want && !std::strcmp(want, name)) *index = n;
        mrb_ary_push(mrb, r, mrb_str_new_cstr(mrb, name));
    }
    return r;
}

mrb_value m_re_names(mrb_state* mrb, mrb_value self) { return names_of(mrb, get_re(mrb, self), nullptr, nullptr); }

mrb_value m_re_name_index(mrb_state* mrb, mrb_value self) {
    const char* n;
    mrb_get_args(mrb, "z", &n);
    mrb_int idx = -1;
    names_of(mrb, get_re(mrb, self), n, &idx);
    return idx < 0 ? mrb_nil_value() : mrb_fixnum_value(idx);
}

mrb_value m_re_source(mrb_state* mrb, mrb_value self) { return mrb_iv_get(mrb, self, mrb_intern_lit(mrb, "@source")); }
mrb_value m_re_options(mrb_state* mrb, mrb_value self) { return mrb_iv_get(mrb, self, mrb_intern_lit(mrb, "@options")); }

// ---- JSON ---------------------------------------------------------------------------------------

struct JsonReader {
    mrb_state* mrb;
    const char* p;
    const char* end;
    bool sym;
    std::string err;
    int depth = 0;

    void ws() {
        while (p < end && (*p == ' ' || *p == '\t' || *p == '\n' || *p == '\r')) ++p;
    }
    bool fail(const char* m) {
        if (err.empty()) err = m;
        return false;
    }
    static void utf8(std::string& o, uint32_t c) {
        if (c < 0x80) o += char(c);
        else if (c < 0x800) o += char(0xC0 | (c >> 6)), o += char(0x80 | (c & 0x3F));
        else if (c < 0x10000) o += char(0xE0 | (c >> 12)), o += char(0x80 | ((c >> 6) & 0x3F)), o += char(0x80 | (c & 0x3F));
        else {
            o += char(0xF0 | (c >> 18));
            o += char(0x80 | ((c >> 12) & 0x3F));
            o += char(0x80 | ((c >> 6) & 0x3F));
            o += char(0x80 | (c & 0x3F));
        }
    }
    bool hex4(uint32_t& v) {
        if (end - p < 4) return false;
        v = 0;
        for (int k = 0; k < 4; ++k) {
            char c = *p++;
            v <<= 4;
            if (c >= '0' && c <= '9') v |= uint32_t(c - '0');
            else if (c >= 'a' && c <= 'f') v |= uint32_t(c - 'a' + 10);
            else if (c >= 'A' && c <= 'F') v |= uint32_t(c - 'A' + 10);
            else return false;
        }
        return true;
    }
    bool string(std::string& o) {
        ++p;
        while (p < end && *p != '"') {
            unsigned char c = static_cast<unsigned char>(*p);
            if (c < 0x20) return fail("control character in a string");
            if (c != '\\') {
                o += char(c);
                ++p;
                continue;
            }
            if (++p >= end) return fail("unfinished escape");
            char e = *p++;
            switch (e) {
                case '"': o += '"'; break;
                case '\\': o += '\\'; break;
                case '/': o += '/'; break;
                case 'b': o += '\b'; break;
                case 'f': o += '\f'; break;
                case 'n': o += '\n'; break;
                case 'r': o += '\r'; break;
                case 't': o += '\t'; break;
                case 'u': {
                    uint32_t v;
                    if (!hex4(v)) return fail("bad \\u escape");
                    if (v >= 0xD800 && v < 0xDC00 && end - p >= 6 && p[0] == '\\' && p[1] == 'u') {
                        p += 2;
                        uint32_t lo;
                        if (!hex4(lo) || lo < 0xDC00 || lo > 0xDFFF) return fail("bad surrogate pair");
                        v = 0x10000 + ((v - 0xD800) << 10) + (lo - 0xDC00);
                    }
                    utf8(o, v);
                    break;
                }
                default: return fail("unknown escape");
            }
        }
        if (p >= end) return fail("unfinished string");
        ++p;
        return true;
    }
    bool value(mrb_value& out) {
        ws();
        if (p >= end) return fail("unexpected end of text");
        if (++depth > 512) return fail("nested too deep");
        bool ok = true;
        char c = *p;
        if (c == '{') {
            ++p;
            out = mrb_hash_new(mrb);
            ws();
            if (p < end && *p == '}') ++p;
            else
                for (;;) {
                    ws();
                    if (p >= end || *p != '"') {
                        ok = fail("expected a key");
                        break;
                    }
                    std::string k;
                    if (!string(k)) {
                        ok = false;
                        break;
                    }
                    ws();
                    if (p >= end || *p != ':') {
                        ok = fail("expected :");
                        break;
                    }
                    ++p;
                    mrb_value v;
                    if (!value(v)) {
                        ok = false;
                        break;
                    }
                    mrb_value key = sym ? mrb_symbol_value(mrb_intern(mrb, k.data(), k.size())) : str(mrb, k);
                    mrb_hash_set(mrb, out, key, v);
                    ws();
                    if (p < end && *p == ',') {
                        ++p;
                        continue;
                    }
                    if (p < end && *p == '}') {
                        ++p;
                        break;
                    }
                    ok = fail("expected , or }");
                    break;
                }
        } else if (c == '[') {
            ++p;
            out = mrb_ary_new(mrb);
            ws();
            if (p < end && *p == ']') ++p;
            else
                for (;;) {
                    mrb_value v;
                    if (!value(v)) {
                        ok = false;
                        break;
                    }
                    mrb_ary_push(mrb, out, v);
                    ws();
                    if (p < end && *p == ',') {
                        ++p;
                        continue;
                    }
                    if (p < end && *p == ']') {
                        ++p;
                        break;
                    }
                    ok = fail("expected , or ]");
                    break;
                }
        } else if (c == '"') {
            std::string s;
            ok = string(s);
            if (ok) out = str(mrb, s);
        } else if (end - p >= 4 && !std::strncmp(p, "true", 4)) out = mrb_true_value(), p += 4;
        else if (end - p >= 5 && !std::strncmp(p, "false", 5)) out = mrb_false_value(), p += 5;
        else if (end - p >= 4 && !std::strncmp(p, "null", 4)) out = mrb_nil_value(), p += 4;
        else if (c == '-' || (c >= '0' && c <= '9')) {
            const char* b = p;
            bool real = false;
            if (*p == '-') ++p;
            while (p < end && ((*p >= '0' && *p <= '9') || *p == '.' || *p == 'e' || *p == 'E' || *p == '+' || *p == '-')) {
                if (*p == '.' || *p == 'e' || *p == 'E') real = true;
                ++p;
            }
            std::string num(b, size_t(p - b));
            if (real) out = mrb_float_value(mrb, std::strtod(num.c_str(), nullptr));
            else {
                errno = 0;
                long long v = std::strtoll(num.c_str(), nullptr, 10);
                out = errno ? mrb_float_value(mrb, std::strtod(num.c_str(), nullptr)) : mrb_int_value(mrb, (mrb_int)v);
            }
        } else ok = fail("unexpected character");
        --depth;
        return ok;
    }
};

mrb_value m_json_parse(mrb_state* mrb, mrb_value) {
    mrb_value s;
    mrb_bool sym;
    mrb_get_args(mrb, "Sb", &s, &sym);
    mrb_value out = mrb_nil_value(), err = mrb_nil_value();
    {
        JsonReader r{mrb, RSTRING_PTR(s), RSTRING_PTR(s) + RSTRING_LEN(s), bool(sym), {}};
        if (r.value(out)) {
            r.ws();
            if (r.p != r.end) r.fail("text after the value");
        }
        if (!r.err.empty())
            err = str(mrb, r.err + " at byte " + std::to_string(r.p - RSTRING_PTR(s)));
    }
    if (!mrb_nil_p(err)) {
        RClass* json = mrb_module_get(mrb, "JSON");
        raise_with(mrb, mrb_class_get_under(mrb, json, "ParserError"), err);
    }
    return out;
}

mrb_value m_json_str(mrb_state* mrb, mrb_value) {
    mrb_value s;
    mrb_get_args(mrb, "S", &s);
    mrb_value r;
    {
        std::string o = "\"";
        const char* p = RSTRING_PTR(s);
        for (mrb_int k = 0; k < RSTRING_LEN(s); ++k) {
            unsigned char c = static_cast<unsigned char>(p[k]);
            switch (c) {
                case '"': o += "\\\""; break;
                case '\\': o += "\\\\"; break;
                case '\n': o += "\\n"; break;
                case '\r': o += "\\r"; break;
                case '\t': o += "\\t"; break;
                case '\b': o += "\\b"; break;
                case '\f': o += "\\f"; break;
                default:
                    if (c < 0x20) {
                        char buf[8];
                        std::snprintf(buf, sizeof buf, "\\u%04x", c);
                        o += buf;
                    } else o += char(c);
            }
        }
        o += '"';
        r = str(mrb, o);
    }
    return r;
}

// ---- errors -------------------------------------------------------------------------------------

// "file:line" of the first place in the person's code (the built-in part has no file name).
std::string where(mrb_state* mrb, mrb_value exc) {
    mrb_value bt = mrb_funcall(mrb, exc, "backtrace", 0);
    if (mrb->exc) mrb->exc = nullptr;
    if (!mrb_array_p(bt)) return "";
    for (mrb_int k = 0; k < RARRAY_LEN(bt); ++k) {
        mrb_value e = mrb_ary_ref(mrb, bt, k);
        if (!mrb_string_p(e)) continue;
        std::string s = cxx(e);
        if (s.empty() || s[0] == '(' || s.compare(0, 2, "-:") == 0) continue;
        size_t in = s.find(":in ");
        if (in != std::string::npos) s = s.substr(0, in);
        return s;
    }
    return "";
}

// Prints an exception the script did not handle; returns the exit status for it.
int report(mrb_state* mrb, mrb_value exc) {
    RObject* o = mrb_obj_ptr(exc);
    if (MRB_EXC_EXIT_P(o)) {
        mrb_value st = mrb_iv_get(mrb, exc, mrb_intern_lit(mrb, "status"));
        return mrb_integer_p(st) ? int(mrb_integer(st)) : 0;
    }
    if (mrb_obj_is_kind_of(mrb, exc, mrb_class_get(mrb, "Interrupt"))) return 130;
    mrb_value msg = mrb_funcall(mrb, exc, "message", 0);
    if (mrb->exc) mrb->exc = nullptr;
    std::string m = mrb_string_p(msg) ? cxx(msg) : "error";
    std::string at = where(mrb, exc);
    RClass* failed = mrb_class_get_under(mrb, rsh_mod(mrb), "Failed");
    if (mrb_obj_is_kind_of(mrb, exc, failed)) {
        mrb_value st = mrb_funcall(mrb, exc, "status", 0);
        mrb_value code = mrb_funcall(mrb, st, "exitstatus", 0);
        std::fprintf(stderr, "rsh: %s%s%s\n", at.c_str(), at.empty() ? "" : ": ", m.c_str());
        int c = mrb_integer_p(code) ? int(mrb_integer(code)) : 1;
        return c ? c : 1;
    }
    const char* cls = mrb_obj_classname(mrb, exc);
    if (m.compare(0, 5, "line ") == 0 && !std::strcmp(cls, "SyntaxError")) at.clear();
    std::fprintf(stderr, "rsh: %s%s%s (%s)\n", at.c_str(), at.empty() ? "" : ": ", m.c_str(), cls);
    return 1;
}

mrb_ccontext* repl_cxt = nullptr;

}  // namespace

mrb_state* rt_open(const std::string& script, const std::vector<std::string>& args) {
    mrb_state* mrb = mrb_open();
    if (!mrb) return nullptr;
    host.mrb = mrb;
    host.argv = args;
    RClass* r = mrb_define_module(mrb, "Rsh");
    mrb_define_module_function(mrb, r, "__exec", m_exec, MRB_ARGS_REQ(3));
    mrb_define_module_function(mrb, r, "__argv", m_argv, MRB_ARGS_REQ(2));
    mrb_define_module_function(mrb, r, "__set_status", m_set_status, MRB_ARGS_REQ(1));
    mrb_define_module_function(mrb, r, "__set_match", m_set_match, MRB_ARGS_REQ(2));
    mrb_define_module_function(mrb, r, "__chdir", m_chdir, MRB_ARGS_REQ(1));
    mrb_define_module_function(mrb, r, "__nullglob", m_nullglob, MRB_ARGS_REQ(1));
    mrb_define_module_function(mrb, r, "__getenv", m_getenv, MRB_ARGS_REQ(1));
    mrb_define_module_function(mrb, r, "__setenv", m_setenv, MRB_ARGS_REQ(2));
    mrb_define_module_function(mrb, r, "__env_hash", m_env_hash, MRB_ARGS_NONE());
    mrb_define_module_function(mrb, r, "__pid", m_pid, MRB_ARGS_NONE());
    mrb_define_module_function(mrb, r, "__nproc", m_nproc, MRB_ARGS_NONE());
    mrb_define_module_function(mrb, r, "__kill", m_kill, MRB_ARGS_REQ(2));
    mrb_define_module_function(mrb, r, "__glob", m_glob, MRB_ARGS_REQ(1));
    mrb_define_module_function(mrb, r, "__require", m_require, MRB_ARGS_REQ(2));
    mrb_define_module_function(mrb, r, "__json_parse", m_json_parse, MRB_ARGS_REQ(2));
    mrb_define_module_function(mrb, r, "__json_str", m_json_str, MRB_ARGS_REQ(1));
    RClass* re = mrb_define_class(mrb, "Regexp", mrb->object_class);
    MRB_SET_INSTANCE_TT(re, MRB_TT_CDATA);
    mrb_define_method(mrb, re, "initialize", m_re_init, MRB_ARGS_ARG(1, 1));
    mrb_define_method(mrb, re, "__match", m_re_match, MRB_ARGS_REQ(2));
    mrb_define_method(mrb, re, "names", m_re_names, MRB_ARGS_NONE());
    mrb_define_method(mrb, re, "__name_index", m_re_name_index, MRB_ARGS_REQ(1));
    mrb_define_method(mrb, re, "source", m_re_source, MRB_ARGS_NONE());
    mrb_define_method(mrb, re, "options", m_re_options, MRB_ARGS_NONE());
    mrb_load_irep(mrb, rsh_prelude);
    if (mrb->exc) {
        mrb_print_error(mrb);
        return nullptr;
    }
    mrb_value av = mrb_ary_new(mrb);
    for (auto& a : args) mrb_ary_push(mrb, av, str(mrb, a));
    mrb_define_global_const(mrb, "ARGV", av);
    mrb_gv_set(mrb, mrb_intern_lit(mrb, "$0"), str(mrb, script));
    mrb_gv_set(mrb, mrb_intern_lit(mrb, "$*"), av);
    return mrb;
}

void rt_close(mrb_state* mrb) {
    if (repl_cxt) mrb_ccontext_free(mrb, repl_cxt);
    repl_cxt = nullptr;
    mrb_close(mrb);
}

int rt_run(mrb_state* mrb, const std::string& src, const std::string& file, bool rsh_syntax) {
    bool syn;
    load_text(mrb, src, file, rsh_syntax, syn);
    if (syn) return 2;
    if (mrb->exc) {
        mrb_value e = mrb_obj_value(mrb->exc);
        mrb->exc = nullptr;
        if (!std::strcmp(mrb_obj_classname(mrb, e), "SyntaxError")) {
            // the compiler says "line N: ...": put the file in front, like every other message
            mrb_value msg = mrb_funcall(mrb, e, "message", 0);
            std::string m = mrb_string_p(msg) ? cxx(msg) : "syntax error";
            if (m.compare(0, 5, "line ") == 0) m = m.substr(5);
            std::fprintf(stderr, "rsh: %s:%s\n", file.c_str(), m.c_str());
            return 2;
        }
        return report(mrb, e);
    }
    return 0;
}

int rt_eval(mrb_state* mrb, const std::string& src, Known& known, bool& exited, bool& incomplete) {
    exited = false;
    incomplete = false;
    mrb_value code = mrb_nil_value();
    bool commands = false;
    {
        Known trial = known;
        size_t before = table().size();
        Result r = transpile(src, "(rsh)", trial);
        if (!r.ok()) {
            bool open = false;
            for (auto& d : r.diags)
                if (d.msg.find("not closed") != std::string::npos || d.msg.find("without its end word") != std::string::npos ||
                    d.msg.find("without a closing") != std::string::npos)
                    open = true;
            table().resize(before);
            if (open) {
                incomplete = true;
                return 0;
            }
            for (auto& d : r.diags)
                if (!d.warning) std::fprintf(stderr, "rsh: %s\n", d.msg.c_str());
            return 2;
        }
        // Does the Ruby parse? An unfinished block or string asks for another line.
        mrb_ccontext* c = mrb_ccontext_new(mrb);
        c->capture_errors = TRUE;
        c->no_exec = TRUE;
        mrb_parser_state* p = mrb_parse_nstring(mrb, r.ruby.data(), r.ruby.size(), c);
        if (p && p->nerr) {
            std::string m = p->error_buffer[0].message ? p->error_buffer[0].message : "";
            if (m.find("end of file") != std::string::npos || m.find("$end") != std::string::npos ||
                m.find("unterminated") != std::string::npos)
                incomplete = true;
        }
        if (p) mrb_parser_free(p);
        mrb_ccontext_free(mrb, c);
        if (incomplete) {
            table().resize(before);
            return 0;
        }
        known = trial;
        commands = !r.marks.empty();
        code = str(mrb, r.ruby);
    }
    if (!repl_cxt) {
        repl_cxt = mrb_ccontext_new(mrb);
        mrb_ccontext_filename(mrb, repl_cxt, "(rsh)");
        repl_cxt->capture_errors = TRUE;
    }
    int ai = mrb_gc_arena_save(mrb);
    mrb_value v = mrb_load_nstring_cxt(mrb, RSTRING_PTR(code), RSTRING_LEN(code), repl_cxt);
    int status = 0;
    if (mrb->exc) {
        mrb_value e = mrb_obj_value(mrb->exc);
        mrb->exc = nullptr;
        if (MRB_EXC_EXIT_P(mrb_obj_ptr(e))) exited = true;
        status = report(mrb, e);
    } else if (!commands && !mrb_nil_p(v) && !mrb_undef_p(v)) {
        mrb_value s = mrb_inspect(mrb, v);
        if (mrb->exc) mrb->exc = nullptr;
        else if (mrb_string_p(s)) std::printf("=> %s\n", cxx(s).c_str());
        std::fflush(stdout);
    } else if (commands) status = shell().last_status;
    mrb_gc_arena_restore(mrb, ai);
    return status;
}

int rt_check(mrb_state* mrb, const std::string& src, const std::string& file, bool lint) {
    int bad = 0;
    size_t before = table().size();
    Known k;
    collect_defs(src, dir_of(file), k.methods);
    Result r = transpile(src, file, k);
    for (auto& d : r.diags) {
        std::fprintf(stderr, "%s:%d: %s%s\n", file.c_str(), d.line, d.warning ? "warning: " : "", d.msg.c_str());
        if (!d.warning) bad = 2;
    }
    if (bad) return bad;
    mrb_ccontext* c = mrb_ccontext_new(mrb);
    mrb_ccontext_filename(mrb, c, file.c_str());
    c->capture_errors = TRUE;
    c->no_exec = TRUE;
    mrb_parser_state* p = mrb_parse_nstring(mrb, r.ruby.data(), r.ruby.size(), c);
    if (!p) bad = 2;
    else {
        for (size_t e = 0; e < p->nerr && e < 10; ++e) {
            std::fprintf(stderr, "%s:%d:%d: %s\n", file.c_str(), p->error_buffer[e].lineno, p->error_buffer[e].column,
                         p->error_buffer[e].message ? p->error_buffer[e].message : "syntax error");
            bad = 2;
        }
        mrb_parser_free(p);
    }
    mrb_ccontext_free(mrb, c);
    if (bad || !lint) return bad;
    // Warnings: words taken as commands that are not on PATH, and risky forms.
    int warn = 0;
    for (size_t li = before; li < table().size(); ++li) {
        const Line& L = table()[li];
        for (auto& it : L.items)
            for (auto& cmd : it.pipeline) {
                if (cmd.words.empty()) continue;
                const Word& w0 = cmd.words[0];
                std::string name;
                bool plain = true;
                for (auto& part : w0.parts) {
                    if (part.k != Part::Pat && part.k != Part::Lit) plain = false;
                    else name += part.s;
                }
                if (plain && !name.empty() && !is_builtin(name) && find_program(name).empty() && name[0] != '.' &&
                    name[0] != '/' && name[0] != '~') {
                    std::fprintf(stderr, "%s:%d: warning: `%s` is taken as a command and is not on PATH; if it is a Ruby method, write %s(...)\n",
                                 file.c_str(), L.line, name.c_str(), name.c_str());
                    ++warn;
                }
                bool shell_c = (name == "sh" || name == "bash" || name == "zsh") && cmd.words.size() > 1;
                for (size_t k2 = 1; k2 < cmd.words.size(); ++k2) {
                    bool has_val = false;
                    std::string lit;
                    for (auto& part : cmd.words[k2].parts) {
                        if (part.k == Part::Val || part.k == Part::Env) has_val = true;
                        else lit += part.s;
                    }
                    if (shell_c && has_val) {
                        std::fprintf(stderr, "%s:%d: warning: a Ruby value inside %s -c text is parsed again by that shell; pass it as an argument\n",
                                     file.c_str(), L.line, name.c_str());
                        ++warn;
                    }
                    if (name == "chmod" && (lit == "777" || lit == "666" || lit.find("o+w") != std::string::npos ||
                                            lit.find("a+w") != std::string::npos)) {
                        std::fprintf(stderr, "%s:%d: warning: chmod %s lets everyone write\n", file.c_str(), L.line, lit.c_str());
                        ++warn;
                    }
                    if (lit.compare(0, 5, "/tmp/") == 0 && lit.size() > 5) {
                        std::fprintf(stderr, "%s:%d: warning: a fixed name under /tmp; use mktemp\n", file.c_str(), L.line);
                        ++warn;
                    }
                }
            }
    }
    return warn ? 1 : 0;
}

bool rt_prompt(mrb_state* mrb, std::string& out) {
    mrb_value self = mrb_top_self(mrb);
    if (!mrb_respond_to(mrb, self, mrb_intern_lit(mrb, "prompt"))) return false;
    mrb_value v = mrb_funcall(mrb, self, "prompt", 0);
    if (mrb->exc) {
        mrb->exc = nullptr;
        return false;
    }
    if (!mrb_string_p(v)) return false;
    out = cxx(v);
    return true;
}

void rt_add_history(const std::string& line) { host.hist.push_back(line); }

}  // namespace rsh

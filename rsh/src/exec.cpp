// SPDX-License-Identifier: GPL-2.0-or-later
#include "exec.hpp"

#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <spawn.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <termios.h>
#include <unistd.h>

#include <algorithm>
#include <cerrno>
#include <cstdio>
#include <cstdlib>
#include <cstring>

#include "glob.hpp"

extern char** environ;

namespace rsh {

Shell& shell() {
    static Shell s;
    return s;
}

namespace {

constexpr size_t TAIL = 4096;

struct Job {
    int id;
    pid_t pgid;
    std::vector<pid_t> pids;
    std::vector<int> status;
    std::string text;
    bool stopped = false;
    bool done = false;
};
std::vector<Job> jobs;

void say(int fd, const std::string& s) {
    size_t o = 0;
    while (o < s.size()) {
        ssize_t w = write(fd, s.data() + o, s.size() - o);
        if (w < 0 && errno == EINTR) continue;
        if (w <= 0) return;
        o += size_t(w);
    }
}
void complain(const std::string& s) { say(2, "rsh: " + s + "\n"); }

int decode(int st, int* sig) {
    if (WIFEXITED(st)) return WEXITSTATUS(st);
    if (WIFSIGNALED(st)) {
        if (sig) *sig = WTERMSIG(st);
        return 128 + WTERMSIG(st);
    }
    if (WIFSTOPPED(st)) return 128 + WSTOPSIG(st);
    return 1;
}

bool valid_name(const std::string& n) {
    if (n.empty() || !(std::isalpha((unsigned char)n[0]) || n[0] == '_')) return false;
    for (char c : n)
        if (!(std::isalnum((unsigned char)c) || c == '_')) return false;
    return true;
}

}  // namespace

std::string home_of(const std::string& user) {
    if (user.empty()) {
        const char* h = getenv("HOME");
        return h ? h : "/";
    }
    FILE* f = std::fopen("/etc/passwd", "r");
    if (!f) return "";
    char buf[1024];
    std::string home;
    while (std::fgets(buf, sizeof buf, f)) {
        std::string ln(buf);
        size_t c = ln.find(':');
        if (c == std::string::npos || ln.compare(0, c, user) != 0 || c != user.size()) continue;
        size_t p = c;
        for (int k = 0; k < 4 && p != std::string::npos; ++k) p = ln.find(':', p + 1);
        if (p == std::string::npos) break;
        size_t e = ln.find(':', p + 1);
        home = ln.substr(p + 1, e == std::string::npos ? std::string::npos : e - p - 1);
        break;
    }
    std::fclose(f);
    return home;
}

std::string find_program(const std::string& name) {
    if (name.find('/') != std::string::npos) return access(name.c_str(), X_OK) == 0 ? name : "";
    const char* p = getenv("PATH");
    std::string path = p ? p : "/usr/local/bin:/usr/bin:/bin";
    size_t i = 0;
    while (i <= path.size()) {
        size_t e = path.find(':', i);
        if (e == std::string::npos) e = path.size();
        std::string dir = e > i ? path.substr(i, e - i) : ".";
        std::string full = dir + "/" + name;
        struct stat st;
        if (stat(full.c_str(), &st) == 0 && S_ISREG(st.st_mode) && access(full.c_str(), X_OK) == 0) return full;
        i = e + 1;
    }
    return "";
}

static const char* const BUILTINS[] = {"cd",  "pushd", "popd",  "dirs", "export", "unset", "exec",    "umask",
                                       "jobs", "fg",   "bg",    "wait", "source", "echo",  "type",    "pwd",
                                       "history", nullptr};
static const char* const IN_PROCESS[] = {"cd",  "pushd", "popd",   "export", "unset",   "exec",
                                         "umask", "jobs", "fg", "bg", "wait", "source", "history", nullptr};

bool is_builtin(const std::string& n) {
    for (auto p = BUILTINS; *p; ++p)
        if (n == *p) return true;
    return false;
}
static bool in_process(const std::string& n) {
    for (auto p = IN_PROCESS; *p; ++p)
        if (n == *p) return true;
    return false;
}

bool expand(const Word& w, const std::vector<Value>& vals, Host& h, std::vector<std::string>& out, std::string& err,
            bool glob_ok) {
    if (w.parts.size() == 1 && w.parts[0].k == Part::Val && vals[w.parts[0].idx].array) {
        for (auto& s : vals[w.parts[0].idx].items) out.push_back(s);
        return true;
    }
    if (w.parts.size() == 1 && w.parts[0].k == Part::Args) {
        for (auto& s : h.args()) out.push_back(s);
        return true;
    }
    std::string pat;
    bool meta = false, brace = false, has_val = false;
    for (size_t k = 0; k < w.parts.size(); ++k) {
        const Part& p = w.parts[k];
        switch (p.k) {
            case Part::Lit: pat += glob_escape(p.s); break;
            case Part::Pat: {
                std::string s = p.s;
                if (k == 0 && !s.empty() && s[0] == '~') {
                    size_t e = s.find('/');
                    std::string user = s.substr(1, e == std::string::npos ? std::string::npos : e - 1);
                    bool whole = e == std::string::npos && w.parts.size() == 1;
                    if (e != std::string::npos || whole || user.empty()) {
                        std::string home = home_of(user);
                        if (home.empty()) {
                            err = "no such user: " + user;
                            return false;
                        }
                        pat += glob_escape(home);
                        s = e == std::string::npos ? "" : s.substr(e);
                    }
                }
                pat += s;
                if (s.find_first_of("*?[") != std::string::npos) meta = true;
                if (s.find('{') != std::string::npos) brace = true;
                break;
            }
            case Part::Val: {
                const Value& v = vals[p.idx];
                if (v.array) {
                    err = "an Array value must be a whole word (\"-x=#{list}\" does not split)";
                    return false;
                }
                has_val = true;
                pat += glob_escape(v.items.empty() ? "" : v.items[0]);
                break;
            }
            case Part::Env: {
                const char* e = getenv(p.s.c_str());
                if (!e) {
                    err = "$" + p.s + " is not set (ENV.fetch(\"" + p.s + "\", default) gives a default)";
                    return false;
                }
                pat += glob_escape(e);
                break;
            }
            case Part::Status: pat += std::to_string(shell().last_status); break;
            case Part::BgPid: pat += std::to_string(shell().last_bg); break;
            case Part::Args: {
                err = "$* must be a whole word";
                return false;
            }
        }
    }
    std::vector<std::string> alts = (brace && !has_val) ? brace_expand(pat) : std::vector<std::string>{pat};
    for (auto& a : alts) {
        if (glob_ok && meta && glob_has_meta(a)) {
            auto m = glob(a);
            if (m.empty() && !shell().nullglob) {
                err = "no file matches " + glob_unescape(a);
                return false;
            }
            for (auto& x : m) out.push_back(x);
        } else out.push_back(glob_unescape(a));
    }
    return true;
}

namespace {

// One command of a pipeline after expansion.
struct Stage {
    std::vector<std::string> argv;
    std::vector<std::pair<std::string, std::string>> env;
    struct R {
        Redir::Kind k;
        int fd, dupfd;
        std::string target;
    };
    std::vector<R> redirs;
    int block = -1;
    bool pipe_err = false;
    pid_t pid = -1;
    int status = 0;
    int sig = 0;
    bool done = false;
    int errfd = -1;
    std::string tail;
};

bool expand_cmd(const Cmd& c, const std::vector<Value>& vals, Host& h, Stage& st, std::string& err) {
    st.block = c.block;
    st.pipe_err = c.pipe_err;
    for (auto& w : c.words)
        if (!expand(w, vals, h, st.argv, err)) return false;
    if (c.block < 0 && st.argv.empty()) {
        err = "the command word is empty";
        return false;
    }
    for (auto& [name, w] : c.env) {
        std::vector<std::string> v;
        if (!expand(w, vals, h, v, err, false)) return false;
        std::string j;
        for (size_t k = 0; k < v.size(); ++k) j += (k ? " " : "") + v[k];
        st.env.emplace_back(name, j);
    }
    for (auto& r : c.redirs) {
        Stage::R x{r.k, r.fd, r.dupfd, ""};
        if (r.k == Redir::Here) {
            for (auto& p : r.target.parts) {
                if (p.k == Part::Lit || p.k == Part::Pat) x.target += p.s;
                else if (p.k == Part::Val) {
                    auto& v = vals[p.idx].items;
                    for (size_t k = 0; k < v.size(); ++k) x.target += (k ? " " : "") + v[k];
                } else if (p.k == Part::Env) {
                    const char* e = getenv(p.s.c_str());
                    if (!e) {
                        err = "$" + p.s + " is not set";
                        return false;
                    }
                    x.target += e;
                }
            }
        } else if (r.k != Redir::Dup) {
            std::vector<std::string> v;
            if (!expand(r.target, vals, h, v, err)) return false;
            if (v.size() != 1) {
                err = "a redirect needs exactly one file name";
                return false;
            }
            x.target = v[0];
        }
        st.redirs.push_back(x);
    }
    return true;
}

// Opens the files of a stage's redirects: (fd in the child, fd here) pairs, in order.
bool open_redirs(const Stage& st, std::vector<std::pair<int, int>>& map, std::vector<int>& opened, std::string& err) {
    for (auto& r : st.redirs) {
        int f = -1;
        switch (r.k) {
            case Redir::In: f = open(r.target.c_str(), O_RDONLY | O_CLOEXEC); break;
            case Redir::Out:
            case Redir::Both: f = open(r.target.c_str(), O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC, 0666); break;
            case Redir::Append:
            case Redir::BothAppend: f = open(r.target.c_str(), O_WRONLY | O_CREAT | O_APPEND | O_CLOEXEC, 0666); break;
            case Redir::Here: {
                f = memfd_create("rsh-here", MFD_CLOEXEC);
                if (f >= 0) {
                    say(f, r.target);
                    lseek(f, 0, SEEK_SET);
                }
                break;
            }
            case Redir::Dup: map.emplace_back(r.fd, -(r.dupfd + 1)); continue;
        }
        if (f < 0) {
            err = r.target + ": " + std::strerror(errno);
            return false;
        }
        opened.push_back(f);
        if (r.k == Redir::Both || r.k == Redir::BothAppend) {
            map.emplace_back(1, f);
            map.emplace_back(2, f);
        } else map.emplace_back(r.fd, f);
    }
    return true;
}

std::vector<std::string> env_with(const std::vector<std::pair<std::string, std::string>>& extra) {
    std::vector<std::string> e;
    for (char** p = environ; *p; ++p) {
        std::string kv(*p);
        std::string k = kv.substr(0, kv.find('='));
        bool over = false;
        for (auto& x : extra) over |= x.first == k;
        if (!over) e.push_back(kv);
    }
    for (auto& x : extra) e.push_back(x.first + "=" + x.second);
    return e;
}

int run_builtin(const std::vector<std::string>& a, Host& h);

// Moves a set of child fds into place in this process (builtins); the old ones are saved.
struct FdSave {
    std::vector<std::pair<int, int>> saved;
    void set(int target, int src) {
        int keep = fcntl(target, F_DUPFD_CLOEXEC, 10);
        saved.emplace_back(target, keep);
        dup2(src, target);
    }
    void restore() {
        for (auto it = saved.rbegin(); it != saved.rend(); ++it) {
            if (it->second >= 0) {
                dup2(it->second, it->first);
                close(it->second);
            } else close(it->first);
        }
        saved.clear();
    }
};

void apply_map(const std::vector<std::pair<int, int>>& map, FdSave* save) {
    for (auto [target, src] : map) {
        int from = src < 0 ? -src - 1 : src;
        if (save) save->set(target, from);
        else dup2(from, target);
    }
}

void child_signals_default() {
    for (int s : {SIGINT, SIGQUIT, SIGTSTP, SIGTTIN, SIGTTOU, SIGPIPE, SIGCHLD, SIGTERM, SIGHUP}) signal(s, SIG_DFL);
}

// Starts one stage. in/out: -1 = this process's own.
bool spawn(Stage& st, int in, int out, int err_to, pid_t pgid, Host& h, std::string& error) {
    std::vector<std::pair<int, int>> map;
    std::vector<int> opened;
    if (!open_redirs(st, map, opened, error)) return false;
    auto done = [&] {
        for (int f : opened) close(f);
    };
    if (is_builtin(st.argv[0])) {  // a builtin in a pipeline or in the background: its own process
        pid_t pid = fork();
        if (pid == 0) {
            child_signals_default();
            if (shell().job_control) setpgid(0, pgid);
            if (in >= 0) dup2(in, 0);
            if (out >= 0) dup2(out, 1);
            if (err_to >= 0) dup2(err_to, 2);
            apply_map(map, nullptr);
            _exit(run_builtin(st.argv, h));
        }
        done();
        st.pid = pid;
        return pid > 0;
    }
    std::string prog = find_program(st.argv[0]);
    if (prog.empty()) {
        std::string m = "rsh: " + st.argv[0] + ": command not found\n";
        say(err_to >= 0 ? err_to : 2, m);
        st.tail = m;
        st.status = 127;
        st.done = true;
        done();
        return true;
    }
    posix_spawn_file_actions_t fa;
    posix_spawn_file_actions_init(&fa);
    if (in >= 0) posix_spawn_file_actions_adddup2(&fa, in, 0);
    if (out >= 0) posix_spawn_file_actions_adddup2(&fa, out, 1);
    if (err_to >= 0) posix_spawn_file_actions_adddup2(&fa, err_to, 2);
    for (auto [target, src] : map) posix_spawn_file_actions_adddup2(&fa, src < 0 ? -src - 1 : src, target);
    posix_spawnattr_t at;
    posix_spawnattr_init(&at);
    sigset_t def, none;
    sigemptyset(&def);
    for (int s : {SIGINT, SIGQUIT, SIGTSTP, SIGTTIN, SIGTTOU, SIGPIPE, SIGCHLD, SIGTERM, SIGHUP}) sigaddset(&def, s);
    sigemptyset(&none);
    posix_spawnattr_setsigdefault(&at, &def);
    posix_spawnattr_setsigmask(&at, &none);
    short flags = POSIX_SPAWN_SETSIGDEF | POSIX_SPAWN_SETSIGMASK;
    if (shell().job_control) {
        flags |= POSIX_SPAWN_SETPGROUP;
        posix_spawnattr_setpgroup(&at, pgid);
    }
    posix_spawnattr_setflags(&at, flags);
    std::vector<std::string> envs = env_with(st.env);
    std::vector<char*> envp, argv;
    for (auto& e : envs) envp.push_back(e.data());
    envp.push_back(nullptr);
    for (auto& a : st.argv) argv.push_back(a.data());
    argv.push_back(nullptr);
    pid_t pid;
    int rc = posix_spawn(&pid, prog.c_str(), &fa, &at, argv.data(), envp.data());
    posix_spawn_file_actions_destroy(&fa);
    posix_spawnattr_destroy(&at);
    done();
    if (rc != 0) {
        std::string m = "rsh: " + st.argv[0] + ": " + std::strerror(rc) + "\n";
        say(err_to >= 0 ? err_to : 2, m);
        st.tail = m;
        st.status = rc == ENOENT ? 127 : 126;
        st.done = true;
        return true;
    }
    if (shell().job_control) setpgid(pid, pgid ? pgid : pid);
    st.pid = pid;
    return true;
}

void keep_tail(std::string& tail, const char* buf, size_t n) {
    tail.append(buf, n);
    if (tail.size() > TAIL) tail.erase(0, tail.size() - TAIL);
}

int wait_pid(pid_t pid, int* sig, bool* stopped) {
    int st = 0;
    for (;;) {
        pid_t r = waitpid(pid, &st, shell().job_control ? WUNTRACED : 0);
        if (r < 0 && errno == EINTR) continue;
        if (r < 0) return 1;
        break;
    }
    if (WIFSTOPPED(st) && stopped) *stopped = true;
    return decode(st, sig);
}

void give_terminal(pid_t pgid) {
    if (shell().job_control && shell().tty >= 0) tcsetpgrp(shell().tty, pgid);
}

struct Pipe {
    int r = -1, w = -1;
    bool make() {
        int f[2];
        if (pipe2(f, O_CLOEXEC) != 0) return false;
        r = f[0];
        w = f[1];
        return true;
    }
};

void close_fd(int& f) {
    if (f >= 0) close(f);
    f = -1;
}

struct PipeResult {
    int code = 0;
    std::vector<int> statuses;
    int failed = -1;
    int sig = 0;
    bool stopped = false;
    bool interrupted = false;
};

// Runs one pipeline in the foreground (or starts it in the background).
bool pipeline(const Item& it, const std::vector<Value>& vals, Host& h, const Options& o, bool bg, const std::string& text,
              Outcome& oc, PipeResult& pr, std::vector<Stage>& stages) {
    for (auto& c : it.pipeline) {
        Stage st;
        if (!expand_cmd(c, vals, h, st, oc.error)) return false;
        stages.push_back(std::move(st));
    }
    size_t m = stages.size();
    int r0 = -1, r1 = -1;
    for (size_t k = 0; k < m; ++k)
        if (stages[k].block >= 0) {
            if (r0 < 0) r0 = int(k);
            else if (r1 != int(k) - 1) {
                oc.error = "Ruby stages of one pipeline must be next to each other";
                return false;
            }
            r1 = int(k);
        }
    if (r0 == 0) {
        oc.error = "a Ruby stage needs a command before it";
        return false;
    }
    // One builtin that changes this shell (cd, export...): run here.
    if (m == 1 && !bg && in_process(stages[0].argv[0])) {
        std::vector<std::pair<int, int>> map;
        std::vector<int> opened;
        if (!open_redirs(stages[0], map, opened, oc.error)) return false;
        FdSave save;
        Pipe cap;
        if (o.capture && cap.make()) save.set(1, cap.w);
        apply_map(map, &save);
        int code = run_builtin(stages[0].argv, h);
        if (stages[0].argv.size() == 1 && stages[0].argv[0] == "exec" && !o.capture) {
            for (auto& sv : save.saved)  // exec with only redirects: they stay
                if (sv.second >= 0) close(sv.second);
            save.saved.clear();
        } else save.restore();
        for (int f : opened) close(f);
        if (cap.w >= 0) {
            close_fd(cap.w);
            char buf[4096];
            ssize_t n;
            while ((n = read(cap.r, buf, sizeof buf)) > 0) oc.out.append(buf, size_t(n));
            close_fd(cap.r);
        }
        pr.code = code;
        pr.statuses = {code};
        if (code) pr.failed = 0;
        return true;
    }
    bool tee = o.tee && !o.cond && !o.relaxed && !shell().interactive && !bg && r0 < 0;
    Pipe cap;
    if (o.capture && !bg && !cap.make()) {
        oc.error = "no pipe";
        return false;
    }
    int final_out = o.capture && !bg ? cap.w : -1;
    int in = -1;
    if (bg && !shell().job_control) in = open("/dev/null", O_RDONLY | O_CLOEXEC);
    pid_t pgid = 0;
    int stage_in = -1, stage_out = -1;  // the Ruby stages' pipes (read here, write here)
    bool stage_out_mine = false;
    for (size_t k = 0; k < m; ++k) {
        if (int(k) == r0) {
            stage_in = in;
            in = -1;
            if (size_t(r1) == m - 1) stage_out = final_out >= 0 ? final_out : 1;
            else {
                Pipe p;
                p.make();
                stage_out = p.w;
                stage_out_mine = true;
                in = p.r;
            }
            k = size_t(r1);
            continue;
        }
        Stage& st = stages[k];
        Pipe p;
        int out = final_out;
        if (k + 1 < m) {
            p.make();
            out = p.w;
        }
        Pipe e;
        int err_to = -1;
        if (st.pipe_err && out >= 0) err_to = out;
        else if (st.pipe_err) err_to = 1;
        else if (tee && e.make()) {
            err_to = e.w;
            st.errfd = e.r;
        }
        bool ok = spawn(st, in, out, err_to, pgid, h, oc.error);
        if (st.pid > 0 && !pgid) pgid = st.pid;
        if (k == 0 && shell().job_control && !bg && st.pid > 0) give_terminal(pgid);
        close_fd(e.w);
        if (in >= 0) close_fd(in);
        if (out >= 0 && out != final_out) close(out);
        in = p.r;
        if (!ok) {
            close_fd(in);
            break;
        }
    }
    if (in >= 0) close_fd(in);
    if (final_out >= 0) close_fd(cap.w);
    if (bg) {
        Job j;
        j.id = jobs.empty() ? 1 : jobs.back().id + 1;
        j.pgid = pgid;
        for (auto& s : stages)
            if (s.pid > 0) j.pids.push_back(s.pid);
        j.text = text;
        j.status.assign(j.pids.size(), 0);
        if (!j.pids.empty()) {
            shell().last_bg = j.pids.back();
            if (shell().interactive) say(2, "[" + std::to_string(j.id) + "] " + std::to_string(j.pids.back()) + "\n");
            jobs.push_back(j);
        }
        pr.code = 0;
        pr.statuses = {0};
        return true;
    }
    // Read what comes back: captured output, error output to keep, lines for Ruby stages.
    bool ignore_int = !shell().interactive;
    struct sigaction ign {}, old_int {}, old_quit {};
    ign.sa_handler = SIG_IGN;
    if (ignore_int) {
        sigaction(SIGINT, &ign, &old_int);
        sigaction(SIGQUIT, &ign, &old_quit);
    }
    std::string partial;
    bool stage_failed = false;
    auto feed = [&](const std::string& ln) {
        if (stage_failed) return;
        std::string cur = ln;
        bool keep = true;
        for (int k = r0; k <= r1 && keep; ++k) {
            std::string out;
            if (!h.stage(stages[k].block, cur, out, keep)) {
                stage_failed = true;
                return;
            }
            if (keep) cur = out;
        }
        if (!keep) return;
        cur += '\n';
        if (stage_out == final_out && final_out >= 0) oc.out += cur;
        else say(stage_out, cur);
    };
    for (;;) {
        std::vector<pollfd> fds;
        if (cap.r >= 0) fds.push_back({cap.r, POLLIN, 0});
        if (stage_in >= 0) fds.push_back({stage_in, POLLIN, 0});
        for (auto& s : stages)
            if (s.errfd >= 0) fds.push_back({s.errfd, POLLIN, 0});
        if (fds.empty()) break;
        if (poll(fds.data(), fds.size(), -1) < 0) {
            if (errno == EINTR) continue;
            break;
        }
        for (auto& f : fds) {
            if (!(f.revents & (POLLIN | POLLHUP | POLLERR))) continue;
            char buf[16384];
            ssize_t n = read(f.fd, buf, sizeof buf);
            if (n < 0 && errno == EINTR) continue;
            if (f.fd == cap.r) {
                if (n <= 0) close_fd(cap.r);
                else oc.out.append(buf, size_t(n));
            } else if (f.fd == stage_in) {
                if (n <= 0) {
                    if (!partial.empty()) feed(partial);
                    partial.clear();
                    close_fd(stage_in);
                    if (stage_out_mine) close_fd(stage_out);
                } else {
                    partial.append(buf, size_t(n));
                    size_t p;
                    while (!stage_failed && (p = partial.find('\n')) != std::string::npos) {
                        feed(partial.substr(0, p));
                        partial.erase(0, p + 1);
                    }
                    if (stage_failed) {
                        close_fd(stage_in);
                        if (stage_out_mine) close_fd(stage_out);
                    }
                }
            } else {
                for (auto& s : stages)
                    if (s.errfd == f.fd) {
                        if (n <= 0) close_fd(s.errfd);
                        else {
                            say(2, std::string(buf, size_t(n)));
                            keep_tail(s.tail, buf, size_t(n));
                        }
                    }
            }
        }
    }
    if (stage_out_mine) close_fd(stage_out);
    for (auto& s : stages) {
        if (s.pid <= 0 || s.done) continue;
        bool stopped = false;
        s.status = wait_pid(s.pid, &s.sig, &stopped);
        if (stopped) pr.stopped = true;
        if (s.sig == SIGINT) pr.interrupted = true;
    }
    if (ignore_int) {
        sigaction(SIGINT, &old_int, nullptr);
        sigaction(SIGQUIT, &old_quit, nullptr);
    }
    if (shell().job_control) give_terminal(shell().pgid);
    if (pr.stopped) {
        Job j;
        j.id = jobs.empty() ? 1 : jobs.back().id + 1;
        j.pgid = pgid;
        j.text = text;
        j.stopped = true;
        for (auto& s : stages)
            if (s.pid > 0) j.pids.push_back(s.pid);
        j.status.assign(j.pids.size(), 0);
        jobs.push_back(j);
        say(2, "\n[" + std::to_string(j.id) + "]+ Stopped  " + text + "\n");
    }
    if (stage_failed) oc.ruby_error = true;
    pr.code = 0;
    for (size_t k = 0; k < m; ++k) {
        // A writer ended by SIGPIPE before the last stage: the reader stopped on purpose
        // (yes | head -2). Its status stays visible in $?.statuses.
        bool reader_done = k + 1 < m && stages[k].sig == SIGPIPE;
        pr.statuses.push_back(stages[k].status);
        if (reader_done) continue;
        if (stages[k].status != 0) {
            pr.code = stages[k].status;
            pr.failed = int(k);
            pr.sig = stages[k].sig;
        }
    }
    return true;
}

}  // namespace

Outcome run(int idx, const std::vector<Value>& vals, Host& h, const Options& o) {
    Outcome oc;
    const Line& L = table()[size_t(idx)];
    int last = shell().last_status;
    int last_item = -1;
    Op prev = Op::End;
    std::vector<Stage> fail_stages;
    int fail_stage = -1;
    for (size_t k = 0; k < L.items.size(); ++k) {
        const Item& it = L.items[k];
        bool go = prev == Op::And ? last == 0 : prev == Op::Or ? last != 0 : true;
        prev = it.op;
        if (!go) continue;
        std::vector<Stage> stages;
        PipeResult pr;
        if (!pipeline(it, vals, h, o, it.op == Op::Bg, L.text, oc, pr, stages)) {
            oc.code = 1;
            shell().last_status = 1;
            return oc;
        }
        if (oc.ruby_error) {
            oc.code = 1;
            shell().last_status = 1;
            return oc;
        }
        last = pr.code;
        if (it.bang) last = last ? 0 : 1;
        last_item = int(k);
        oc.statuses = pr.statuses;
        oc.sig = pr.sig;
        if (pr.interrupted) oc.interrupted = true;
        fail_stage = it.bang ? -1 : pr.failed;
        fail_stages = std::move(stages);
        if (oc.interrupted && !shell().interactive) break;
    }
    oc.code = last;
    shell().last_status = last;
    bool final_item = last_item == int(L.items.size()) - 1 && L.items.back().op != Op::Bg;
    if (last != 0 && final_item && !o.cond && !o.relaxed && !shell().interactive && fail_stage >= 0) {
        oc.raise = true;
        oc.failed_argv = fail_stages[size_t(fail_stage)].argv;
        oc.tail = fail_stages[size_t(fail_stage)].tail;
    }
    return oc;
}

Outcome run_argv(const std::vector<std::string>& argv, Host& h, const Options& o) {
    Line L;
    L.file = "(system)";
    Item it;
    Cmd c;
    std::vector<Value> vals;
    for (auto& a : argv) {
        Word w;
        Part p;
        p.k = Part::Val;
        p.idx = int(vals.size());
        w.parts.push_back(p);
        c.words.push_back(w);
        Value v;
        v.items = {a};
        vals.push_back(v);
        L.text += (L.text.empty() ? "" : " ") + a;
    }
    it.pipeline.push_back(c);
    L.items.push_back(it);
    table().push_back(L);
    int idx = int(table().size()) - 1;
    Outcome oc = run(idx, vals, h, o);
    table().pop_back();
    return oc;
}

void report_jobs() {
    for (auto& j : jobs) {
        if (j.done) continue;
        bool all = true;
        for (size_t k = 0; k < j.pids.size(); ++k) {
            if (j.pids[k] <= 0) continue;
            int st;
            pid_t r = waitpid(j.pids[k], &st, WNOHANG | (shell().job_control ? WUNTRACED : 0));
            if (r == j.pids[k] && !WIFSTOPPED(st)) {
                j.status[k] = decode(st, nullptr);
                j.pids[k] = -j.pids[k];
            } else all = false;
        }
        if (all) {
            j.done = true;
            if (shell().interactive) say(2, "[" + std::to_string(j.id) + "]  Done  " + j.text + "\n");
        }
    }
    jobs.erase(std::remove_if(jobs.begin(), jobs.end(), [](const Job& j) { return j.done; }), jobs.end());
}

void setup_interactive() {
    Shell& s = shell();
    s.interactive = true;
    if (!isatty(0)) return;
    s.tty = 0;
    while (tcgetpgrp(0) != getpgrp()) kill(-getpgrp(), SIGTTIN);
    for (int sig : {SIGINT, SIGQUIT, SIGTSTP, SIGTTIN, SIGTTOU}) signal(sig, SIG_IGN);
    s.pgid = getpid();
    setpgid(s.pgid, s.pgid);
    tcsetpgrp(0, s.pgid);
    s.job_control = true;
}

namespace {

Job* find_job(const std::vector<std::string>& a) {
    if (jobs.empty()) return nullptr;
    if (a.size() < 2) return &jobs.back();
    std::string w = a[1];
    if (!w.empty() && w[0] == '%') w = w.substr(1);
    int id = std::atoi(w.c_str());
    for (auto& j : jobs)
        if (j.id == id) return &j;
    return nullptr;
}

int wait_job(Job& j) {
    int code = 0;
    bool stopped = false;
    for (size_t k = 0; k < j.pids.size(); ++k) {
        if (j.pids[k] <= 0) {
            code = j.status[k];
            continue;
        }
        bool st = false;
        code = wait_pid(j.pids[k], nullptr, &st);
        if (st) stopped = true;
        else {
            j.status[k] = code;
            j.pids[k] = -j.pids[k];
        }
    }
    j.stopped = stopped;
    if (!stopped) j.done = true;
    return code;
}

int cd_to(const std::string& dir) {
    char old[4096];
    if (!getcwd(old, sizeof old)) old[0] = 0;
    if (chdir(dir.c_str()) != 0) {
        complain("cd: " + dir + ": " + std::strerror(errno));
        return 1;
    }
    char now[4096];
    setenv("OLDPWD", old, 1);
    if (getcwd(now, sizeof now)) setenv("PWD", now, 1);
    return 0;
}

int run_builtin(const std::vector<std::string>& a, Host& h) {
    const std::string& n = a[0];
    if (n == "cd") {
        if (a.size() > 2) {
            complain("cd: one folder at a time");
            return 2;
        }
        if (a.size() == 1) return cd_to(home_of(""));
        if (a[1] == "-") {
            const char* o = getenv("OLDPWD");
            if (!o) {
                complain("cd: no previous folder");
                return 1;
            }
            std::string dir = o;
            int r = cd_to(dir);
            if (!r) say(1, dir + "\n");
            return r;
        }
        return cd_to(a[1]);
    }
    if (n == "pwd") {
        char now[4096];
        if (!getcwd(now, sizeof now)) return 1;
        say(1, std::string(now) + "\n");
        return 0;
    }
    if (n == "pushd" || n == "popd" || n == "dirs") {
        auto& st = shell().dirstack;
        char now[4096];
        if (!getcwd(now, sizeof now)) now[0] = 0;
        if (n == "pushd") {
            if (a.size() != 2) {
                complain("pushd: one folder");
                return 2;
            }
            if (cd_to(a[1])) return 1;
            st.push_back(now);
        } else if (n == "popd") {
            if (st.empty()) {
                complain("popd: the folder stack is empty");
                return 1;
            }
            std::string d = st.back();
            st.pop_back();
            if (cd_to(d)) return 1;
        }
        if (!getcwd(now, sizeof now)) now[0] = 0;
        std::string line = now;
        for (auto it = st.rbegin(); it != st.rend(); ++it) line += " " + *it;
        say(1, line + "\n");
        return 0;
    }
    if (n == "export") {
        if (a.size() == 1) {
            std::vector<std::string> e;
            for (char** p = environ; *p; ++p) e.push_back(*p);
            std::sort(e.begin(), e.end());
            for (auto& x : e) say(1, x + "\n");
            return 0;
        }
        for (size_t k = 1; k < a.size(); ++k) {
            size_t eq = a[k].find('=');
            std::string name = a[k].substr(0, eq);
            if (!valid_name(name)) {
                complain("export: not a variable name: " + name);
                return 2;
            }
            if (eq != std::string::npos) setenv(name.c_str(), a[k].c_str() + eq + 1, 1);
            else if (!getenv(name.c_str())) setenv(name.c_str(), "", 1);
        }
        return 0;
    }
    if (n == "unset") {
        for (size_t k = 1; k < a.size(); ++k) unsetenv(a[k].c_str());
        return 0;
    }
    if (n == "umask") {
        if (a.size() == 1) {
            mode_t m = umask(0);
            umask(m);
            char buf[8];
            std::snprintf(buf, sizeof buf, "%04o", unsigned(m));
            say(1, std::string(buf) + "\n");
            return 0;
        }
        umask(mode_t(std::strtol(a[1].c_str(), nullptr, 8)));
        return 0;
    }
    if (n == "exec") {
        if (a.size() == 1) return 0;  // only redirects: they stay (handled by the caller)
        std::string prog = find_program(a[1]);
        if (prog.empty()) {
            complain(a[1] + ": command not found");
            return 127;
        }
        std::vector<char*> argv;
        std::vector<std::string> copy(a.begin() + 1, a.end());
        for (auto& x : copy) argv.push_back(x.data());
        argv.push_back(nullptr);
        child_signals_default();
        execv(prog.c_str(), argv.data());
        complain(a[1] + ": " + std::strerror(errno));
        return 126;
    }
    if (n == "echo") {
        size_t k = 1;
        bool nl = true, esc = false;
        while (k < a.size() && (a[k] == "-n" || a[k] == "-e" || a[k] == "-ne" || a[k] == "-en")) {
            if (a[k].find('n') != std::string::npos) nl = false;
            if (a[k].find('e') != std::string::npos) esc = true;
            ++k;
        }
        std::string out;
        for (size_t j = k; j < a.size(); ++j) {
            if (j > k) out += ' ';
            if (!esc) {
                out += a[j];
                continue;
            }
            const std::string& s = a[j];
            for (size_t i = 0; i < s.size(); ++i) {
                if (s[i] != '\\' || i + 1 >= s.size()) {
                    out += s[i];
                    continue;
                }
                char c = s[++i];
                out += c == 'n' ? '\n' : c == 't' ? '\t' : c == '\\' ? '\\' : c == 'e' ? '\033' : c == 'r' ? '\r' : c;
            }
        }
        if (nl) out += '\n';
        say(1, out);
        return 0;
    }
    if (n == "type") {
        int rc = 0;
        for (size_t k = 1; k < a.size(); ++k) {
            if (is_builtin(a[k])) say(1, a[k] + " is a shell builtin\n");
            else {
                std::string p = find_program(a[k]);
                if (p.empty()) {
                    say(1, a[k] + " is not a command\n");
                    rc = 1;
                } else say(1, a[k] + " is " + p + "\n");
            }
        }
        return rc;
    }
    if (n == "jobs") {
        report_jobs();
        for (auto& j : jobs)
            say(1, "[" + std::to_string(j.id) + "]  " + (j.stopped ? "Stopped" : "Running") + "  " + j.text + "\n");
        return 0;
    }
    if (n == "fg" || n == "bg") {
        Job* j = find_job(a);
        if (!j) {
            complain(n + ": no such job");
            return 1;
        }
        if (n == "bg") {
            kill(-j->pgid, SIGCONT);
            j->stopped = false;
            return 0;
        }
        say(2, j->text + "\n");
        give_terminal(j->pgid);
        kill(-j->pgid, SIGCONT);
        int code = wait_job(*j);
        give_terminal(shell().pgid);
        if (j->stopped) say(2, "\n[" + std::to_string(j->id) + "]+ Stopped  " + j->text + "\n");
        jobs.erase(std::remove_if(jobs.begin(), jobs.end(), [](const Job& x) { return x.done; }), jobs.end());
        return code;
    }
    if (n == "wait") {
        int code = 0;
        if (a.size() > 1) {
            Job* j = find_job(a);
            if (!j) return 127;
            code = wait_job(*j);
        } else
            for (auto& j : jobs) code = wait_job(j);
        jobs.erase(std::remove_if(jobs.begin(), jobs.end(), [](const Job& x) { return x.done; }), jobs.end());
        return code;
    }
    if (n == "source") return h.source(a);
    if (n == "history") {
        h.history(1);
        return 0;
    }
    return 127;
}

}  // namespace

}  // namespace rsh

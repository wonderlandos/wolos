// SPDX-License-Identifier: GPL-2.0-or-later
// rsh: running command lines. Every argument is a separate argv entry handed to posix_spawn;
// nothing is joined into a string or given to another shell.
#pragma once
#include <sys/types.h>

#include <string>
#include <vector>

#include "ast.hpp"

namespace rsh {

// One Ruby value of a command line, already turned into text by the Ruby side.
struct Value {
    std::vector<std::string> items;  // one entry, or an Array's entries
    bool array = false;
    bool proc = false;  // a Ruby stage; the host calls it by value number
};

struct Outcome {
    int code = 0;
    std::vector<int> statuses;             // of the pipeline that set `code`
    std::vector<std::string> failed_argv;  // of the command that failed, when `raise`
    std::string tail;                      // the end of that command's error output
    int sig = 0;                           // the signal that ended it, if any
    bool raise = false;                    // a failure the script did not handle
    bool interrupted = false;              // Ctrl-C ended a command (scripts stop)
    std::string out;                       // captured standard output
    std::string error;                     // a mistake in the line itself (nil value, no match)
    bool ruby_error = false;               // a Ruby stage raised; the host has the exception
};

struct Host {
    // Calls Ruby stage `val` with one line (without its newline). Returns false when it raised;
    // `keep` false drops the line, else `out` is written (with a newline).
    virtual bool stage(int val, const std::string& line, std::string& out, bool& keep) = 0;
    virtual int source(const std::vector<std::string>& argv) = 0;
    virtual const std::vector<std::string>& args() = 0;
    virtual void history(int fd) = 0;
    virtual ~Host() = default;
};

struct Options {
    bool cond = false;      // a condition or a value: failures do not raise
    bool capture = false;   // keep standard output in Outcome::out
    bool relaxed = false;   // relaxed!: failures do not raise
    bool tee = true;        // keep the end of error output for Rsh::Failed
};

struct Shell {
    bool interactive = false;
    bool job_control = false;
    bool nullglob = false;
    int last_status = 0;
    pid_t last_bg = 0;
    pid_t pgid = 0;
    int tty = -1;
    std::vector<std::string> dirstack;
};
Shell& shell();

Outcome run(int line, const std::vector<Value>& vals, Host& h, const Options& o);
// argv given as separate strings (Kernel#system, Rsh.run): no parsing at all.
Outcome run_argv(const std::vector<std::string>& argv, Host& h, const Options& o);

bool is_builtin(const std::string& name);
// The program a command word runs, from PATH; empty when there is none.
std::string find_program(const std::string& name);
// Home folder of `user` from /etc/passwd ("" = the current person's $HOME).
std::string home_of(const std::string& user);
// Jobs that ended since the last call, as "[1]+ Done  text" lines (interactive).
void report_jobs();
void setup_interactive();

// Expands one word into arguments (tests use this).
bool expand(const Word& w, const std::vector<Value>& vals, Host& h, std::vector<std::string>& out, std::string& err,
            bool glob_ok = true);

}  // namespace rsh

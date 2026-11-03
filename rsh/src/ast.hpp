// SPDX-License-Identifier: GPL-2.0-or-later
// rsh: the parsed form of one command line (docs/modules/rsh.md). The transpiler builds it once;
// the runner gets it by index with the Ruby values of that run.
#pragma once
#include <string>
#include <utility>
#include <vector>

namespace rsh {

// One piece of a word. Lit: quoted text, never globbed. Pat: unquoted text (glob, braces, tilde).
// Val: the Ruby value number idx (one argument; an Array gives one argument per element).
// Env: $NAME. Status: $?. Args: $* (every script argument). BgPid: $!.
struct Part {
    enum Kind : unsigned char { Lit, Pat, Val, Env, Status, Args, BgPid };
    Kind k = Lit;
    std::string s;
    int idx = -1;
};

struct Word {
    std::vector<Part> parts;
};

struct Redir {
    // In: < file. Out: > file. Append: >> file. Dup: n>&m. Both: &> file. BothAppend: &>> file.
    // Here: a here-doc or here-string; its text is `target`.
    enum Kind : unsigned char { In, Out, Append, Dup, Both, BothAppend, Here };
    Kind k = Out;
    int fd = 1;
    int dupfd = -1;
    Word target;
};

struct Cmd {
    std::vector<std::pair<std::string, Word>> env;  // NAME=value before the command word
    std::vector<Word> words;
    std::vector<Redir> redirs;
    int block = -1;         // a Ruby stage: the value number of its Proc
    bool pipe_err = false;  // |& after this command: stderr goes down the pipe too
};

// What joins an item to the next one.
enum class Op : unsigned char { End, And, Or, Bg };

struct Item {
    std::vector<Cmd> pipeline;
    Op op = Op::End;
    bool bang = false;  // ! cmd: the status is inverted
};

struct Line {
    std::vector<Item> items;
    std::string file;
    int line = 0;
    std::string text;  // the source text, for messages and --explain
};

// Every command line of every file loaded in this process, by number.
std::vector<Line>& table();

}  // namespace rsh

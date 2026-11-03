// SPDX-License-Identifier: GPL-2.0-or-later
#include "edit.hpp"

#include <fcntl.h>
#include <sys/ioctl.h>
#include <sys/stat.h>
#include <termios.h>
#include <unistd.h>

#include <algorithm>
#include <cerrno>
#include <cstdio>
#include <cstring>

namespace rsh {

namespace {

void out(const std::string& s) {
    size_t o = 0;
    while (o < s.size()) {
        ssize_t w = write(1, s.data() + o, s.size() - o);
        if (w < 0 && errno == EINTR) continue;
        if (w <= 0) return;
        o += size_t(w);
    }
}

bool cont(unsigned char c) { return (c & 0xC0) == 0x80; }

size_t prev_char(const std::string& s, size_t i) {
    if (i == 0) return 0;
    --i;
    while (i > 0 && cont((unsigned char)s[i])) --i;
    return i;
}
size_t next_char(const std::string& s, size_t i) {
    if (i >= s.size()) return s.size();
    ++i;
    while (i < s.size() && cont((unsigned char)s[i])) ++i;
    return i;
}

int read_key() {
    unsigned char c;
    for (;;) {
        ssize_t n = read(0, &c, 1);
        if (n == 1) return c;
        if (n < 0 && errno == EINTR) continue;
        return -1;
    }
}

enum Key { K_UP = 1000, K_DOWN, K_LEFT, K_RIGHT, K_HOME, K_END, K_DEL, K_WLEFT, K_WRIGHT };

int escape() {
    int a = read_key();
    if (a == 'b') return K_WLEFT;
    if (a == 'f') return K_WRIGHT;
    if (a != '[' && a != 'O') return -2;
    int b = read_key();
    if (b >= '0' && b <= '9') {
        std::string num(1, char(b));
        int c;
        while ((c = read_key()) >= 0 && ((c >= '0' && c <= '9') || c == ';')) num += char(c);
        if (c == '~') {
            if (num == "3") return K_DEL;
            if (num == "1" || num == "7") return K_HOME;
            if (num == "4" || num == "8") return K_END;
        }
        if (num == "1;5" && c == 'D') return K_WLEFT;
        if (num == "1;5" && c == 'C') return K_WRIGHT;
        return -2;
    }
    switch (b) {
        case 'A': return K_UP;
        case 'B': return K_DOWN;
        case 'C': return K_RIGHT;
        case 'D': return K_LEFT;
        case 'H': return K_HOME;
        case 'F': return K_END;
    }
    return -2;
}

int columns() {
    winsize ws;
    if (ioctl(1, TIOCGWINSZ, &ws) == 0 && ws.ws_col > 0) return ws.ws_col;
    return 80;
}

std::string common_prefix(const std::vector<std::string>& v) {
    if (v.empty()) return "";
    std::string p = v[0];
    for (auto& s : v) {
        size_t k = 0;
        while (k < p.size() && k < s.size() && p[k] == s[k]) ++k;
        p.resize(k);
    }
    return p;
}

}  // namespace

size_t visible_width(const std::string& s) {
    size_t w = 0;
    for (size_t i = 0; i < s.size(); ++i) {
        unsigned char c = (unsigned char)s[i];
        if (c == 0x1b) {  // CSI ... letter
            ++i;
            if (i < s.size() && s[i] == '[')
                while (i + 1 < s.size() && !std::isalpha((unsigned char)s[i + 1])) ++i;
            ++i;
            continue;
        }
        if (!cont(c)) ++w;
    }
    return w;
}

void Editor::load() {
    if (history_file.empty()) return;
    FILE* f = std::fopen(history_file.c_str(), "r");
    if (!f) return;
    char buf[8192];
    while (std::fgets(buf, sizeof buf, f)) {
        std::string l(buf);
        while (!l.empty() && l.back() == '\n') l.pop_back();
        // a stored entry keeps its own newlines as \n
        std::string e;
        for (size_t k = 0; k < l.size(); ++k) {
            if (l[k] == '\\' && k + 1 < l.size()) {
                e += l[k + 1] == 'n' ? '\n' : l[k + 1];
                ++k;
            } else e += l[k];
        }
        if (!e.empty()) history.push_back(e);
    }
    std::fclose(f);
    if (history.size() > 5000) history.erase(history.begin(), history.end() - 5000);
}

void Editor::add(const std::string& entry) {
    if (entry.empty() || (!history.empty() && history.back() == entry)) return;
    history.push_back(entry);
    if (history_file.empty()) return;
    int fd = open(history_file.c_str(), O_WRONLY | O_CREAT | O_APPEND | O_CLOEXEC, 0600);
    if (fd < 0) return;
    std::string l;
    for (char c : entry) {
        if (c == '\n') l += "\\n";
        else if (c == '\\') l += "\\\\";
        else l += c;
    }
    l += '\n';
    if (write(fd, l.data(), l.size()) < 0) {
    }
    close(fd);
}

bool Editor::read(const std::string& prompt, std::string& result) {
    if (!isatty(0)) {  // no terminal: plain lines
        out(prompt);
        std::string l;
        int c;
        while ((c = read_key()) >= 0 && c != '\n') l += char(c);
        if (c < 0 && l.empty()) return false;
        result = l;
        return true;
    }
    termios old, raw;
    tcgetattr(0, &old);
    raw = old;
    raw.c_iflag &= ~tcflag_t(BRKINT | ICRNL | INPCK | ISTRIP | IXON);
    raw.c_lflag &= ~tcflag_t(ECHO | ICANON | IEXTEN | ISIG);
    raw.c_cc[VMIN] = 1;
    raw.c_cc[VTIME] = 0;
    tcsetattr(0, TCSAFLUSH, &raw);

    std::string buf;
    size_t pos = 0;
    size_t hidx = history.size();
    std::string saved;
    size_t pw = visible_width(prompt);
    bool tabbed = false;

    auto draw = [&] {
        std::string shown = paint ? paint(buf) : buf;
        size_t col = pw + visible_width(buf.substr(0, pos));
        std::string s = "\r" + prompt + shown + "\x1b[K\r";
        int cols = columns();
        if (col >= size_t(cols)) col = size_t(cols) - 1;
        if (col) s += "\x1b[" + std::to_string(col) + "C";
        out(s);
    };
    auto done = [&](bool ok) {
        pos = buf.size();
        draw();
        out("\r\n");
        tcsetattr(0, TCSAFLUSH, &old);
        result = buf;
        return ok;
    };
    draw();
    for (;;) {
        int k = read_key();
        if (k < 0) return done(!buf.empty());
        if (k == 27) k = escape();
        bool was_tab = tabbed;
        tabbed = false;
        switch (k) {
            case '\r':
            case '\n': return done(true);
            case 4:  // Ctrl-D
                if (buf.empty()) {
                    out("\r\n");
                    tcsetattr(0, TCSAFLUSH, &old);
                    return false;
                }
                [[fallthrough]];
            case K_DEL:
                if (pos < buf.size()) buf.erase(pos, next_char(buf, pos) - pos);
                break;
            case 3:  // Ctrl-C: drop the line
                out("^C");
                buf.clear();
                return done(true);
            case 127:
            case 8:
                if (pos > 0) {
                    size_t p = prev_char(buf, pos);
                    buf.erase(p, pos - p);
                    pos = p;
                }
                break;
            case 1:
            case K_HOME: pos = 0; break;
            case 5:
            case K_END: pos = buf.size(); break;
            case 2:
            case K_LEFT: pos = prev_char(buf, pos); break;
            case 6:
            case K_RIGHT: pos = next_char(buf, pos); break;
            case K_WLEFT:
                while (pos > 0 && buf[pos - 1] == ' ') --pos;
                while (pos > 0 && buf[pos - 1] != ' ') --pos;
                break;
            case K_WRIGHT:
                while (pos < buf.size() && buf[pos] == ' ') ++pos;
                while (pos < buf.size() && buf[pos] != ' ') ++pos;
                break;
            case 11: buf.erase(pos); break;  // Ctrl-K
            case 21:                         // Ctrl-U
                buf.erase(0, pos);
                pos = 0;
                break;
            case 23: {  // Ctrl-W
                size_t p = pos;
                while (p > 0 && buf[p - 1] == ' ') --p;
                while (p > 0 && buf[p - 1] != ' ') --p;
                buf.erase(p, pos - p);
                pos = p;
                break;
            }
            case 12: out("\x1b[H\x1b[2J"); break;  // Ctrl-L
            case 16:
            case K_UP:
                if (hidx > 0) {
                    if (hidx == history.size()) saved = buf;
                    buf = history[--hidx];
                    pos = buf.size();
                }
                break;
            case 14:
            case K_DOWN:
                if (hidx < history.size()) {
                    ++hidx;
                    buf = hidx == history.size() ? saved : history[hidx];
                    pos = buf.size();
                }
                break;
            case 18: {  // Ctrl-R: search back
                std::string q;
                size_t found = history.size();
                for (;;) {
                    std::string shown = found < history.size() ? history[found] : "";
                    out("\r(search) " + q + ": " + shown + "\x1b[K");
                    int c = read_key();
                    if (c == 127 || c == 8) {
                        if (!q.empty()) q.pop_back();
                    } else if (c == 18) {
                        size_t from = found;
                        found = history.size();
                        for (size_t h = from; h-- > 0;)
                            if (history[h].find(q) != std::string::npos) {
                                found = h;
                                break;
                            }
                        continue;
                    } else if (c >= 32 && c != 127) q += char(c);
                    else {
                        if (found < history.size() && c != 3 && c != 7) {
                            buf = history[found];
                            pos = buf.size();
                        }
                        break;
                    }
                    found = history.size();
                    for (size_t h = history.size(); h-- > 0;)
                        if (history[h].find(q) != std::string::npos) {
                            found = h;
                            break;
                        }
                }
                break;
            }
            case '\t': {
                if (!complete) break;
                size_t start = pos;
                while (start > 0 && buf[start - 1] != ' ' && buf[start - 1] != '\t') --start;
                auto c = complete(buf.substr(0, pos), start);
                if (c.empty()) break;
                std::string word = buf.substr(start, pos - start);
                std::string p = common_prefix(c);
                if (c.size() == 1) p += c[0].back() == '/' ? "" : " ";
                if (p.size() > word.size()) {
                    buf.replace(start, pos - start, p);
                    pos = start + p.size();
                } else if (was_tab || c.size() > 1) {
                    std::string list = "\r\n";
                    size_t n = 0;
                    for (auto& x : c) {
                        if (n++ >= 100) {
                            list += "...";
                            break;
                        }
                        list += x + "  ";
                    }
                    out(list + "\r\n");
                }
                tabbed = true;
                break;
            }
            default:
                if (k >= 32 && k < 256 && k != 127) {
                    buf.insert(pos, 1, char(k));
                    ++pos;
                }
        }
        draw();
    }
}

}  // namespace rsh

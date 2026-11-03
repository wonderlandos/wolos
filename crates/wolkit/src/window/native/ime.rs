// SPDX-License-Identifier: BSD-2-Clause
//! The client side of text-input-v3, as a state machine without Wayland: given whether a field
//! wants the input method, the surrounding text and the cursor rectangle, it says which requests
//! to send (enable, content type, surrounding text, cursor rectangle, disable, commit) and counts
//! commits, because a `done(serial)` is answered with fresh state only when its serial is the
//! number of commits sent (the protocol's rule; the batch itself is applied either way).

/// One request to send, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Out {
    Enable,
    Disable,
    ContentType {
        hint: u32,
        purpose: u32,
    },
    Surrounding {
        text: String,
        cursor: i32,
        anchor: i32,
    },
    Cursor {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    },
    Commit,
}

/// What the focused field wants from the input method right now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Want {
    pub on: bool,
    /// Text around the caret, caret and anchor (bytes), as `edit::surrounding` gives it.
    pub surrounding: Option<(String, usize, usize)>,
    pub rect: Option<[i32; 4]>,
    pub hint: u32,
    pub purpose: u32,
}

#[derive(Clone, Debug, Default)]
pub struct TextInput {
    /// The input method entered our surface (`enter`).
    pub entered: bool,
    enabled: bool,
    commits: u32,
    sent_surrounding: Option<(String, usize, usize)>,
    sent_rect: Option<[i32; 4]>,
}

fn i(v: usize) -> i32 {
    i32::try_from(v).unwrap_or(i32::MAX)
}

impl TextInput {
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn commits(&self) -> u32 {
        self.commits
    }

    fn state(&mut self, want: &Want, out: &mut Vec<Out>, force: bool) {
        if let Some(s) = &want.surrounding
            && (force || self.sent_surrounding.as_ref() != Some(s))
        {
            out.push(Out::Surrounding {
                text: s.0.clone(),
                cursor: i(s.1),
                anchor: i(s.2),
            });
            self.sent_surrounding = Some(s.clone());
        }
        if let Some(r) = want.rect
            && (force || self.sent_rect != Some(r))
        {
            out.push(Out::Cursor {
                x: r[0],
                y: r[1],
                w: r[2],
                h: r[3],
            });
            self.sent_rect = Some(r);
        }
    }

    fn commit(&mut self, out: &mut Vec<Out>) {
        out.push(Out::Commit);
        self.commits = self.commits.wrapping_add(1);
    }

    /// Bring the compositor's view in line with `want`.
    pub fn update(&mut self, want: &Want, out: &mut Vec<Out>) {
        let should = want.on && self.entered;
        if should && !self.enabled {
            self.enabled = true;
            out.push(Out::Enable);
            out.push(Out::ContentType {
                hint: want.hint,
                purpose: want.purpose,
            });
            self.state(want, out, true);
            self.commit(out);
        } else if !should && self.enabled {
            self.enabled = false;
            self.sent_surrounding = None;
            self.sent_rect = None;
            out.push(Out::Disable);
            self.commit(out);
        } else if should {
            let before = out.len();
            self.state(want, out, false);
            if out.len() > before {
                self.commit(out);
            }
        }
    }

    /// After a `done(serial)` was applied: whether to answer with the new state (the serial is
    /// current), and the requests to do so.
    pub fn done(&mut self, serial: u32, want: &Want, out: &mut Vec<Out>) -> bool {
        if serial != self.commits || !self.enabled {
            return false;
        }
        let before = out.len();
        self.state(want, out, false);
        if out.len() > before {
            self.commit(out);
        }
        true
    }

    /// The input method left our surface: it forgets everything we told it.
    pub fn leave(&mut self) {
        self.entered = false;
        self.enabled = false;
        self.sent_surrounding = None;
        self.sent_rect = None;
    }
}

/// Text-input purposes and hints (the protocol's numbers).
pub mod content {
    pub const HINT_NONE: u32 = 0;
    pub const HINT_SENSITIVE_DATA: u32 = 0x80;
    pub const HINT_HIDDEN_TEXT: u32 = 0x40;
    pub const PURPOSE_NORMAL: u32 = 0;
    pub const PURPOSE_PASSWORD: u32 = 8;
}

/// The text-input protocol counts the cursor and anchor in bytes of the surrounding text and
/// the preedit cursor in bytes of the preedit; both may be -1 ("hidden"). A cursor outside the
/// text or off a character boundary is dropped.
pub fn preedit_cursor(text: &str, begin: i32, end: i32) -> Option<(usize, usize)> {
    let b = usize::try_from(begin).ok()?;
    let e = usize::try_from(end).ok()?;
    (b <= e && e <= text.len() && text.is_char_boundary(b) && text.is_char_boundary(e))
        .then_some((b, e))
}

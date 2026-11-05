// SPDX-License-Identifier: GPL-2.0-or-later
//! One verb, start to end: the file is looked at, wold is asked, and its answer becomes the reply
//! the browser shows. wold's own texts are never passed on (they are written for app developers and
//! could name a path); every text is one of `shared::atlas`'s.
use crate::wold::{UNKNOWN_CALL, Wold};
use shared::atlas::{ErrorCode, Outcome, Reply, TEXT_UNREACHABLE, Verb, text};
use shared::portal::{Answer, BUSY, Call, Code};
use std::process::{Command, Stdio};

/// The fallback opener for a WolOS older than libwol 1.15: the program and the arguments before the
/// path. xdg-open takes no `--` (it refuses any argument starting with '-'); the path is a full path
/// (`shared::portal::file_path`), so it starts with '/' and never reads as an option.
pub const OPENER: &[&str] = &["xdg-open"];

fn error(id: u32, verb: Verb, code: ErrorCode) -> Reply {
    Reply::error(id, code, text(verb, code))
}

/// What the file is, before wold is asked: gone, or a folder where only a file can go.
fn look(id: u32, verb: Verb, path: &str) -> Option<Reply> {
    if std::fs::symlink_metadata(path).is_err() {
        return Some(error(id, verb, ErrorCode::NotFound));
    }
    if verb != Verb::Trash {
        match std::fs::metadata(path) {
            Err(_) => return Some(error(id, verb, ErrorCode::NotFound)),
            Ok(m) if !m.is_file() => return Some(error(id, verb, ErrorCode::Failed)),
            Ok(_) => {}
        }
    }
    None
}

/// Run `verb` on `path` for request `id`. Blocks while the person chooses.
pub fn run(
    id: u32,
    verb: Verb,
    path: &str,
    wold: &dyn Wold,
    opener: &[String],
    log: &dyn Fn(&str),
) -> Reply {
    if let Some(r) = look(id, verb, path) {
        return r;
    }
    let path = path.to_owned();
    let call = match verb {
        Verb::Share => Call::ShareFile { path: path.clone() },
        Verb::OpenWith => Call::OpenWith { path: path.clone() },
        Verb::Trash => Call::FileTrash { path: path.clone() },
    };
    let a = wold.call(&call);
    if a.ok {
        let outcome = match verb {
            Verb::Trash => Outcome::Trashed,
            _ => Outcome::of_done(
                a.data
                    .as_ref()
                    .and_then(|d| d.get("done"))
                    .and_then(|d| d.as_str()),
            ),
        };
        return Reply::Ok { id, outcome };
    }
    refused(id, verb, &path, &a, opener, log)
}

fn refused(
    id: u32,
    verb: Verb,
    path: &str,
    a: &Answer,
    opener: &[String],
    log: &dyn Fn(&str),
) -> Reply {
    let msg = a.message.as_deref().unwrap_or("");
    match a.code {
        Some(Code::PermissionDenied) => error(id, verb, ErrorCode::PermissionDenied),
        Some(Code::Unavailable) if msg == BUSY => error(id, verb, ErrorCode::Busy),
        Some(Code::Unavailable) => Reply::error(id, ErrorCode::Unavailable, TEXT_UNREACHABLE),
        Some(Code::InvalidArgument) if msg == UNKNOWN_CALL => older(id, verb, path, opener, log),
        _ => error(id, verb, ErrorCode::Failed),
    }
}

/// A wold older than libwol 1.15: Open With opens the file with the default app; Share and Trash
/// cannot be done for the person by anything the bridge may use.
fn older(id: u32, verb: Verb, path: &str, opener: &[String], log: &dyn Fn(&str)) -> Reply {
    if verb != Verb::OpenWith {
        return error(id, verb, ErrorCode::Unavailable);
    }
    let Some((prog, args)) = opener.split_first() else {
        return error(id, verb, ErrorCode::Unavailable);
    };
    let run = Command::new(prog)
        .args(args)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    match run {
        Ok(s) if s.success() => Reply::Ok {
            id,
            outcome: Outcome::Opened,
        },
        // The reason goes to the journal; the person reads the verb's text.
        Ok(s) => {
            log(&format!("{prog} for request {id}: {s}"));
            error(id, verb, ErrorCode::Failed)
        }
        Err(e) => {
            log(&format!("{prog} for request {id}: {e}"));
            error(id, verb, ErrorCode::Failed)
        }
    }
}

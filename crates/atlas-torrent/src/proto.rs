// SPDX-License-Identifier: BSD-2-Clause
//! Protocol `ATT/1` between the browser and the helper: one JSON object per line both ways, over
//! one long-lived connection to `$XDG_RUNTIME_DIR/atlas/torrent.sock`. The browser sends
//! [`Request`]s; the helper sends [`Event`]s: an answer to each request that changes something, and
//! a [`Event::Job`] line whenever a job's row would look different (at most every [`TICK_MS`]).
//!
//! Lines are bounded ([`MAX_LINE`]): a `.torrent` file rides as hex, so the line limit is twice
//! [`crate::metainfo::MAX_TORRENT`] plus room for the field names.
use serde::{Deserialize, Serialize};

/// The version the browser says in its first line.
pub const VERSION: u32 = 1;
/// The longest line either side sends, newline included.
pub const MAX_LINE: usize = 2 * crate::metainfo::MAX_TORRENT + 4096;
/// How often the helper looks at its jobs to send the rows that changed.
pub const TICK_MS: u64 = 500;
/// Jobs one helper runs at once; one more `add` is refused with [`ErrorCode::TooMany`].
pub const MAX_JOBS: usize = 8;

/// What the browser asks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    /// First line of a connection, with the token `launch` printed for the browser that started
    /// this helper (the identity check inside the confinement, see `server::Peers`).
    Hello {
        version: u32,
        #[serde(default)]
        token: Option<String>,
    },
    /// A new download from a magnet link (`magnet:?xt=urn:btih:...`).
    AddMagnet { magnet: String },
    /// A new download from the bytes of a `.torrent` file, hex encoded.
    AddTorrent { torrent_hex: String },
    /// Stop moving data for a job; it keeps its place.
    Pause { id: String },
    /// Start a paused or interrupted job again.
    Resume { id: String },
    /// Forget a job; `delete` also removes what it wrote.
    Cancel { id: String, delete: bool },
    /// Send every job's row now.
    List,
}

/// A row's state, as the download list shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum JobState {
    /// A magnet link: the helper is asking peers for the list of files.
    Metadata,
    /// Pieces already on disk are being checked (after a restart).
    Checking,
    Downloading,
    Paused,
    /// Every piece is on disk and checked; nothing is shared.
    Done,
    /// Every piece is on disk and the person allowed sharing.
    Seeding,
    Failed,
}

/// A refusal, with the words the row shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ErrorCode {
    BadRequest,
    BadMagnet,
    BadTorrent,
    UnsafePath,
    TooMany,
    NotFound,
    Engine,
}

impl ErrorCode {
    /// The sentence the row or the sheet shows (sentence case, the person as the subject).
    pub fn text(self) -> &'static str {
        match self {
            ErrorCode::BadRequest => "Atlas sent a request the torrent helper does not understand",
            ErrorCode::BadMagnet => "This magnet link is not one Atlas can open",
            ErrorCode::BadTorrent => "This torrent file is damaged or too large",
            ErrorCode::UnsafePath => {
                "This torrent names files outside its folder, so Atlas did not start it"
            }
            ErrorCode::TooMany => "Atlas runs 8 torrents at once; finish or cancel one first",
            ErrorCode::NotFound => "This torrent is no longer in the list",
            ErrorCode::Engine => "The torrent could not continue",
        }
    }
}

/// What the helper says.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "ev", rename_all = "snake_case", deny_unknown_fields)]
pub enum Event {
    /// Answer to `hello`: the helper's version and whether sharing is allowed in this run.
    Hello { version: u32, sharing: bool },
    /// Answer to an `add`: the job's id (the info hash, 40 hex digits).
    Added { id: String },
    /// A request was refused.
    Error { code: ErrorCode, text: String },
    /// One row.
    Job(Job),
    /// A job left the list (after `cancel`).
    Gone { id: String },
}

/// One row of the download list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Job {
    /// Info hash, 40 lowercase hex digits.
    pub id: String,
    /// The torrent's name (the magnet's `dn` until the metadata arrives).
    pub name: String,
    pub state: JobState,
    /// Bytes on disk and checked.
    pub have: u64,
    /// Bytes in the torrent (0 until the metadata arrives).
    pub total: u64,
    /// Peers connected now.
    pub peers: u32,
    /// Peers known (from trackers, DHT, peer exchange).
    pub peers_known: u32,
    /// Download rate, bytes per second.
    pub down_bps: u64,
    /// The file (a one-file torrent in the downloads folder) or the job's folder.
    pub path: String,
    /// What went wrong, when `state` is `failed`.
    pub error: Option<String>,
    /// The distinct lowercase extensions of the job's files (at most [`MAX_EXTS`], each at most
    /// [`MAX_EXT`] bytes; empty until the metadata arrives): the browser runs its dangerous-file-type
    /// policy on them, as for any other download (DL-SEC-04).
    #[serde(default)]
    pub exts: Vec<String>,
}

/// Most extensions reported for one job.
pub const MAX_EXTS: usize = 32;
/// Longest extension reported.
pub const MAX_EXT: usize = 16;

/// The extensions of `names` for [`Job::exts`]: lowercase, distinct, bounded; a name without an
/// extension adds nothing, a longer extension than [`MAX_EXT`] is reported as `"?"` (unknown).
pub fn exts_of<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for n in names {
        let Some((_, e)) = n.rsplit_once('.') else {
            continue;
        };
        let e = if e.is_empty()
            || e.len() > MAX_EXT
            || !e
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        {
            "?".to_string()
        } else {
            e.to_ascii_lowercase()
        };
        if !out.contains(&e) {
            if out.len() >= MAX_EXTS {
                break;
            }
            out.push(e);
        }
    }
    out
}

/// Parse one line from the browser. The caller already bounded it to [`MAX_LINE`].
pub fn parse_request(line: &str) -> Result<Request, ErrorCode> {
    serde_json::from_str(line.trim_end_matches(['\n', '\r'])).map_err(|_| ErrorCode::BadRequest)
}

/// One event as a line, newline included.
pub fn event_line(ev: &Event) -> String {
    let mut s = serde_json::to_string(ev).unwrap_or_else(|_| {
        "{\"ev\":\"error\",\"code\":\"engine\",\"text\":\"encoding\"}".to_string()
    });
    s.push('\n');
    s
}

/// Decode the hex of a `.torrent` file, refusing more than [`crate::metainfo::MAX_TORRENT`] bytes.
pub fn decode_hex(hex: &str) -> Result<Vec<u8>, ErrorCode> {
    let b = hex.as_bytes();
    if !b.len().is_multiple_of(2) || b.len() > 2 * crate::metainfo::MAX_TORRENT {
        return Err(ErrorCode::BadTorrent);
    }
    let nib = |c: u8| -> Result<u8, ErrorCode> {
        match c {
            b'0'..=b'9' => Ok(c - b'0'),
            b'a'..=b'f' => Ok(c - b'a' + 10),
            b'A'..=b'F' => Ok(c - b'A' + 10),
            _ => Err(ErrorCode::BadTorrent),
        }
    };
    b.as_chunks::<2>()
        .0
        .iter()
        .map(|p| Ok((nib(p[0])? << 4) | nib(p[1])?))
        .collect()
}

/// A job id is exactly 40 lowercase hex digits.
pub fn valid_id(id: &str) -> bool {
    id.len() == 40 && id.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_round_trip_and_unknown_fields_are_refused() {
        let r = parse_request("{\"op\":\"pause\",\"id\":\"ab\"}\n").unwrap();
        assert_eq!(r, Request::Pause { id: "ab".into() });
        assert_eq!(
            parse_request("{\"op\":\"pause\",\"id\":\"ab\",\"x\":1}"),
            Err(ErrorCode::BadRequest)
        );
        assert_eq!(
            parse_request("{\"op\":\"seed_everything\"}"),
            Err(ErrorCode::BadRequest)
        );
        assert_eq!(parse_request("not json"), Err(ErrorCode::BadRequest));
        let c = parse_request("{\"op\":\"cancel\",\"id\":\"ab\",\"delete\":true}").unwrap();
        assert_eq!(
            c,
            Request::Cancel {
                id: "ab".into(),
                delete: true
            }
        );
    }

    #[test]
    fn events_are_one_line() {
        let j = Event::Job(Job {
            id: "0".repeat(40),
            name: "a\nb".into(),
            state: JobState::Downloading,
            have: 1,
            total: 2,
            peers: 1,
            peers_known: 3,
            down_bps: 10,
            path: "/d/a".into(),
            error: None,
            exts: vec!["bin".into()],
        });
        let l = event_line(&j);
        assert_eq!(l.matches('\n').count(), 1);
        assert!(l.ends_with('\n'));
        assert!(l.contains("\"ev\":\"job\"") && l.contains("\"state\":\"downloading\""));
    }

    #[test]
    fn extensions_are_reported_bounded() {
        assert_eq!(
            exts_of([
                "a/evil.DESKTOP",
                "b.sh",
                "c.sh",
                "README",
                "x.y.AppImage",
                "z.\u{202e}exe"
            ]),
            vec!["desktop", "sh", "appimage", "?"]
        );
        let many: Vec<String> = (0..100).map(|i| format!("f.e{i}")).collect();
        assert_eq!(exts_of(many.iter().map(String::as_str)).len(), MAX_EXTS);
    }

    #[test]
    fn hex_is_bounded_and_strict() {
        assert_eq!(decode_hex("00ff7F").unwrap(), vec![0, 255, 127]);
        assert_eq!(decode_hex("0"), Err(ErrorCode::BadTorrent));
        assert_eq!(decode_hex("zz"), Err(ErrorCode::BadTorrent));
        let big = "0".repeat(2 * crate::metainfo::MAX_TORRENT + 2);
        assert_eq!(decode_hex(&big), Err(ErrorCode::BadTorrent));
    }

    #[test]
    fn ids_are_info_hashes() {
        assert!(valid_id(&"a".repeat(40)));
        assert!(!valid_id(&"A".repeat(40)));
        assert!(!valid_id("../../etc"));
        assert!(!valid_id(&"a".repeat(41)));
    }
}

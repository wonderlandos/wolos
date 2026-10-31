// SPDX-License-Identifier: GPL-2.0-or-later
//! Reads the public text signature formats (hash lists, hex-pattern lists, logical lists) into a
//! [`Builder`]. Written from the published format description; shares no code with any scanner.
//! Lines it cannot honour exactly are counted and left out, never guessed at.
use crate::automaton::{Pattern, Place, Tok};
use crate::db::{Builder, Table};
use crate::scan::Kind;

/// The lists [import] reads.
pub const EXTENSIONS: [&str; 8] = ["hdb", "hsb", "mdb", "msb", "ndb", "ldb", "fp", "ign2"];

/// What an import kept and left out.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Stats {
    pub hashes: u64,
    pub sections: u64,
    pub patterns: u64,
    pub logical: u64,
    pub allowed: u64,
    /// Lines of each kind that could not be honoured.
    pub left_out_hashes: u64,
    pub left_out_patterns: u64,
    pub left_out_logical: u64,
    /// Skipped on purpose: optional-threat names and the source's own ignore list.
    pub ignored: u64,
}

/// Family part of a signature name: `Win.Trojan.Agent-1435085-0` -> `Win.Trojan.Agent`.
pub fn family(name: &str) -> &str {
    let mut n = name;
    for _ in 0..2 {
        match n.rsplit_once('-') {
            Some((head, tail)) if !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()) => {
                n = head
            }
            _ => break,
        }
    }
    n
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

fn hex(c: u8) -> Option<u8> {
    (c as char).to_digit(16).map(|d| d as u8)
}

/// A hex pattern body. None: alternatives, negations or anything else this engine does not do.
pub fn body(s: &str) -> Option<Vec<Tok>> {
    let b = s.as_bytes();
    let (mut i, mut toks) = (0, Vec::<Tok>::new());
    let any = |toks: &mut Vec<Tok>, n: u32| match toks.last_mut() {
        Some(Tok::Any(m)) => *m += n,
        _ => toks.push(Tok::Any(n)),
    };
    while i < b.len() {
        match b[i] {
            b'*' => {
                toks.push(Tok::Gap {
                    min: 0,
                    max: u32::MAX,
                });
                i += 1;
            }
            b'{' => {
                let end = i + s[i..].find('}')?;
                let inner = &s[i + 1..end];
                match inner.split_once('-') {
                    None => any(&mut toks, inner.parse().ok()?),
                    Some(("", hi)) => toks.push(Tok::Gap {
                        min: 0,
                        max: hi.parse().ok()?,
                    }),
                    Some((lo, "")) => toks.push(Tok::Gap {
                        min: lo.parse().ok()?,
                        max: u32::MAX,
                    }),
                    Some((lo, hi)) => toks.push(Tok::Gap {
                        min: lo.parse().ok()?,
                        max: hi.parse().ok()?,
                    }),
                }
                i = end + 1;
            }
            _ => {
                let (x, y) = (*b.get(i)?, *b.get(i + 1)?);
                i += 2;
                match (x, y) {
                    (b'?', b'?') => any(&mut toks, 1),
                    (b'?', y) => toks.push(Tok::Nibble {
                        mask: 0x0f,
                        value: hex(y)?,
                    }),
                    (x, b'?') => toks.push(Tok::Nibble {
                        mask: 0xf0,
                        value: hex(x)? << 4,
                    }),
                    (x, y) => {
                        let v = hex(x)? << 4 | hex(y)?;
                        match toks.last_mut() {
                            Some(Tok::Lit(l)) => l.push(v),
                            _ => toks.push(Tok::Lit(vec![v])),
                        }
                    }
                }
            }
        }
    }
    Some(toks)
}

fn place(s: &str) -> Option<Place> {
    let (off, shift) = match s.split_once(',') {
        Some((o, m)) => (o, m.parse().ok()?),
        None => (s, 0),
    };
    if off == "*" {
        return Some(Place::Anywhere);
    }
    if let Some(back) = off.strip_prefix("EOF-") {
        return Some(Place::End {
            back: back.parse().ok()?,
            shift,
        });
    }
    // Entry-point and section offsets need an unpacked view; the bytes are matched anywhere.
    if off.starts_with("EP") || off.starts_with('S') {
        return Some(Place::Anywhere);
    }
    Some(Place::Start {
        at: off.parse().ok()?,
        shift,
    })
}

fn target(s: &str) -> Option<u32> {
    let k = match s.parse::<u32>().ok()? {
        0 => return Some(0),
        1 => Kind::Pe,
        2 => Kind::Ole2,
        5 => Kind::Graphics,
        6 => Kind::Elf,
        9 => Kind::MachO,
        10 => Kind::Pdf,
        11 => Kind::Swf,
        12 => Kind::Java,
        // Normalized text, mail and the like are matched on a rewritten copy this engine lacks.
        _ => return None,
    };
    Some(k as u32)
}

/// One name per line, `name` or `name:hash`: the source's own list of retired rules.
fn retired(text: &str) -> std::collections::HashSet<&str> {
    text.lines()
        .map(|l| l.split(':').next().unwrap_or(l))
        .collect()
}

/// `(file extension, text)` for the signature lists of one source -> [`Builder`].
pub fn import(b: &mut Builder, files: &[(&str, &str)]) -> Stats {
    let mut st = Stats::default();
    let mut skip = std::collections::HashSet::new();
    for (ext, text) in files {
        if *ext == "ign2" {
            skip.extend(retired(text));
        }
    }
    let wanted = |name: &str, st: &mut Stats| {
        let ok = !name.starts_with("PUA.") && !skip.contains(family(name)) && !skip.contains(name);
        st.ignored += u64::from(!ok);
        ok
    };
    for (ext, text) in files {
        for l in text
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
        {
            match *ext {
                "hdb" | "hsb" => {
                    let f: Vec<&str> = l.split(':').collect();
                    let table = |h: &str| match h.len() {
                        32 => Some(Table::Md5),
                        64 => Some(Table::Sha256),
                        _ => None,
                    };
                    match (f.as_slice(), f.first().and_then(|h| table(h))) {
                        ([h, size, name, ..], Some(t)) => {
                            if !wanted(name, &mut st) {
                                continue;
                            }
                            match (size.parse::<u32>(), unhex(h)) {
                                (Ok(s), Some(d)) => {
                                    b.hash(t, s, &d, family(name));
                                    st.hashes += 1;
                                }
                                _ => st.left_out_hashes += 1,
                            }
                        }
                        _ => st.left_out_hashes += 1,
                    }
                }
                "mdb" | "msb" => {
                    let f: Vec<&str> = l.split(':').collect();
                    let Some(&[size, h, name]) =
                        f.get(..3).map(|s| <&[&str; 3]>::try_from(s).expect("3"))
                    else {
                        st.left_out_hashes += 1;
                        continue;
                    };
                    if !wanted(name, &mut st) {
                        continue;
                    }
                    let t = match h.len() {
                        32 => Table::SectionMd5,
                        64 => Table::SectionSha256,
                        _ => {
                            st.left_out_hashes += 1;
                            continue;
                        }
                    };
                    match (size.parse::<u32>(), unhex(h)) {
                        (Ok(s), Some(d)) => {
                            b.hash(t, s, &d, family(name));
                            st.sections += 1;
                        }
                        _ => st.left_out_hashes += 1,
                    }
                }
                "fp" => {
                    let f: Vec<&str> = l.split(':').collect();
                    if let ([h, size, name, ..], Some(d)) =
                        (f.as_slice(), f.first().and_then(|h| unhex(h)))
                    {
                        let t = if h.len() == 64 {
                            Table::AllowSha256
                        } else {
                            Table::AllowMd5
                        };
                        if let Ok(s) = size.parse::<u32>() {
                            b.hash(t, s, &d, name);
                            st.allowed += 1;
                        }
                    }
                }
                "ndb" => {
                    let f: Vec<&str> = l.splitn(6, ':').collect();
                    let [name, tgt, off, sig, ..] = f.as_slice() else {
                        st.left_out_patterns += 1;
                        continue;
                    };
                    if !wanted(name, &mut st) {
                        continue;
                    }
                    let p = (|| {
                        Some(Pattern {
                            kinds: target(tgt)?,
                            place: place(off)?,
                            toks: body(sig)?,
                            id: 0,
                            part: 0,
                        })
                    })();
                    if p.is_some_and(|p| b.rule(family(name), vec![p])) {
                        st.patterns += 1;
                    } else {
                        st.left_out_patterns += 1;
                    }
                }
                "ldb" => {
                    let f: Vec<&str> = l.split(';').collect();
                    let Some(name) = f.first() else { continue };
                    if !wanted(name, &mut st) {
                        continue;
                    }
                    if logical(&f).is_some_and(|p| b.rule(family(name), p)) {
                        st.logical += 1;
                    } else {
                        st.left_out_logical += 1;
                    }
                }
                _ => {}
            }
        }
    }
    st
}

/// A logical line that is a plain AND of all its hex parts, with a known target.
fn logical(f: &[&str]) -> Option<Vec<Pattern>> {
    let [_, meta, expr, subs @ ..] = f else {
        return None;
    };
    let mut kinds = None;
    for kv in meta.split(',') {
        let (k, v) = kv.split_once(':')?;
        match k {
            "Engine" => {}
            "Target" => kinds = Some(target(v)?),
            _ => return None,
        }
    }
    let plain = expr.trim().trim_start_matches('(').trim_end_matches(')');
    let ids: Vec<usize> = plain
        .split('&')
        .map(|t| t.trim().parse().ok())
        .collect::<Option<_>>()?;
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    if sorted != (0..subs.len()).collect::<Vec<_>>() {
        return None;
    }
    subs.iter()
        .map(|s| {
            Some(Pattern {
                toks: body(s)?,
                place: Place::Anywhere,
                kinds: kinds?,
                id: 0,
                part: 0,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    #[test]
    fn family_names_lose_their_serial() {
        for (n, f) in [
            ("Win.Trojan.Agent-1435085-0", "Win.Trojan.Agent"),
            ("Win.Trojan.Hotkey-1", "Win.Trojan.Hotkey"),
            ("Eicar-Test-Signature", "Eicar-Test-Signature"),
            ("Pdf.Dropper.Agent-8087615-0", "Pdf.Dropper.Agent"),
        ] {
            assert_eq!(family(n), f);
        }
    }

    #[test]
    fn hex_bodies_table() {
        let l = |b: &[u8]| Tok::Lit(b.to_vec());
        assert_eq!(body("4142"), Some(vec![l(b"AB")]));
        assert_eq!(
            body("41??????42"),
            Some(vec![l(b"A"), Tok::Any(3), l(b"B")])
        );
        assert_eq!(
            body("41*42"),
            Some(vec![
                l(b"A"),
                Tok::Gap {
                    min: 0,
                    max: u32::MAX
                },
                l(b"B")
            ])
        );
        assert_eq!(
            body("41{3}42{-4}43{1-2}44{5-}45"),
            Some(vec![
                l(b"A"),
                Tok::Any(3),
                l(b"B"),
                Tok::Gap { min: 0, max: 4 },
                l(b"C"),
                Tok::Gap { min: 1, max: 2 },
                l(b"D"),
                Tok::Gap {
                    min: 5,
                    max: u32::MAX
                },
                l(b"E"),
            ])
        );
        assert_eq!(
            body("4?"),
            Some(vec![Tok::Nibble {
                mask: 0xf0,
                value: 0x40
            }])
        );
        assert_eq!(
            body("?4"),
            Some(vec![Tok::Nibble {
                mask: 0x0f,
                value: 4
            }])
        );
        for bad in ["(41|42)", "!(41)", "41[1-2]42", "4", "zz", "41{", "41{x}"] {
            assert_eq!(body(bad), None, "{bad}");
        }
    }

    #[test]
    fn places_table() {
        assert_eq!(place("*"), Some(Place::Anywhere));
        assert_eq!(place("100"), Some(Place::Start { at: 100, shift: 0 }));
        assert_eq!(place("100,8"), Some(Place::Start { at: 100, shift: 8 }));
        assert_eq!(place("EOF-20"), Some(Place::End { back: 20, shift: 0 }));
        assert_eq!(place("EP+4,8"), Some(Place::Anywhere));
        assert_eq!(place("VI"), None);
    }

    const EICAR_MD5: &str = "44d88612fea8a8f36de82e1278abb02f";
    const SIG: &str = "c0ffee11deadbeef22ab33cd44ef55aa66bb77cc";

    #[test]
    fn an_import_keeps_what_it_can_honour_and_counts_the_rest() {
        let mut b = Builder::default();
        let hdb =
            format!("{EICAR_MD5}:68:Eicar-Test-Signature\nnothex:68:Bad\n{EICAR_MD5}:*:Any\n");
        let mdb = format!("512:{EICAR_MD5}:Win.Trojan.A-1-0\n");
        let ndb = format!(
            "Win.Trojan.B-2-0:1:*:{SIG}\nHtml.C-3-0:3:*:{SIG}\nD-4:0:*:(41|42)\nE-5:0:*:4141\nPUA.Win.F-6:0:*:{SIG}\nOld.G-7:0:*:{SIG}\n"
        );
        let ldb = format!(
            "Win.Trojan.H-8-0;Engine:51-255,Target:1;(0&1);{SIG};0102030405060708\nI-9;Engine:51-255,Target:7;0;{SIG}\nJ-10;Target:1;0|1;{SIG};{SIG}\nK-11;Target:1,FileSize:1-2;0;{SIG}\n"
        );
        let st = import(
            &mut b,
            &[
                ("ign2", "Old.G-7\n"),
                ("hdb", &hdb),
                ("mdb", &mdb),
                ("ndb", &ndb),
                ("ldb", &ldb),
                ("fp", &format!("{EICAR_MD5}:70:sid-1\n")),
            ],
        );
        assert_eq!(
            st,
            Stats {
                hashes: 1,
                sections: 1,
                patterns: 1,
                logical: 1,
                allowed: 1,
                left_out_hashes: 2,
                left_out_patterns: 3,
                left_out_logical: 3,
                ignored: 2,
            }
        );
        let d = Db::from_bytes(b.build(1, 1)).unwrap();
        assert_eq!(d.counts().1, 2);
        let md5 = unhex(EICAR_MD5).unwrap();
        assert_eq!(
            d.lookup(Table::Md5, 68, &md5).as_deref(),
            Some("Eicar-Test-Signature")
        );
        assert_eq!(
            d.lookup(Table::SectionMd5, 512, &md5).as_deref(),
            Some("Win.Trojan.A")
        );
        assert!(d.lookup(Table::AllowMd5, 70, &md5).is_some());
    }
}

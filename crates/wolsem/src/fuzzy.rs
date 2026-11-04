// SPDX-License-Identifier: BSD-2-Clause
//! "Did you mean": a query word that is in no title or text is replaced by the nearest word that
//! is (Damerau-Levenshtein distance 1 for short words, 2 for long ones).

/// Edit distance between two words (insert, delete, replace, swap of neighbours), None above `max`.
pub(crate) fn distance(a: &str, b: &str, max: usize) -> Option<usize> {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a.len().abs_diff(b.len()) > max {
        return None;
    }
    let mut rows = vec![vec![0usize; b.len() + 1]; 3];
    for (j, c) in rows[0].iter_mut().enumerate() {
        *c = j;
    }
    let (mut prev2, mut prev, mut cur) = (2, 0, 1);
    for i in 1..=a.len() {
        rows[cur][0] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut d = (rows[prev][j] + 1)
                .min(rows[cur][j - 1] + 1)
                .min(rows[prev][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d = d.min(rows[prev2][j - 2] + 1);
            }
            rows[cur][j] = d;
        }
        (prev2, prev, cur) = (prev, cur, prev2);
    }
    let d = rows[prev][b.len()];
    (d <= max).then_some(d)
}

/// The nearest of `words` to `w` within the allowed distance, if `w` is not itself one of them (or,
/// for the `last` word of a query, the start of one).
pub(crate) fn nearest<'a>(
    words: impl Iterator<Item = &'a str> + Clone,
    w: &str,
    last: bool,
) -> Option<&'a str> {
    let n = w.chars().count();
    if !(3..=32).contains(&n) {
        return None;
    }
    // The word still being typed (the last) may be the start of a longer one.
    if words.clone().any(|x| x == w || (last && x.starts_with(w))) {
        return None;
    }
    let max = if n >= 7 { 2 } else { 1 };
    let mut best: Option<(usize, &str)> = None;
    for x in words {
        if let Some(d) = distance(w, x, max)
            && best.is_none_or(|(bd, bx)| d < bd || (d == bd && x.len() < bx.len()))
        {
            best = Some((d, x));
        }
    }
    best.map(|b| b.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typos_are_one_or_two_edits() {
        assert_eq!(distance("pasword", "password", 2), Some(1));
        assert_eq!(distance("blutooth", "bluetooth", 2), Some(1));
        assert_eq!(distance("wifi", "wi-fi", 1), Some(1));
        assert_eq!(distance("teh", "the", 1), Some(1), "a swap is one edit");
        assert_eq!(distance("sound", "mouse", 2), None);
    }

    #[test]
    fn a_known_word_or_a_prefix_is_left_alone() {
        let words = ["firewall", "password", "sound"];
        assert_eq!(
            nearest(words.iter().copied(), "passwrd", true),
            Some("password")
        );
        assert_eq!(
            nearest(words.iter().copied(), "fire", true),
            None,
            "a prefix is not a typo"
        );
        assert_eq!(nearest(words.iter().copied(), "xyzzy", true), None);
        assert_eq!(nearest(words.iter().copied(), "up", true), None);
    }
}

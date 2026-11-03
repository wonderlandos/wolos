// SPDX-License-Identifier: BSD-2-Clause
//! CLDR cardinal plural rules for whole numbers, for the ten languages (CLDR 46, `plurals.xml`).
//! A count picks a [`Plural`] category; a plural key has one form per category its language
//! uses ([`categories`]). Only integers: a count of files, minutes or updates is whole.
use super::Lang;

/// A CLDR plural category. Table order: the form of category `c` is at index `c.index()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Plural {
    /// Zero (no language here uses it for integers; kept for the format).
    Zero,
    /// One ("1 file"; in French and Portuguese also 0, in Tagalog most counts).
    One,
    /// Two (no language here uses it; kept for the format).
    Two,
    /// Few (no language here uses it; kept for the format).
    Few,
    /// Many (Spanish, French and Portuguese for whole millions: "1 000 000 de fichiers").
    Many,
    /// Other: every count the rules above do not take.
    Other,
}

impl Plural {
    /// Every category, in table order.
    pub const ALL: [Plural; 6] = [
        Plural::Zero,
        Plural::One,
        Plural::Two,
        Plural::Few,
        Plural::Many,
        Plural::Other,
    ];

    /// Position in [`Plural::ALL`] (the form slot in a table).
    pub fn index(self) -> usize {
        Plural::ALL.iter().position(|p| *p == self).unwrap_or(5)
    }

    /// The CLDR name, also the TSV key suffix (`files_count#one`).
    pub fn name(self) -> &'static str {
        const NAMES: [&str; 6] = ["zero", "one", "two", "few", "many", "other"];
        NAMES[self.index()]
    }

    /// The category a CLDR name names; None for any other text.
    pub fn from_name(s: &str) -> Option<Plural> {
        Plural::ALL.into_iter().find(|p| p.name() == s)
    }
}

/// The categories `l` uses for whole numbers, `other` last. A plural key in `l` has a form for each.
pub fn categories(l: Lang) -> &'static [Plural] {
    const ONE_OTHER: [Plural; 2] = [Plural::One, Plural::Other];
    const ONE_MANY_OTHER: [Plural; 3] = [Plural::One, Plural::Many, Plural::Other];
    const OTHER: [Plural; 1] = [Plural::Other];
    match l {
        Lang::En | Lang::De | Lang::Tl | Lang::Xx => &ONE_OTHER,
        Lang::PtBr | Lang::Es | Lang::Fr => &ONE_MANY_OTHER,
        Lang::Ja | Lang::Ko | Lang::ZhHans | Lang::Id => &OTHER,
    }
}

/// The category of the count `n` in `l`.
pub fn category(l: Lang, n: u64) -> Plural {
    // es, fr, pt: many = e = 0 and i != 0 and i % 1000000 = 0 and v = 0.
    let million = n != 0 && n.is_multiple_of(1_000_000);
    match l {
        // one: i = 1 and v = 0.
        Lang::En | Lang::De | Lang::Xx => {
            if n == 1 {
                Plural::One
            } else {
                Plural::Other
            }
        }
        // one: n = 1.
        Lang::Es => {
            if n == 1 {
                Plural::One
            } else if million {
                Plural::Many
            } else {
                Plural::Other
            }
        }
        // one: i = 0,1.
        Lang::Fr | Lang::PtBr => {
            if n <= 1 {
                Plural::One
            } else if million {
                Plural::Many
            } else {
                Plural::Other
            }
        }
        // one: v = 0 and i = 1,2,3 or v = 0 and i % 10 != 4,6,9.
        Lang::Tl => {
            if matches!(n, 1..=3) || ![4, 6, 9].contains(&(n % 10)) {
                Plural::One
            } else {
                Plural::Other
            }
        }
        Lang::Ja | Lang::Ko | Lang::ZhHans | Lang::Id => Plural::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_pick_the_cldr_category() {
        use Plural::{Many, One, Other};
        let cases: [(Lang, u64, Plural); 22] = [
            (Lang::En, 0, Other),
            (Lang::En, 1, One),
            (Lang::En, 2, Other),
            (Lang::De, 1, One),
            (Lang::Fr, 0, One),
            (Lang::Fr, 1, One),
            (Lang::Fr, 2, Other),
            (Lang::Fr, 1_000_000, Many),
            (Lang::PtBr, 0, One),
            (Lang::PtBr, 2_000_000, Many),
            (Lang::Es, 0, Other),
            (Lang::Es, 1, One),
            (Lang::Es, 1_000_000, Many),
            (Lang::Es, 1_000_001, Other),
            (Lang::Tl, 0, One),
            (Lang::Tl, 4, Other),
            (Lang::Tl, 6, Other),
            (Lang::Tl, 11, One),
            (Lang::Tl, 19, Other),
            (Lang::Ja, 1, Other),
            (Lang::ZhHans, 1, Other),
            (Lang::Id, 1, Other),
        ];
        for (l, n, want) in cases {
            assert_eq!(category(l, n), want, "{l:?} {n}");
            assert!(categories(l).contains(&want), "{l:?} uses {want:?}");
        }
    }

    #[test]
    fn every_language_ends_its_categories_with_other() {
        for l in Lang::ALL {
            assert_eq!(categories(l).last(), Some(&Plural::Other));
        }
        for p in Plural::ALL {
            assert_eq!(Plural::from_name(p.name()), Some(p));
        }
    }
}

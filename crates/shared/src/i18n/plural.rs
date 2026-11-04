// SPDX-License-Identifier: BSD-2-Clause
//! CLDR cardinal plural rules for whole numbers, for the seven languages (CLDR 46, `plurals.xml`).
//! A count picks a [`Plural`] category; a plural key has one form per category its language
//! uses ([`categories`]). Only integers: a count of files, minutes or updates is whole.
use super::Lang;

/// A CLDR plural category. Table order: the form of category `c` is at index `c.index()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Plural {
    /// Zero (no language here uses it for integers; kept for the format).
    Zero,
    /// One ("1 file"; in French and Portuguese also 0, in Russian 21, 31 too).
    One,
    /// Two (no language here uses it; kept for the format).
    Two,
    /// Few (Russian 2-4, 22-24: "2 файла").
    Few,
    /// Many (French, Italian and Portuguese for whole millions: "1 000 000 de fichiers"; Russian
    /// 0, 5-20, 25-30: "5 файлов").
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
    const ONE_FEW_MANY_OTHER: [Plural; 4] = [Plural::One, Plural::Few, Plural::Many, Plural::Other];
    const OTHER: [Plural; 1] = [Plural::Other];
    match l {
        Lang::En | Lang::De | Lang::Xx => &ONE_OTHER,
        Lang::PtBr | Lang::Fr | Lang::It => &ONE_MANY_OTHER,
        Lang::Ru => &ONE_FEW_MANY_OTHER,
        Lang::Ja => &OTHER,
    }
}

/// The category of the count `n` in `l`.
pub fn category(l: Lang, n: u64) -> Plural {
    // fr, it, pt: many = e = 0 and i != 0 and i % 1000000 = 0 and v = 0.
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
        // one: i = 1 and v = 0.
        Lang::It => {
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
        // one: v = 0 and i % 10 = 1 and i % 100 != 11; few: i % 10 = 2..4 and i % 100 != 12..14;
        // many: every other whole number (other is for fractions only).
        Lang::Ru => {
            let (d, h) = (n % 10, n % 100);
            if d == 1 && h != 11 {
                Plural::One
            } else if (2..=4).contains(&d) && !(12..=14).contains(&h) {
                Plural::Few
            } else {
                Plural::Many
            }
        }
        Lang::Ja => Plural::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_pick_the_cldr_category() {
        use Plural::{Few, Many, One, Other};
        let cases: [(Lang, u64, Plural); 26] = [
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
            (Lang::It, 0, Other),
            (Lang::It, 1, One),
            (Lang::It, 1_000_000, Many),
            (Lang::It, 1_000_001, Other),
            (Lang::Ru, 0, Many),
            (Lang::Ru, 1, One),
            (Lang::Ru, 2, Few),
            (Lang::Ru, 4, Few),
            (Lang::Ru, 5, Many),
            (Lang::Ru, 11, Many),
            (Lang::Ru, 12, Many),
            (Lang::Ru, 21, One),
            (Lang::Ru, 22, Few),
            (Lang::Ru, 111, Many),
            (Lang::Ja, 1, Other),
            (Lang::Ja, 0, Other),
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

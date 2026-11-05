// SPDX-License-Identifier: BSD-2-Clause
//! The manual as items: the sections JSON the site exports (and the image ships next to the
//! table), `{"v":1,"model":"static-mrl","dim":256,"sections":[{"id":"/manual/page#anchor",
//! "title":"Page: Section","text":"...","vec":"<base64 of dim int8 + f32 scale>"}]}`. `vec` is
//! optional: without it the vectors are computed from the title and text with the table. A bare
//! array of the same objects is read too.

use crate::{Error, Index, ItemIn, Table, b64, err};

pub struct Corpus {
    pub ids: Vec<String>,
    pub titles: Vec<String>,
    pub texts: Vec<String>,
    /// The vectors the file brought (int8 and scale), per item.
    pub vecs: Vec<Option<(Vec<i8>, f32)>>,
}

impl Corpus {
    pub fn from_json(text: &str) -> Result<Corpus, Error> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| Error(e.to_string()))?;
        let list = v.as_array().or_else(|| {
            v.get("sections")
                .or_else(|| v.get("items"))
                .and_then(|s| s.as_array())
        });
        let Some(list) = list else {
            return err("no sections in the JSON");
        };
        let s = |x: &serde_json::Value, k: &str| {
            x.get(k).and_then(|s| s.as_str()).unwrap_or("").to_string()
        };
        let mut c = Corpus {
            ids: vec![],
            titles: vec![],
            texts: vec![],
            vecs: vec![],
        };
        for x in list {
            c.ids.push(s(x, "id"));
            c.titles.push(s(x, "title"));
            c.texts.push(s(x, "text"));
            c.vecs.push(
                x.get("vec")
                    .and_then(|b| b.as_str())
                    .and_then(b64::decode)
                    .and_then(split_vec),
            );
        }
        Ok(c)
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn items(&self) -> Vec<ItemIn<'_>> {
        (0..self.len())
            .map(|i| ItemIn {
                id: &self.ids[i],
                title: &self.titles[i],
                text: &self.texts[i],
            })
            .collect()
    }

    /// The index over these items: the file's vectors when every item has one of at least `dims`
    /// dimensions, else computed with `table`.
    pub fn index(&self, table: &Table, dims: usize) -> Index {
        let items = self.items();
        let have: Option<Vec<(Vec<i8>, f32)>> = self.vecs.iter().cloned().collect();
        match have {
            Some(v) if !v.is_empty() && v.iter().all(|x| x.0.len() >= dims) => {
                Index::with_vectors(&items, dims, &v)
                    .unwrap_or_else(|_| Index::build(table, &items, dims))
            }
            _ => Index::build(table, &items, dims),
        }
    }
}

/// `dim` int8 then an `f32` scale, little-endian.
fn split_vec(b: Vec<u8>) -> Option<(Vec<i8>, f32)> {
    let n = b.len().checked_sub(4)?;
    let s = f32::from_le_bytes([b[n], b[n + 1], b[n + 2], b[n + 3]]);
    Some((b[..n].iter().map(|x| i8::from_le_bytes([*x])).collect(), s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_json_is_read_with_or_without_vectors() {
        let json = concat!(
            r#"{"v":1,"sections":[{"id":"/manual/a#b","title":"A: B","text":"x y"},"#,
            r#"{"id":"c","title":"C","text":"","vec":"AAAAAAAAgD8="}]}"#
        );
        let c = Corpus::from_json(json).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(c.len(), 2);
        assert_eq!(c.ids[0], "/manual/a#b");
        assert!(c.vecs[0].is_none());
        assert_eq!(c.vecs[1], Some((vec![0, 0, 0, 0], 1.0)));
        assert!(Corpus::from_json("{}").is_err());
        assert!(Corpus::from_json("nope").is_err());
    }
}

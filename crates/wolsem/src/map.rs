// SPDX-License-Identifier: BSD-2-Clause
//! The query map between languages: the orthogonal matrix `W` that turns the vector of a text
//! in language L into the vector of the same text in English (orthogonal Procrustes, Schönemann
//! 1966; the cross-lingual use is Smith et al. 2017). Trained on aligned pairs, here the rows of
//! the per-language copy tables (`locale/<unit>/<lang>.tsv` against `en.tsv`). A table that is
//! already multilingual needs none; this is for a table whose languages sit in different places.

use crate::table::normalise;
use crate::{Error, err};

/// `W` (row-major, `dim x dim`).
#[derive(Clone, Debug, PartialEq)]
pub struct Procrustes {
    dim: usize,
    w: Vec<f32>,
}

impl Procrustes {
    pub fn identity(dim: usize) -> Procrustes {
        let mut w = vec![0f32; dim * dim];
        for i in 0..dim {
            w[i * dim + i] = 1.0;
        }
        Procrustes { dim, w }
    }

    pub fn dim(&self) -> usize {
        self.dim
    }

    /// The `W` minimising the sum of `|W x - y|^2` over `pairs` of `(x in L, y in English)`
    /// unit vectors, kept near the identity where the pairs say nothing: `ridge` (0.1 is a good
    /// start) adds that share of the mean singular value as a pull towards `I`, and also makes
    /// fewer pairs than dimensions work.
    pub fn train(
        pairs: &[(Vec<f32>, Vec<f32>)],
        dim: usize,
        ridge: f32,
    ) -> Result<Procrustes, Error> {
        if pairs.is_empty() || pairs.iter().any(|(x, y)| x.len() != dim || y.len() != dim) {
            return err("Procrustes needs pairs of vectors of the table's dimension");
        }
        // M = sum y x^T, kept as columns: col[b][a] = sum y[a] x[b].
        let mut cols = vec![vec![0f64; dim]; dim];
        for (x, y) in pairs {
            for (b, xb) in x.iter().enumerate() {
                for (a, ya) in y.iter().enumerate() {
                    cols[b][a] += f64::from(*xb) * f64::from(*ya);
                }
            }
        }
        let mean_sv = cols
            .iter()
            .map(|c| c.iter().map(|v| v * v).sum::<f64>())
            .sum::<f64>()
            .sqrt()
            / (dim as f64).sqrt();
        for (i, c) in cols.iter_mut().enumerate() {
            c[i] += f64::from(ridge) * mean_sv;
        }
        // One-sided Jacobi SVD: rotate column pairs of A (= M) until they are orthogonal; V
        // collects the rotations; then A V = U S and W = U V^T.
        let mut v = vec![vec![0f64; dim]; dim];
        for (i, c) in v.iter_mut().enumerate() {
            c[i] = 1.0;
        }
        for _sweep in 0..40 {
            let mut rotated = false;
            for p in 0..dim {
                for q in p + 1..dim {
                    let (mut alpha, mut beta, mut gamma) = (0f64, 0f64, 0f64);
                    for (a, b) in cols[p].iter().zip(&cols[q]) {
                        alpha += a * a;
                        beta += b * b;
                        gamma += a * b;
                    }
                    if gamma.abs() <= 1e-12 * (alpha * beta).sqrt() || gamma == 0.0 {
                        continue;
                    }
                    rotated = true;
                    let zeta = (beta - alpha) / (2.0 * gamma);
                    let t = zeta.signum() / (zeta.abs() + (1.0 + zeta * zeta).sqrt());
                    let c = 1.0 / (1.0 + t * t).sqrt();
                    let s = c * t;
                    for m in [&mut cols, &mut v] {
                        let (head, tail) = m.split_at_mut(q);
                        for (a, b) in head[p].iter_mut().zip(tail[0].iter_mut()) {
                            let (x, y) = (*a, *b);
                            *a = c * x - s * y;
                            *b = s * x + c * y;
                        }
                    }
                }
            }
            if !rotated {
                break;
            }
        }
        let mut w = vec![0f32; dim * dim];
        for j in 0..dim {
            let sigma = cols[j].iter().map(|x| x * x).sum::<f64>().sqrt();
            if sigma < 1e-12 {
                return err("Procrustes: pairs are degenerate, raise the ridge");
            }
            for a in 0..dim {
                let ua = cols[j][a] / sigma;
                for b in 0..dim {
                    w[a * dim + b] += (ua * v[j][b]) as f32;
                }
            }
        }
        Ok(Procrustes { dim, w })
    }

    /// `W x`, normalised; `x` of another length comes back unchanged.
    pub fn apply(&self, x: &[f32]) -> Vec<f32> {
        if x.len() > self.dim || x.is_empty() {
            return x.to_vec();
        }
        // A vector cut to fewer dimensions (MRL) uses the top-left block, then renormalises.
        let d = x.len();
        let y: Vec<f32> = (0..d)
            .map(|a| {
                self.w[a * self.dim..a * self.dim + d]
                    .iter()
                    .zip(x)
                    .map(|(w, v)| w * v)
                    .sum()
            })
            .collect();
        normalise(y).unwrap_or_else(|| x.to_vec())
    }

    /// `WPRC`, version 1, dim as `u16`, then `dim x dim` little-endian `f32`.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = b"WPRC".to_vec();
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&u16::try_from(self.dim).unwrap_or(0).to_le_bytes());
        for x in &self.w {
            b.extend_from_slice(&x.to_le_bytes());
        }
        b
    }

    pub fn from_bytes(b: &[u8]) -> Result<Procrustes, Error> {
        if b.get(..4) != Some(b"WPRC") || b.get(4..6) != Some(&[1, 0]) {
            return err("not a Procrustes map");
        }
        let dim = usize::from(u16::from_le_bytes([
            *b.get(6).unwrap_or(&0),
            *b.get(7).unwrap_or(&0),
        ]));
        let body = b.get(8..).unwrap_or(&[]);
        if dim == 0 || body.len() != dim * dim * 4 {
            return err("Procrustes map has the wrong size");
        }
        let w = body
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| f32::from_le_bytes(*c))
            .collect();
        Ok(Procrustes { dim, w })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic spread of unit vectors (no rand crate).
    fn unit(seed: u32, dim: usize) -> Vec<f32> {
        let mut s = seed.wrapping_mul(2_654_435_761).wrapping_add(12345);
        let v: Vec<f32> = (0..dim)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                (s % 2001) as f32 / 1000.0 - 1.0
            })
            .collect();
        normalise(v).unwrap_or_default()
    }

    fn rotate(v: &[f32]) -> Vec<f32> {
        // A fixed orthogonal map: reverse the vector and flip alternate signs.
        v.iter()
            .rev()
            .enumerate()
            .map(|(i, x)| if i % 2 == 0 { *x } else { -*x })
            .collect()
    }

    #[test]
    fn a_rotation_is_recovered_from_pairs() {
        let dim = 24;
        let pairs: Vec<_> = (0..60)
            .map(|i| (unit(i, dim), rotate(&unit(i, dim))))
            .collect();
        let p = Procrustes::train(&pairs, dim, 0.0).unwrap_or_else(|e| panic!("{e}"));
        let x = unit(999, dim);
        let got = p.apply(&x);
        let want = rotate(&x);
        let cos: f32 = got.iter().zip(&want).map(|(a, b)| a * b).sum();
        assert!(cos > 0.999, "cos {cos}");
    }

    #[test]
    fn few_pairs_with_a_ridge_stay_close_to_the_identity() {
        let dim = 24;
        let pairs: Vec<_> = (0..5).map(|i| (unit(i, dim), unit(i, dim))).collect();
        let p = Procrustes::train(&pairs, dim, 0.1).unwrap_or_else(|e| panic!("{e}"));
        let x = unit(77, dim);
        let cos: f32 = p.apply(&x).iter().zip(&x).map(|(a, b)| a * b).sum();
        assert!(cos > 0.95, "cos {cos}");
        assert!(Procrustes::train(&[], dim, 0.1).is_err());
    }

    #[test]
    fn a_map_survives_its_bytes() {
        let p = Procrustes::identity(8);
        assert_eq!(Procrustes::from_bytes(&p.to_bytes()), Ok(p));
        assert!(Procrustes::from_bytes(b"WPRC").is_err());
        let x = unit(3, 4);
        assert_eq!(
            Procrustes::identity(8).apply(&x),
            x,
            "a cut vector through the identity"
        );
    }
}

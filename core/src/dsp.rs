//! Curve computations: biquad filter response (same formulas as
//! biquad.c, in f64), interpolation, alignment, CSV reading.

use serde::{Deserialize, Serialize};

use crate::engine::{F_HI, F_LO, FS};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Enum)]
pub enum FilterKind {
    Peak,
    LowShelf,
    HighShelf,
}

impl FilterKind {
    /// Value of the C Type enum.
    pub fn c_type(self) -> i32 {
        match self {
            FilterKind::Peak => 0,
            FilterKind::LowShelf => 1,
            FilterKind::HighShelf => 2,
        }
    }
    /// Name used by the binary and by Equalizer APO.
    pub fn apo_name(self) -> &'static str {
        match self {
            FilterKind::Peak => "PK",
            FilterKind::LowShelf => "LSC",
            FilterKind::HighShelf => "HSC",
        }
    }
}

struct Coefs {
    b0: f64,
    b1: f64,
    b2: f64,
    a0: f64,
    a1: f64,
    a2: f64,
}

fn coefs(kind: FilterKind, f0: f64, gain: f64, q: f64, fs: f64) -> Coefs {
    let a = 10f64.powf(gain / 40.0);
    let w0 = 2.0 * std::f64::consts::PI / fs * f0;
    let (cos_w, sin_w) = (w0.cos(), w0.sin());
    let alpha = sin_w * 0.5 / q;
    match kind {
        FilterKind::Peak => {
            let ra = 1.0 / a;
            Coefs {
                b0: a * alpha + 1.0,
                b1: -2.0 * cos_w,
                b2: -a * alpha + 1.0,
                a0: (a + alpha) * ra,
                a1: -2.0 * cos_w,
                a2: (a - alpha) * ra,
            }
        }
        FilterKind::LowShelf => {
            let (p1, m1) = (a + 1.0, a - 1.0);
            let k = 2.0 * a.sqrt() * alpha;
            Coefs {
                b0: a * (-cos_w * m1 + k + p1),
                b1: 2.0 * a * (-cos_w * p1 + m1),
                b2: a * (-cos_w * m1 - k + p1),
                a0: cos_w * m1 + k + p1,
                a1: -2.0 * cos_w * p1 - 2.0 * m1,
                a2: cos_w * m1 - k + p1,
            }
        }
        FilterKind::HighShelf => {
            let (p1, m1) = (a + 1.0, a - 1.0);
            let k = 2.0 * a.sqrt() * alpha;
            Coefs {
                b0: a * (cos_w * m1 + k + p1),
                b1: -2.0 * a * (cos_w * p1 + m1),
                b2: a * (cos_w * m1 - k + p1),
                a0: -cos_w * m1 + k + p1,
                a1: -2.0 * cos_w * p1 + 2.0 * m1,
                a2: -cos_w * m1 - k + p1,
            }
        }
    }
}

/// Adds to `y` the dB response of a filter at frequencies `f`.
pub fn add_response(kind: FilterKind, f0: f64, gain: f64, q: f64, f: &[f64], y: &mut [f64]) {
    let s = coefs(kind, f0, gain, q, FS);
    let b_x0 = (s.b0 + s.b1 + s.b2).powi(2);
    let b_x1 = -4.0 * (s.b0 * s.b1 + 4.0 * s.b0 * s.b2 + s.b1 * s.b2);
    let b_x2 = 16.0 * s.b0 * s.b2;
    let a_x0 = (s.a0 + s.a1 + s.a2).powi(2);
    let a_x1 = -4.0 * (s.a0 * s.a1 + 4.0 * s.a0 * s.a2 + s.a1 * s.a2);
    let a_x2 = 16.0 * s.a0 * s.a2;
    for (fk, yk) in f.iter().zip(y.iter_mut()) {
        let phi = (std::f64::consts::PI / FS * fk).sin().powi(2);
        let b = b_x0 + phi * (b_x1 + phi * b_x2);
        let a = a_x0 + phi * (a_x1 + phi * a_x2);
        *yk += 10.0 * (b / a).log10();
    }
}

/// Linear interpolation in log f; beyond the edges, the edge value (np.interp).
pub fn interp_log(freqs: &[f64], db: &[f64], f: f64) -> f64 {
    let n = freqs.len();
    if n == 0 {
        return 0.0;
    }
    if f <= freqs[0] {
        return db[0];
    }
    if f >= freqs[n - 1] {
        return db[n - 1];
    }
    // binary search for the segment
    let i = match freqs.binary_search_by(|x| x.partial_cmp(&f).unwrap()) {
        Ok(i) => return db[i],
        Err(i) => i,
    };
    let (f0, f1) = (freqs[i - 1].ln(), freqs[i].ln());
    let t = (f.ln() - f0) / (f1 - f0);
    db[i - 1] + t * (db[i] - db[i - 1])
}

pub fn resample(freqs: &[f64], db: &[f64], at: &[f64]) -> Vec<f64> {
    at.iter().map(|&f| interp_log(freqs, db, f)).collect()
}

/// Logarithmic grid of `n` points between 20 Hz and 20 kHz.
pub fn log_grid(n: usize) -> Vec<f64> {
    let n = n.max(2);
    let (l0, l1) = (F_LO.ln(), F_HI.ln());
    (0..n)
        .map(|k| (l0 + (l1 - l0) * k as f64 / (n - 1) as f64).exp())
        .collect()
}

/// Curve (frequency in Hz, level in dB), sorted by increasing frequency.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CurveData {
    pub freqs: Vec<f64>,
    pub db: Vec<f64>,
}

impl CurveData {
    pub fn new(mut pts: Vec<(f64, f64)>) -> CurveData {
        pts.retain(|(f, d)| f.is_finite() && d.is_finite() && *f > 0.0);
        pts.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        pts.dedup_by(|a, b| a.0 == b.0);
        CurveData {
            freqs: pts.iter().map(|p| p.0).collect(),
            db: pts.iter().map(|p| p.1).collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.freqs.is_empty()
    }

    pub fn at(&self, f: f64) -> f64 {
        interp_log(&self.freqs, &self.db, f)
    }

    pub fn resample(&self, at: &[f64]) -> Vec<f64> {
        resample(&self.freqs, &self.db, at)
    }

    /// Content fingerprint (FNV-1a over the rounded values), for duplicate detection.
    pub fn content_hash(&self) -> String {
        let mut h: u64 = 0xcbf29ce484222325;
        let mut eat = |v: f64| {
            let r = (v * 1e4).round() as i64;
            for b in r.to_le_bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
        };
        for (f, d) in self.freqs.iter().zip(&self.db) {
            eat(*f);
            eat(*d);
        }
        format!("{h:016x}")
    }

    /// CSV in AutoEq format (`frequency,raw`).
    pub fn to_csv(&self, offset: f64) -> String {
        let mut s = String::from("frequency,raw\n");
        for (f, d) in self.freqs.iter().zip(&self.db) {
            s.push_str(&format!("{},{}\n", fmt_num(*f, 2), fmt_num(d + offset, 3)));
        }
        s
    }
}

fn fmt_num(v: f64, decimals: usize) -> String {
    let s = format!("{v:.decimals$}");
    if s.contains('.') {
        let s = s.trim_end_matches('0').trim_end_matches('.');
        if s == "-0" { "0".into() } else { s.to_string() }
    } else {
        s
    }
}

/// Reads a CSV or measurement text: first two numeric columns,
/// `,` `;` tab or whitespace separator, header lines ignored.
/// Accepts a decimal comma when the separator is `;` or a tab.
pub fn parse_curve_text(text: &str) -> Result<CurveData, String> {
    let first_data = text
        .lines()
        .find(|l| l.trim().chars().next().is_some_and(|c| c.is_ascii_digit() || c == '.' || c == '-'))
        .unwrap_or("");
    let delim: Option<char> = if first_data.contains(';') {
        Some(';')
    } else if first_data.contains('\t') {
        Some('\t')
    } else if first_data.contains(',') {
        Some(',')
    } else {
        None
    };
    let decimal_comma = matches!(delim, Some(';') | Some('\t') | None)
        && first_data.contains(',')
        && !first_data.contains('.');

    let mut pts = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = match delim {
            Some(d) => line.split(d).collect(),
            None => line.split_whitespace().collect(),
        };
        if fields.len() < 2 {
            continue;
        }
        let num = |s: &str| {
            let s = s.trim().trim_matches('"');
            if decimal_comma {
                s.replace(',', ".").parse::<f64>().ok()
            } else {
                s.parse::<f64>().ok()
            }
        };
        if let (Some(f), Some(d)) = (num(fields[0]), num(fields[1])) {
            pts.push((f, d));
        }
    }
    let c = CurveData::new(pts);
    if c.freqs.len() < 2 {
        return Err("no usable numeric data (at least two frequency/dB points expected)".into());
    }
    Ok(c)
}

/// Offset `o` minimizing Σ (a + o − b)² over the points of `grid` in [lo, hi].
pub fn align_offset(a: &[f64], b: &[f64], grid: &[f64], lo: f64, hi: f64) -> f64 {
    let mut s = 0.0;
    let mut n = 0usize;
    for ((x, y), f) in a.iter().zip(b).zip(grid) {
        if *f >= lo && *f <= hi {
            s += y - x;
            n += 1;
        }
    }
    if n == 0 { 0.0 } else { s / n as f64 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine;

    #[test]
    fn response_matches_c() {
        let fc = engine::grid_c();
        let f: Vec<f64> = fc.iter().map(|&x| x as f64).collect();
        for kind in [FilterKind::Peak, FilterKind::LowShelf, FilterKind::HighShelf] {
            for &(f0, g, q) in &[(100.0, 6.0, 0.7), (3000.0, -8.5, 2.5), (12000.0, 4.0, 1.2), (19000.0, -3.0, 0.5)] {
                let c = engine::c_spectrum(kind, f0 as f32, g as f32, q as f32);
                let mut y = vec![0.0; f.len()];
                add_response(kind, f0, g, q, &f, &mut y);
                for (a, b) in c.iter().zip(&y) {
                    // the C code computes in f32: ~1e-3 dB difference in the bass
                    assert!((*a as f64 - b).abs() < 2e-2, "{kind:?} {f0} {g} {q}: {a} vs {b}");
                }
            }
        }
    }

    #[test]
    fn csv_variants() {
        let a = parse_curve_text("frequency,raw\n20,1.5\n30,2\n").unwrap();
        assert_eq!(a.freqs, vec![20.0, 30.0]);
        let b = parse_curve_text("Freq;dB\n20,5;1,5\n30;2\n").unwrap();
        assert_eq!(b.db, vec![1.5, 2.0]);
        assert_eq!(b.freqs, vec![20.5, 30.0]);
        let c = parse_curve_text("* header\n20 1\n40\t3\n").unwrap();
        assert_eq!(c.freqs.len(), 2);
        assert!(parse_curve_text("hello").is_err());
    }

    #[test]
    fn interp_edges() {
        let f = [100.0, 1000.0];
        let d = [0.0, 10.0];
        assert_eq!(interp_log(&f, &d, 10.0), 0.0);
        assert_eq!(interp_log(&f, &d, 1e5), 10.0);
        assert!((interp_log(&f, &d, 316.227766) - 5.0).abs() < 1e-6);
    }
}

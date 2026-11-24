//! EQ exchange formats: Equalizer APO parametric text,
//! GraphicEQ (Wavelet), filters CSV, and parsing of these formats.

use crate::dsp::{add_response, FilterKind};

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedFilter {
    pub kind: FilterKind,
    pub f0: f64,
    pub gain: f64,
    pub q: f64,
    pub enabled: bool,
}

#[derive(Clone, Debug, Default)]
pub struct ParsedEq {
    pub preamp: Option<f64>,
    pub filters: Vec<ParsedFilter>,
    /// lines not understood (unsupported filters, etc.)
    pub skipped: Vec<String>,
}

/// Equivalent of JS `Number.prototype.toFixed`: ties round away from zero
/// (Rust rounds half to even), so exports match JavaScript-based EQ tools digit for digit.
pub fn to_fixed(x: f64, d: usize) -> String {
    let m = x.abs() * 10f64.powi(d as i32);
    if m.fract() == 0.5 {
        let r = (m.trunc() + 1.0) / 10f64.powi(d as i32);
        return format!("{}{:.*}", if x < 0.0 { "-" } else { "" }, d, r);
    }
    format!("{:.*}", d, x)
}

/// "Filters" export (Equalizer APO): one line per filter, no trailing newline.
pub fn apo_text(filters: &[ParsedFilter], preamp: f64) -> String {
    let mut lines = vec![format!("Preamp: {} dB", to_fixed(preamp, 1))];
    for (i, f) in filters.iter().enumerate() {
        lines.push(format!(
            "Filter {}: {} {} Fc {} Hz Gain {} dB Q {}",
            i + 1,
            if f.enabled { "ON" } else { "OFF" },
            f.kind.apo_name(),
            to_fixed(f.f0, 0),
            to_fixed(f.gain, 2),
            to_fixed(f.q, 3)
        ));
    }
    lines.join("\n")
}

/// Exported preamp: −(maximum gain of the active filters between 5 Hz and 22 kHz) − 0.2 dB.
pub fn export_preamp(filters: &[ParsedFilter]) -> f64 {
    let (a, b) = (5f64.ln(), 22000f64.ln());
    let n = 1024;
    let f: Vec<f64> = (0..n).map(|i| (a + (b - a) / (n - 1) as f64 * i as f64).exp()).collect();
    let mut y = vec![0.0; n];
    for flt in filters.iter().filter(|x| x.enabled) {
        add_response(flt.kind, flt.f0, flt.gain, flt.q, &f, &mut y);
    }
    -y.iter().cloned().fold(f64::NEG_INFINITY, f64::max) - 0.2
}

/// AutoEq GraphicEQ frequencies (step of 1.0563, 127 points from 20 Hz to ~20 kHz).
pub fn graphic_eq_freqs() -> Vec<f64> {
    let step: f64 = 1.0563;
    let n = ((20000.0f64 / 20.0).ln() / step.ln()).ceil() as i32;
    let mut v: Vec<i64> = (0..n).map(|i| (20.0 * step.powi(i)) as i64).collect();
    v.dedup();
    v.into_iter().map(|x| x as f64).collect()
}

/// "Wavelet" export (GraphicEQ): response of the active filters,
/// brought below 0 dB (the maximum, at least 0, plus a 0.1 dB margin).
pub fn graphic_eq(filters: &[ParsedFilter]) -> String {
    let f = graphic_eq_freqs();
    let mut y = vec![0.0; f.len()];
    for flt in filters.iter().filter(|x| x.enabled) {
        add_response(flt.kind, flt.f0, flt.gain, flt.q, &f, &mut y);
    }
    let r = y.iter().cloned().fold(0.0, f64::max) + 0.1;
    let parts: Vec<String> = f.iter().zip(&y).map(|(f, v)| format!("{} {}", *f as i64, to_fixed(v - r, 3))).collect();
    format!("GraphicEQ: {}", parts.join("; "))
}

pub fn filters_csv(filters: &[ParsedFilter], preamp: f64) -> String {
    let mut s = format!("type,fc,gain,q,enabled\n# preamp {:.2}\n", preamp);
    for f in filters {
        s.push_str(&format!(
            "{},{:.2},{:.3},{:.4},{}\n",
            f.kind.apo_name(),
            f.f0,
            f.gain,
            f.q,
            if f.enabled { 1 } else { 0 }
        ));
    }
    s
}

fn kind_from_token(t: &str) -> Option<FilterKind> {
    match t.to_ascii_uppercase().as_str() {
        "PK" | "PEQ" | "PEAK" | "PEAKING" | "MODAL" => Some(FilterKind::Peak),
        "LSC" | "LS" | "LSQ" | "LOWSHELF" | "LOW_SHELF" => Some(FilterKind::LowShelf),
        "HSC" | "HS" | "HSQ" | "HIGHSHELF" | "HIGH_SHELF" => Some(FilterKind::HighShelf),
        _ => None,
    }
}

fn num(s: &str) -> Option<f64> {
    s.trim().trim_end_matches(['h', 'H', 'z', 'd', 'B']).replace(',', ".").parse().ok()
}

/// Parses a filters text: Equalizer APO (`Filter n: ON PK Fc … Gain … Q …`),
/// raw output of the autoeq binary (`PK f0 gain Q`), or CSV (`type,fc,gain,q[,enabled]`).
pub fn parse_filters(text: &str) -> ParsedEq {
    let mut out = ParsedEq::default();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("preamp:") {
            if let Some(v) = rest.split_whitespace().next().and_then(num) {
                out.preamp = Some(v);
            }
            continue;
        }
        if lower.starts_with("filter") {
            match parse_apo_line(line) {
                Some(Some(f)) => out.filters.push(f),
                Some(None) => {}
                None => out.skipped.push(line.to_string()),
            }
            continue;
        }
        // CSV or raw output
        let fields: Vec<&str> = if line.contains(',') && !line.contains(' ') || line.contains(';') {
            line.split([',', ';']).map(str::trim).collect()
        } else {
            line.split_whitespace().collect()
        };
        if let Some(kind) = fields.first().and_then(|t| kind_from_token(t)) {
            if fields.len() >= 4 {
                if let (Some(f0), Some(gain), Some(q)) = (num(fields[1]), num(fields[2]), num(fields[3])) {
                    let enabled = fields.get(4).map(|e| !matches!(e.trim(), "0" | "false" | "OFF" | "off")).unwrap_or(true);
                    out.filters.push(ParsedFilter { kind, f0, gain, q, enabled });
                    continue;
                }
            }
        } else if fields.len() == 2 && fields.iter().all(|x| num(x).is_some()) {
            // first line of the binary's output (amp max): ignored
            continue;
        } else if fields.first().is_some_and(|t| t.eq_ignore_ascii_case("type")) {
            continue; // CSV header
        }
        out.skipped.push(line.to_string());
    }
    out
}

/// `Some(Some(f))` filter read, `Some(None)` line deliberately ignored, `None` not understood.
fn parse_apo_line(line: &str) -> Option<Option<ParsedFilter>> {
    let body = match line.find(':') {
        Some(i) => &line[i + 1..],
        None => return None,
    };
    let toks: Vec<&str> = body.split_whitespace().collect();
    let mut i = 0;
    let mut enabled = true;
    if let Some(t) = toks.first() {
        match t.to_ascii_uppercase().as_str() {
            "ON" => i = 1,
            "OFF" => {
                enabled = false;
                i = 1
            }
            _ => {}
        }
    }
    let kind_tok = toks.get(i)?;
    if kind_tok.eq_ignore_ascii_case("none") {
        return Some(None);
    }
    let kind = kind_from_token(kind_tok)?;
    let (mut f0, mut gain, mut q) = (None, None, None);
    let mut j = i + 1;
    while j < toks.len() {
        let key = toks[j].to_ascii_lowercase();
        let val = toks.get(j + 1).and_then(|v| num(v));
        match key.as_str() {
            "fc" => f0 = val,
            "gain" => gain = val,
            "q" => q = val,
            "bw" => {
                // bandwidth in octaves -> Q
                q = val.map(|bw| {
                    let e = 2f64.powf(bw);
                    e.sqrt() / (e - 1.0)
                })
            }
            _ => {
                j += 1;
                continue;
            }
        }
        j += 2;
    }
    let q = q.unwrap_or(std::f64::consts::FRAC_1_SQRT_2);
    Some(Some(ParsedFilter { kind, f0: f0?, gain: gain.unwrap_or(0.0), q, enabled }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graphic_freqs() {
        let f = graphic_eq_freqs();
        assert_eq!(f.len(), 127);
        assert_eq!(f[0], 20.0);
        assert_eq!(*f.last().unwrap(), 19871.0);
    }

    #[test]
    fn roundtrip_apo() {
        let fl = vec![
            ParsedFilter { kind: FilterKind::LowShelf, f0: 105.0, gain: 4.5, q: 0.7, enabled: true },
            ParsedFilter { kind: FilterKind::Peak, f0: 3150.5, gain: -2.25, q: 2.0, enabled: false },
        ];
        let t = apo_text(&fl, -4.5);
        let p = parse_filters(&t);
        assert_eq!(p.preamp, Some(-4.5));
        assert_eq!(p.filters.len(), 2);
        assert_eq!(p.filters[1].enabled, false);
        assert_eq!(p.filters[1].f0, 3151.0); // Fc is rounded to the nearest hertz
        assert!(p.skipped.is_empty());
        assert!(t.contains("Filter 2: OFF PK Fc 3151 Hz Gain -2.25 dB Q 2.000"));
        assert!(!t.ends_with('\n'));
    }

    #[test]
    fn js_rounding() {
        assert_eq!(to_fixed(2.5, 0), "3");
        assert_eq!(to_fixed(-2.5, 0), "-3");
        assert_eq!(to_fixed(0.125, 2), "0.13");
        assert_eq!(to_fixed(1.234, 1), "1.2");
    }

    #[test]
    fn wavelet_below_zero() {
        let fl = vec![ParsedFilter { kind: FilterKind::Peak, f0: 1000.0, gain: 6.0, q: 1.0, enabled: true }];
        let g = graphic_eq(&fl);
        assert!(g.starts_with("GraphicEQ: 20 -6.0"), "{g}");
        assert!(g.contains("; 977 ") && !g.ends_with('\n'));
        assert!((export_preamp(&fl) + 6.2).abs() < 0.01);
    }

    #[test]
    fn other_formats() {
        let p = parse_filters("-1.2 3.4\nPK 1000.0 -3.0 1.41\nLSC 105 2 0.7\n");
        assert_eq!(p.filters.len(), 2);
        let p = parse_filters("type,fc,gain,q,enabled\nHSC,10000,-2,0.7,0\n");
        assert_eq!(p.filters.len(), 1);
        assert!(!p.filters[0].enabled);
        let p = parse_filters("Filter 1: ON LP Fc 100 Hz\nFilter 2: ON PK Fc 200 Hz Gain 1 dB BW Oct 1");
        assert_eq!(p.skipped.len(), 1);
        assert_eq!(p.filters.len(), 1);
    }
}

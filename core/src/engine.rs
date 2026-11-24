//! Calls the autoeq algorithm: built-in C variant (FFI, same process)
//! or external binary (same protocol as the autoeq binary: 384 float32
//! target values then 384 float32 measurement values on stdin).

use std::ffi::c_char;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Instant;

use crate::dsp::{interp_log, FilterKind};

/// Number of points of the algorithm's grid (enum K in lib.h).
pub const K: usize = 384;
pub const MAX_N: usize = 32;
pub const FS: f64 = 48000.0;
pub const F_LO: f64 = 20.0;
pub const F_HI: f64 = 20000.0;

#[repr(C)]
struct CResult {
    types: [i32; MAX_N],
    f0: [f32; MAX_N],
    gain: [f32; MAX_N],
    q: [f32; MAX_N],
    amp: f32,
    max: f32,
    mean: f32,
}

extern "C" {
    fn eqing_orig_autoeq_run(
        mode: c_char,
        n: i32,
        steps: i32,
        dst: *const f32,
        src: *const f32,
        out: *mut CResult,
    ) -> i32;
    fn eqing_orig_spectrum(t: i32, f0: f32, gain: f32, q: f32, fs: f32, f: *const f32, y: *mut f32);
    fn eqing_orig_k() -> i32;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Enum)]
pub enum Mode {
    InEar,
    OverEar,
    Precise,
}

impl Mode {
    pub fn letter(self) -> char {
        match self {
            Mode::InEar => 'i',
            Mode::OverEar => 'o',
            Mode::Precise => 'x',
        }
    }
    pub fn from_letter(c: &str) -> Option<Mode> {
        match c {
            "i" => Some(Mode::InEar),
            "o" => Some(Mode::OverEar),
            "x" => Some(Mode::Precise),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize, uniffi::Enum)]
pub enum Engine {
    /// autoeq-c "orig" compiled into the library
    BuiltinOrig,
    /// external binary, called as `<path> <mode> <N> <steps> <args…>`
    External { path: String, args: String },
}

#[derive(Clone, Debug)]
pub struct RawFilter {
    pub kind: FilterKind,
    pub f0: f32,
    pub gain: f32,
    pub q: f32,
}

#[derive(Clone, Debug)]
pub struct AutoeqOutput {
    pub filters: Vec<RawFilter>,
    /// first value printed by the binary (optimized global gain)
    pub amp: f32,
    /// second value: maximum of the filter stack's response
    pub max: f32,
    /// mean removed from the residual (built-in variant only)
    pub mean: Option<f32>,
    pub duration_ms: f64,
}

/// Logarithmic grid of the algorithm, computed as in the Python script
/// (numpy, f64), for resampling curves.
pub fn grid_f64() -> Vec<f64> {
    let (l0, l1) = (F_LO.ln(), F_HI.ln());
    (0..K)
        .map(|k| (l0 + (l1 - l0) * k as f64 / (K - 1) as f64).exp())
        .collect()
}

/// Exact grid used by the C code (f32, logf/expf).
pub fn grid_c() -> Vec<f32> {
    let (l0, l1) = ((F_LO as f32).ln(), (F_HI as f32).ln());
    let lr = l1 - l0;
    (0..K)
        .map(|k| (l0 + lr / (K - 1) as f32 * k as f32).exp())
        .collect()
}

/// Resamples a curve onto the K-point grid (linear interpolation
/// in log f, edge values extended, like np.interp).
pub fn resample_to_grid(freqs: &[f64], db: &[f64]) -> Vec<f32> {
    grid_f64()
        .iter()
        .map(|&f| interp_log(freqs, db, f) as f32)
        .collect()
}

pub fn c_grid_len() -> usize {
    unsafe { eqing_orig_k() as usize }
}

/// Response of a filter computed by the C code (on the C grid).
pub fn c_spectrum(kind: FilterKind, f0: f32, gain: f32, q: f32) -> Vec<f32> {
    let f = grid_c();
    let mut y = vec![0f32; K];
    unsafe { eqing_orig_spectrum(kind.c_type(), f0, gain, q, FS as f32, f.as_ptr(), y.as_mut_ptr()) };
    y
}

fn kind_from_c(t: i32) -> FilterKind {
    match t {
        1 => FilterKind::LowShelf,
        2 => FilterKind::HighShelf,
        _ => FilterKind::Peak,
    }
}

fn kind_from_name(s: &str) -> Option<FilterKind> {
    match s {
        "PK" => Some(FilterKind::Peak),
        "LSC" => Some(FilterKind::LowShelf),
        "HSC" => Some(FilterKind::HighShelf),
        _ => None,
    }
}

/// Runs the algorithm. `dst` = target, `src` = measurement, both on the K-point grid.
pub fn run(
    engine: &Engine,
    mode: Mode,
    n: u32,
    steps: u32,
    dst: &[f32],
    src: &[f32],
) -> Result<AutoeqOutput, String> {
    if dst.len() != K || src.len() != K {
        return Err(format!("curves of {} points expected", K));
    }
    if n == 0 || n as usize > MAX_N {
        return Err(format!("invalid number of filters ({n}, max {MAX_N})"));
    }
    if steps == 0 {
        return Err("invalid number of steps".into());
    }
    match engine {
        Engine::BuiltinOrig => run_builtin(mode, n, steps, dst, src),
        Engine::External { path, args } => run_external(path, args, mode, n, steps, dst, src),
    }
}

fn run_builtin(mode: Mode, n: u32, steps: u32, dst: &[f32], src: &[f32]) -> Result<AutoeqOutput, String> {
    let dst = dst.to_vec();
    let src = src.to_vec();
    let t0 = Instant::now();
    // the C code allocates ~200 KB on the stack: give it a thread with a roomy
    // stack (secondary threads on macOS only get 512 KB)
    let handle = std::thread::Builder::new()
        .name("autoeq".into())
        .stack_size(16 << 20)
        .spawn(move || {
            let mut out = CResult {
                types: [0; MAX_N],
                f0: [0.0; MAX_N],
                gain: [0.0; MAX_N],
                q: [0.0; MAX_N],
                amp: 0.0,
                max: 0.0,
                mean: 0.0,
            };
            let code = unsafe {
                eqing_orig_autoeq_run(
                    mode.letter() as c_char,
                    n as i32,
                    steps as i32,
                    dst.as_ptr(),
                    src.as_ptr(),
                    &mut out,
                )
            };
            (code, out)
        })
        .map_err(|e| format!("could not start the computation: {e}"))?;
    let (code, out) = handle.join().map_err(|_| "the computation crashed".to_string())?;
    if code != 0 {
        return Err(format!("autoeq rejected the parameters (code {code})"));
    }
    let filters = (0..n as usize)
        .map(|i| RawFilter {
            kind: kind_from_c(out.types[i]),
            f0: out.f0[i],
            gain: out.gain[i],
            q: out.q[i],
        })
        .collect();
    Ok(AutoeqOutput {
        filters,
        amp: out.amp,
        max: out.max,
        mean: Some(out.mean),
        duration_ms: t0.elapsed().as_secs_f64() * 1000.0,
    })
}

fn run_external(
    path: &str,
    args: &str,
    mode: Mode,
    n: u32,
    steps: u32,
    dst: &[f32],
    src: &[f32],
) -> Result<AutoeqOutput, String> {
    let t0 = Instant::now();
    let mut cmd = Command::new(path);
    cmd.arg(mode.letter().to_string())
        .arg(n.to_string())
        .arg(steps.to_string())
        .args(args.split_whitespace())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("could not launch {path}: {e}"))?;
    let mut payload = Vec::with_capacity(8 * K);
    for v in dst.iter().chain(src.iter()) {
        payload.extend_from_slice(&v.to_le_bytes());
    }
    child
        .stdin
        .take()
        .ok_or("stdin unavailable")?
        .write_all(&payload)
        .map_err(|e| format!("writing to the binary: {e}"))?;
    let out = child
        .wait_with_output()
        .map_err(|e| format!("waiting for the binary: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "the binary failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let mut res = parse_binary_output(&String::from_utf8_lossy(&out.stdout))?;
    res.duration_ms = t0.elapsed().as_secs_f64() * 1000.0;
    Ok(res)
}

/// Reads the text output of the autoeq binary (`amp max` then `TYPE f0 gain Q`).
pub fn parse_binary_output(text: &str) -> Result<AutoeqOutput, String> {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let first = lines.next().ok_or("empty output")?;
    let nums: Vec<f32> = first
        .split_whitespace()
        .filter_map(|x| x.parse().ok())
        .collect();
    if nums.len() != 2 {
        return Err(format!("unexpected first line: {first}"));
    }
    let mut filters = Vec::new();
    for l in lines {
        let p: Vec<&str> = l.split_whitespace().collect();
        if p.len() != 4 {
            return Err(format!("unexpected filter line: {l}"));
        }
        let kind = kind_from_name(p[0]).ok_or(format!("unknown filter type: {}", p[0]))?;
        let num = |s: &str| s.parse::<f32>().map_err(|_| format!("invalid number: {s}"));
        filters.push(RawFilter { kind, f0: num(p[1])?, gain: num(p[2])?, q: num(p[3])? });
    }
    Ok(AutoeqOutput { filters, amp: nums[0], max: nums[1], mean: None, duration_ms: 0.0 })
}

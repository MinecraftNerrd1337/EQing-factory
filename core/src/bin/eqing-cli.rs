//! Command line (Mac, Linux, WSL) around the same core as the app.
//!
//!   eqing-cli autoeq -t <target.csv> -m <measurement.csv> [-c <compensation.csv>] [-n 20] [-s 3000] [--mode i]
//!                    [--format parametric|graphic|csv|raw] [--binary <path> [--args "<args>"]]
//!   eqing-cli version

use std::process::ExitCode;

use eqing_core::dsp::parse_curve_text;
use eqing_core::engine::{self, Engine, Mode};
use eqing_core::formats::{self, ParsedFilter};
use eqing_core::model::{Filter, FilterOrigin};

fn usage() -> ExitCode {
    eprintln!(
        "usage:\n  eqing-cli autoeq -t <target.csv> -m <measurement.csv> [-c <compensation.csv>]\n\
         \x20                  [-n 20] [-s 3000] [--mode i|o|x]\n\
         \x20                  [--format parametric|graphic|csv|raw] [--binary <path> [--args \"…\"]]\n\
         \x20 eqing-cli version"
    );
    ExitCode::from(2)
}

/// Same rendering as printf("%.7e") in C (signed exponent, at least two digits).
fn c_exp(v: f32) -> String {
    let s = format!("{:.7e}", v);
    let (m, e) = s.split_once('e').unwrap();
    let e: i32 = e.parse().unwrap();
    format!("{m}e{}{:02}", if e < 0 { '-' } else { '+' }, e.abs())
}

fn read_curve(path: &str) -> Result<(Vec<f64>, Vec<f64>), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let c = parse_curve_text(&text).map_err(|e| format!("{path}: {e}"))?;
    Ok((c.freqs, c.db))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("version") => {
            println!("eqing-cli {} (built-in autoeq-c orig, K={})", env!("CARGO_PKG_VERSION"), engine::c_grid_len());
            ExitCode::SUCCESS
        }
        Some("autoeq") => match autoeq(&args[1..]) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
        _ => usage(),
    }
}

fn autoeq(args: &[String]) -> Result<(), String> {
    let (mut target, mut meas, mut n, mut steps, mut mode, mut format) =
        (None, None, 20u32, 3000u32, Mode::InEar, "parametric".to_string());
    let (mut binary, mut extra, mut comp) = (None, String::new(), None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = || it.next().cloned().ok_or(format!("missing value after {a}"));
        match a.as_str() {
            "-t" | "--target" => target = Some(val()?),
            "-m" | "--measurement" => meas = Some(val()?),
            "-c" | "--compensation" => comp = Some(val()?),
            "-n" | "--bands" => n = val()?.parse().map_err(|_| "invalid number of bands")?,
            "-s" | "--steps" => steps = val()?.parse().map_err(|_| "invalid number of steps")?,
            "--mode" => mode = Mode::from_letter(&val()?).ok_or("mode: i, o or x")?,
            "--format" => format = val()?,
            "--binary" => binary = Some(val()?),
            "--args" => extra = val()?,
            other => return Err(format!("unknown option: {other}")),
        }
    }
    let target = target.ok_or("missing target (-t)")?;
    let meas = meas.ok_or("missing measurement (-m)")?;
    let (tf, td) = read_curve(&target)?;
    let (mf, md) = read_curve(&meas)?;
    let dst = engine::resample_to_grid(&tf, &td);
    let mut src = engine::resample_to_grid(&mf, &md);
    if let Some(c) = comp {
        // compensation subtracted from the measurement before autoeq
        let (cf, cd) = read_curve(&c)?;
        for (s, c) in src.iter_mut().zip(engine::resample_to_grid(&cf, &cd)) {
            *s -= c;
        }
    }
    let eng = match binary {
        Some(path) => Engine::External { path, args: extra },
        None => Engine::BuiltinOrig,
    };
    let out = engine::run(&eng, mode, n, steps, &dst, &src)?;
    let filters: Vec<Filter> = out
        .filters
        .iter()
        .enumerate()
        .map(|(i, f)| Filter {
            id: i.to_string(),
            kind: f.kind,
            f0: f.f0 as f64,
            gain: f.gain as f64,
            q: f.q as f64,
            enabled: true,
            origin: FilterOrigin::Auto,
        })
        .collect();
    let parsed: Vec<ParsedFilter> = filters
        .iter()
        .map(|f| ParsedFilter { kind: f.kind, f0: f.f0, gain: f.gain, q: f.q, enabled: true })
        .collect();
    let pre = formats::export_preamp(&parsed);
    match format.as_str() {
        "raw" => {
            println!("{} {}", c_exp(out.amp), c_exp(out.max));
            for f in &out.filters {
                println!("{} {} {} {}", f.kind.apo_name(), c_exp(f.f0), c_exp(f.gain), c_exp(f.q));
            }
        }
        "parametric" => println!("{}", formats::apo_text(&parsed, pre)),
        "graphic" => println!("{}", formats::graphic_eq(&parsed)),
        "csv" => print!("{}", formats::filters_csv(&parsed, pre)),
        other => return Err(format!("unknown format: {other}")),
    }
    eprintln!("{:.0} ms", out.duration_ms);
    Ok(())
}

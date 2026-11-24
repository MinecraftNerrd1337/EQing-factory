//! Core of EQing Factory, an EQ correction tool.
//!
//! - `engine`: autoeq-c compiled into the library (FFI) or an external binary
//! - `dsp`: filter response, interpolation, CSV reading
//! - `formats`: Equalizer APO, GraphicEQ (Wavelet), filters CSV
//! - `model` / `state`: persisted data and app logic
//! - `api`: `AppCore` object exposed to Swift (and Python…) by uniFFI

uniffi::setup_scaffolding!();

pub mod api;
pub mod dsp;
pub mod engine;
pub mod error;
pub mod formats;
pub mod model;
pub mod state;

pub use api::AppCore;
pub use error::CoreError;

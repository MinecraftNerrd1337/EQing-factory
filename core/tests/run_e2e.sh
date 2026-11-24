#!/bin/sh
# Generates the Python bindings and runs the end-to-end test of the uniFFI layer.
set -e
cd "$(dirname "$0")/.."
T=${CARGO_TARGET_DIR:-target}
cargo build --release --features bindgen
LIB=$(ls "$T"/release/libeqing_core.so "$T"/release/libeqing_core.dylib 2>/dev/null | head -1)
OUT=$(mktemp -d)
"$T"/release/uniffi-bindgen generate --library "$LIB" --language python --out-dir "$OUT" >/dev/null
cp "$LIB" "$OUT"/
python3 tests/uniffi_e2e.py "$OUT" "${CURVES:-../../bench/curves}"

#!/bin/sh
# Builds the command-line tool (macOS, Linux, WSL). Requirements: Rust and a C compiler.
set -e
cd "$(dirname "$0")"
cargo build --release --bin eqing-cli
echo "Binary: $(pwd)/target/release/eqing-cli"

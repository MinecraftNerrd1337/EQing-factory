// Bindings generator (Swift, Python, Kotlin…) working from the compiled library:
//   cargo run --features bindgen --bin uniffi-bindgen -- generate \
//       --library <libeqing_core.dylib|.so> --language swift --out-dir <folder>
fn main() {
    uniffi::uniffi_bindgen_main()
}

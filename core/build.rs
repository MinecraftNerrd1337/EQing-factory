// Compiles the autoeq C sources ("orig" variant, identical to src-orig)
// into the Rust library, with the csrc/api.c entry point.
fn main() {
    let dir = "csrc/orig";
    for f in ["autoeq.c", "biquad.c", "init.c", "lib.h"] {
        println!("cargo:rerun-if-changed={dir}/{f}");
    }
    println!("cargo:rerun-if-changed=csrc/api.c");

    cc::Build::new()
        .files([
            format!("{dir}/autoeq.c"),
            format!("{dir}/biquad.c"),
            format!("{dir}/init.c"),
            "csrc/api.c".to_string(),
        ])
        .include(dir)
        // same optimization level as the reference binary (cc -O2)
        .opt_level(2)
        .define("main", "eqing_orig_cli_main")
        .define("EQING_PREFIX", "orig")
        .warnings(false)
        .compile("autoeq_orig");
}

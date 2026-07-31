/// Build script: compiles the vendored tinyuz C++ library for wireless RGB compression.
fn main() {
    let vendor = std::path::Path::new("../../vendor");
    let tinyuz = vendor.join("tinyuz");
    let hdiff = vendor.join("HDiffPatch");

    // All C++ source files needed for tinyuz compression
    let cpp_sources = [
        vendor.join("tuz_wrapper.cpp"),
        tinyuz.join("compress/tuz_enc.cpp"),
        tinyuz.join("compress/tuz_enc_private/tuz_enc_clip.cpp"),
        tinyuz.join("compress/tuz_enc_private/tuz_enc_code.cpp"),
        tinyuz.join("compress/tuz_enc_private/tuz_enc_match.cpp"),
        tinyuz.join("compress/tuz_enc_private/tuz_sstring.cpp"),
        hdiff.join("libHDiffPatch/HDiff/private_diff/libdivsufsort/divsufsort.cpp"),
    ];

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .opt_level(3)
        .define("NDEBUG", None)
        // Disable multi-threading (not needed, avoids extra dependencies)
        .define("_IS_USED_MULTITHREAD", "0")
        // Include paths so tinyuz can find its headers and HDiffPatch
        .include(vendor)
        .include(&tinyuz)
        .include(&hdiff)
        .warnings(false);

    for src in &cpp_sources {
        build.file(src);
    }

    // Decompressor (used for round-trip validation tests)
    build.file(tinyuz.join("decompress/tuz_dec.c"));

    build.compile("tinyuz");

    // Re-run if vendor sources change
    println!("cargo:rerun-if-changed=../../vendor/tuz_wrapper.cpp");
    println!("cargo:rerun-if-changed=../../vendor/tinyuz/compress/");
    println!("cargo:rerun-if-changed=../../vendor/tinyuz/decompress/");
}

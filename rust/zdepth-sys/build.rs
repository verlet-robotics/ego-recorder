fn main() {
    // Find system libzstd
    let zstd = pkg_config::Config::new()
        .atleast_version("1.0")
        .probe("libzstd")
        .expect("libzstd not found via pkg-config. Install libzstd-dev.");

    // Compile Zdepth source + our C wrapper
    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++17")
        .file("vendor/src/zdepth.cpp")
        .file("zdepth_c.cpp")
        .include("vendor/include")
        .include(".");

    // Add zstd include paths from pkg-config
    for path in &zstd.include_paths {
        build.include(path);
    }

    build.compile("zdepth_c");

    // Link zstd
    println!("cargo:rustc-link-lib=zstd");
    println!("cargo:rerun-if-changed=zdepth_c.cpp");
    println!("cargo:rerun-if-changed=wrapper.h");
    println!("cargo:rerun-if-changed=vendor/src/zdepth.cpp");
    println!("cargo:rerun-if-changed=vendor/include/zdepth.hpp");
}

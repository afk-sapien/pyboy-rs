// SPDX-License-Identifier: LGPL-3.0-only
// Builds the vendored mGBA (MPL-2.0) as a minimal static library with only the GBA core,
// then compiles the small C shim in shim/ against it.
use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let mgba = root.join("vendor/mgba");
    if !mgba.join("CMakeLists.txt").exists() {
        panic!(
            "mGBA sources are missing at {}. Run `git submodule update --init crates/pyboy-gba/vendor/mgba`",
            mgba.display()
        );
    }
    println!("cargo:rerun-if-changed=shim/pbgba.c");
    println!("cargo:rerun-if-changed=build.rs");

    let dst = cmake::Config::new(&mgba)
        .profile("Release")
        // mGBA 0.10.5 asks for CMake 3.1, which CMake 4 (macOS and Windows runners) rejects.
        .define("CMAKE_POLICY_VERSION_MINIMUM", "3.5")
        .define("LIBMGBA_ONLY", "ON")
        .define("BUILD_STATIC", "ON")
        .define("BUILD_SHARED", "OFF")
        .define("M_CORE_GBA", "ON")
        .define("M_CORE_GB", "OFF")
        .define("DISABLE_DEPS", "ON")
        .define("USE_DEBUGGERS", "OFF")
        .define("USE_GDB_STUB", "OFF")
        .define("ENABLE_SCRIPTING", "OFF")
        .define("USE_EPOXY", "OFF")
        .define("BUILD_GL", "OFF")
        .define("BUILD_GLES2", "OFF")
        .define("BUILD_GLES3", "OFF")
        .define("BUILD_LTO", "OFF")
        .define("SKIP_GIT", "ON")
        .define("CMAKE_POSITION_INDEPENDENT_CODE", "ON")
        .build_target("mgba")
        .build();
    let build_dir = dst.join("build");

    cc::Build::new()
        .file("shim/pbgba.c")
        .include(mgba.join("include"))
        .include(build_dir.join("include"))
        .include(mgba.join("src"))
        .opt_level(2)
        .warnings(false)
        .compile("pbgba");

    println!("cargo:rustc-link-search=native={}", build_dir.display());
    println!("cargo:rustc-link-lib=static=mgba");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        println!("cargo:rustc-link-lib=m");
        println!("cargo:rustc-link-lib=pthread");
    }
}

use std::env;
use std::path::Path;

/// Canonicalize a path without the \\?\ prefix on Windows
fn clean_canonicalize(p: &Path) -> String {
    let canon = p.canonicalize().unwrap();
    let s = canon.to_str().unwrap();
    s.strip_prefix(r"\\?\").unwrap_or(s).to_string()
}

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let wrapper_dir = Path::new(&manifest_dir).join("cmake");
    let neural_audio_dir = Path::new(&manifest_dir).join("../vendor/NeuralAudio");

    // Copy our patched CAPI files over the submodule versions before building.
    // The upstream submodule uses `wchar_t*` on all platforms; our patch makes
    // Linux/macOS use `const char*` instead, matching what the Rust FFI sends.
    let capi_dir = neural_audio_dir.join("NeuralAudioCAPI");
    let override_dir = Path::new(&manifest_dir).join("capi-override");
    for filename in &["NeuralAudioCApi.h", "NeuralAudioCApi.cpp"] {
        let src = override_dir.join(filename);
        let dst = capi_dir.join(filename);
        std::fs::copy(&src, &dst)
            .unwrap_or_else(|e| panic!("Failed to copy {} override: {}", filename, e));
        println!("cargo:rerun-if-changed=capi-override/{}", filename);
    }

    let mut cfg = cmake::Config::new(&wrapper_dir);
    // Force single-config generator on Unix to avoid libraries landing in Release/ subdirs
    if !cfg!(target_os = "windows") {
        cfg.generator("Unix Makefiles");
    }
    let dst = cfg
        .define("CMAKE_BUILD_TYPE", "Release")
        .define("NEURALAUDIO_SOURCE_DIR", clean_canonicalize(&neural_audio_dir))
        .define("BUILD_NAMCORE", "OFF")
        .define("BUILD_STATIC_RTNEURAL", "OFF")
        .define("BUILD_UTILS", "OFF")
        .define("WAVENET_FRAMES", "64")
        .define("WAVENET_MATH", "FastMath")
        .define("LSTM_MATH", "FastMath")
        .build_target("NeuralAudioCAPI")
        .build();

    let build_dir = dst.join("build");

    // The wrapper CMakeLists adds NeuralAudio as a subdirectory, so libraries
    // are nested under build/NeuralAudio/{NeuralAudioCAPI,NeuralAudio}/
    // Also search Release/ paths for multi-config generator compatibility (Windows)
    println!("cargo:rustc-link-search=native={}", build_dir.join("NeuralAudio").join("NeuralAudioCAPI").display());
    println!("cargo:rustc-link-search=native={}", build_dir.join("NeuralAudio").join("NeuralAudioCAPI").join("Release").display());
    println!("cargo:rustc-link-search=native={}", build_dir.join("NeuralAudio").join("NeuralAudio").display());
    println!("cargo:rustc-link-search=native={}", build_dir.join("NeuralAudio").join("NeuralAudio").join("Release").display());

    println!("cargo:rustc-link-lib=static=NeuralAudioCAPI");
    println!("cargo:rustc-link-lib=static=NeuralAudio");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    match target_os.as_str() {
        "macos" => println!("cargo:rustc-link-lib=c++"),
        "linux" => println!("cargo:rustc-link-lib=stdc++"),
        _ => {}
    }

}

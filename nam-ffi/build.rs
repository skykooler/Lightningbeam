use std::env;

fn main() {
    let mut cfg = cmake::Config::new("../vendor/NeuralAudio");
    // Force single-config generator on Unix to avoid libraries landing in Release/ subdirs
    if !cfg!(target_os = "windows") {
        cfg.generator("Unix Makefiles");
    }
    let dst = cfg
        .define("CMAKE_BUILD_TYPE", "Release")
        .define("BUILD_NAMCORE", "OFF")
        .define("BUILD_STATIC_RTNEURAL", "OFF")
        .define("BUILD_UTILS", "OFF")
        .define("WAVENET_FRAMES", "64")
        .define("WAVENET_MATH", "FastMath")
        .define("LSTM_MATH", "FastMath")
        .build_target("NeuralAudioCAPI")
        .build();

    let build_dir = dst.join("build");

    // Static libraries land in the build subdirectories
    // Search both direct and Release/ paths for multi-config generator compatibility
    println!("cargo:rustc-link-search=native={}", build_dir.join("NeuralAudioCAPI").display());
    println!("cargo:rustc-link-search=native={}", build_dir.join("NeuralAudioCAPI").join("Release").display());
    println!("cargo:rustc-link-search=native={}", build_dir.join("NeuralAudio").display());
    println!("cargo:rustc-link-search=native={}", build_dir.join("NeuralAudio").join("Release").display());

    println!("cargo:rustc-link-lib=static=NeuralAudioCAPI");
    println!("cargo:rustc-link-lib=static=NeuralAudio");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    match target_os.as_str() {
        "macos" => println!("cargo:rustc-link-lib=c++"),
        "linux" => println!("cargo:rustc-link-lib=stdc++"),
        _ => {}
    }

    println!("cargo:rerun-if-changed=../vendor/NeuralAudio/NeuralAudioCAPI/NeuralAudioCApi.h");
    println!("cargo:rerun-if-changed=../vendor/NeuralAudio/NeuralAudioCAPI/NeuralAudioCApi.cpp");
}

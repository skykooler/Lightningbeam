use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    // Only bundle libs on Linux
    if env::var("CARGO_CFG_TARGET_OS").unwrap() != "linux" {
        return;
    }

    // Skip bundling if using static FFmpeg linking
    if env::var("PKG_CONFIG_ALL_STATIC").is_ok() || env::var("FFMPEG_STATIC").is_ok() {
        println!("cargo:warning=Skipping FFmpeg library bundling (static linking enabled)");
        return;
    }

    // Get the output directory
    let out_dir = env::var("OUT_DIR").unwrap();
    let target_dir = PathBuf::from(&out_dir)
        .parent().unwrap()
        .parent().unwrap()
        .parent().unwrap()
        .to_path_buf();

    // Create lib directory in target
    let lib_dir = target_dir.join("lib");
    fs::create_dir_all(&lib_dir).ok();

    println!("cargo:warning=Bundling FFmpeg libraries to {:?}", lib_dir);

    // List of FFmpeg 8.x libraries to bundle
    let ffmpeg_libs = [
        "libavcodec.so.62",
        "libavdevice.so.62",
        "libavfilter.so.11",
        "libavformat.so.62",
        "libavutil.so.60",
        "libpostproc.so.57",  // Actually version 57 in Ubuntu 24.04
        "libswresample.so.6",  // Actually version 6
        "libswscale.so.9",
    ];

    let lib_search_paths = [
        "/usr/lib/x86_64-linux-gnu",
        "/usr/lib64",
        "/usr/lib",
    ];

    // Copy FFmpeg libraries
    for lib_name in &ffmpeg_libs {
        copy_library(lib_name, &lib_search_paths, &lib_dir);
    }

    // Also bundle all FFmpeg codec dependencies to avoid version mismatches
    let codec_libs = [
        // Codec libraries
        "libaom.so.3", "libdav1d.so.7", "librav1e.so.0", "libSvtAv1Enc.so.1",
        "libvpx.so.9", "libx264.so.164", "libx265.so.199",
        "libopus.so.0", "libvorbis.so.0", "libvorbisenc.so.2", "libmp3lame.so.0",
        "libtheora.so.0", "libtheoraenc.so.1", "libtheoradec.so.1",
        "libtwolame.so.0", "libspeex.so.1", "libshine.so.3",
        "libwebp.so.7", "libwebpmux.so.3", "libjxl.so.0.7", "libjxl_threads.so.0.7",
        // Container/protocol libraries
        "librabbitmq.so.4", "librist.so.4", "libsrt-gnutls.so.1.5", "libzmq.so.5",
        "libbluray.so.2", "libdvdnav.so.4", "libdvdread.so.8",
        // Other dependencies
        "libaribb24.so.0", "libcodec2.so.1.2", "libgsm.so.1",
        "libopencore-amrnb.so.0", "libopencore-amrwb.so.0",
        "libvo-amrwbenc.so.0", "libfdk-aac.so.2", "libilbc.so.3",
        "libopenjp2.so.7", "libsnappy.so.1", "libvvenc.so.1.12",
    ];

    for lib_name in &codec_libs {
        copy_library(lib_name, &lib_search_paths, &lib_dir);
    }

    // Set rpath to look in ./lib and $ORIGIN/lib
    println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN/lib");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib_dir.display());
}

fn copy_library(lib_name: &str, search_paths: &[&str], lib_dir: &PathBuf) {
    let mut copied = false;

    for search_path in search_paths {
        let src = PathBuf::from(search_path).join(lib_name);
        if src.exists() {
            let dst = lib_dir.join(lib_name);
            if let Err(e) = fs::copy(&src, &dst) {
                println!("cargo:warning=Failed to copy {}: {}", lib_name, e);
            } else {
                copied = true;
                break;
            }
        }
    }

    if !copied {
        // Don't warn for optional libraries
        if !lib_name.contains("shine") && !lib_name.contains("fdk-aac") {
            println!("cargo:warning=Could not find {} (optional)", lib_name);
        }
    }
}

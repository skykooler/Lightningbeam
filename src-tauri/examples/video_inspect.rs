extern crate ffmpeg_next as ffmpeg;

use std::env;

fn main() {
    ffmpeg::init().unwrap();

    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <video_file>", args[0]);
        std::process::exit(1);
    }

    let path = &args[1];
    let input = ffmpeg::format::input(path).expect("Failed to open video");

    println!("=== VIDEO FILE INFORMATION ===");
    println!("File: {}", path);
    println!("Format: {}", input.format().name());
    println!("Duration: {:.2}s", input.duration() as f64 / f64::from(ffmpeg::ffi::AV_TIME_BASE));
    println!();

    let video_stream = input.streams()
        .best(ffmpeg::media::Type::Video)
        .expect("No video stream found");

    let stream_index = video_stream.index();
    let time_base = f64::from(video_stream.time_base());
    let duration = video_stream.duration() as f64 * time_base;
    let fps = f64::from(video_stream.avg_frame_rate());

    println!("=== VIDEO STREAM ===");
    println!("Stream index: {}", stream_index);
    println!("Time base: {} ({:.10})", video_stream.time_base(), time_base);
    println!("Duration: {:.2}s", duration);
    println!("FPS: {:.2}", fps);
    println!("Frames: {}", video_stream.frames());

    let context = ffmpeg::codec::context::Context::from_parameters(video_stream.parameters())
        .expect("Failed to create context");
    let decoder = context.decoder().video().expect("Failed to create decoder");

    println!("Codec: {:?}", decoder.id());
    println!("Resolution: {}x{}", decoder.width(), decoder.height());
    println!("Pixel format: {:?}", decoder.format());
    println!();

    println!("=== SCANNING FRAMES ===");
    println!("Timestamp (ts) | Time (s) | Key | Type");
    println!("---------------|----------|-----|-----");

    let mut input = ffmpeg::format::input(path).expect("Failed to reopen video");
    let context = ffmpeg::codec::context::Context::from_parameters(
        input.streams().best(ffmpeg::media::Type::Video).unwrap().parameters()
    ).expect("Failed to create context");
    let mut decoder = context.decoder().video().expect("Failed to create decoder");

    let mut frame_count = 0;
    let mut keyframe_count = 0;

    for (stream, packet) in input.packets() {
        if stream.index() == stream_index {
            let packet_pts = packet.pts().unwrap_or(0);
            let packet_time = packet_pts as f64 * time_base;
            let is_key = packet.is_key();

            if is_key {
                keyframe_count += 1;
            }

            // Print first 50 packets and all keyframes
            if frame_count < 50 || is_key {
                println!("{:14} | {:8.2} | {:3} | {:?}",
                    packet_pts,
                    packet_time,
                    if is_key { "KEY" } else { " " },
                    if is_key { "I-frame" } else { "P/B-frame" }
                );
            }

            decoder.send_packet(&packet).ok();
            let mut frame = ffmpeg::util::frame::Video::empty();
            while decoder.receive_frame(&mut frame).is_ok() {
                frame_count += 1;
            }
        }
    }

    // Flush decoder
    decoder.send_eof().ok();
    let mut frame = ffmpeg::util::frame::Video::empty();
    while decoder.receive_frame(&mut frame).is_ok() {
        frame_count += 1;
    }

    println!();
    println!("=== SUMMARY ===");
    println!("Total frames decoded: {}", frame_count);
    println!("Total keyframes: {}", keyframe_count);
    if keyframe_count > 0 {
        println!("Average keyframe interval: {:.2} frames", frame_count as f64 / keyframe_count as f64);
        println!("Average keyframe interval: {:.2}s", duration / keyframe_count as f64);
    }
}

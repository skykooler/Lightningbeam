/// Minimal test program to validate FFmpeg audio encoding workflow
///
/// This program tests encoding raw PCM samples to MP3 using ffmpeg-next.
/// Run with: cargo run --example ffmpeg_test

use std::path::Path;

fn main() -> Result<(), String> {
    println!("Testing FFmpeg audio encoding...");

    // Initialize FFmpeg
    ffmpeg_next::init().map_err(|e| format!("Failed to initialize FFmpeg: {}", e))?;

    // Test 1: List available encoders
    println!("\nAvailable MP3 encoders:");
    if let Some(encoder) = ffmpeg_next::encoder::find(ffmpeg_next::codec::Id::MP3) {
        println!("  - Found MP3 encoder: {}", encoder.name());
    } else {
        println!("  - No MP3 encoder found!");
    }

    println!("\nAvailable AAC encoders:");
    if let Some(encoder) = ffmpeg_next::encoder::find(ffmpeg_next::codec::Id::AAC) {
        println!("  - Found AAC encoder: {}", encoder.name());
    } else {
        println!("  - No AAC encoder found!");
    }

    // Test 2: Create a simple MP3 encoder and encode silence
    test_mp3_encoding()?;

    // Test 3: Create a simple AAC encoder and encode silence
    test_aac_encoding()?;

    println!("\n✅ All tests passed!");
    Ok(())
}

fn test_mp3_encoding() -> Result<(), String> {
    println!("\nTest: Encoding 1 second of silence to MP3...");

    // Output file
    let output_path = "/tmp/test_silence.mp3";

    // Generate 1 second of stereo silence at 44.1 kHz
    let sample_rate = 44100;
    let channels = 2;
    let duration_secs = 1.0;
    let num_samples = (sample_rate as f64 * duration_secs * channels as f64) as usize;
    let pcm_samples: Vec<f32> = vec![0.0; num_samples]; // Silence

    println!("  Generated {} PCM samples ({}Hz, {} channels, {:.1}s)",
             num_samples, sample_rate, channels, duration_secs);

    // Encode to MP3
    encode_pcm_to_mp3(&pcm_samples, sample_rate, channels, 320, output_path)?;

    // Check output file exists
    if Path::new(output_path).exists() {
        let metadata = std::fs::metadata(output_path).unwrap();
        println!("  ✅ Created MP3 file: {} ({} bytes)", output_path, metadata.len());
    } else {
        return Err("MP3 file was not created!".to_string());
    }

    Ok(())
}

fn test_aac_encoding() -> Result<(), String> {
    println!("\nTest: Encoding 1 second of silence to AAC...");

    // Output file
    let output_path = "/tmp/test_silence.m4a";

    // Generate 1 second of stereo silence at 44.1 kHz
    let sample_rate = 44100;
    let channels = 2;
    let duration_secs = 1.0;
    let num_samples = (sample_rate as f64 * duration_secs * channels as f64) as usize;
    let pcm_samples: Vec<f32> = vec![0.0; num_samples]; // Silence

    println!("  Generated {} PCM samples ({}Hz, {} channels, {:.1}s)",
             num_samples, sample_rate, channels, duration_secs);

    // Encode to AAC
    encode_pcm_to_aac(&pcm_samples, sample_rate, channels, 192, output_path)?;

    // Check output file exists
    if Path::new(output_path).exists() {
        let metadata = std::fs::metadata(output_path).unwrap();
        println!("  ✅ Created AAC file: {} ({} bytes)", output_path, metadata.len());
    } else {
        return Err("AAC file was not created!".to_string());
    }

    Ok(())
}

/// Encode raw PCM samples to MP3 using ffmpeg-next
fn encode_pcm_to_mp3(
    samples: &[f32],
    sample_rate: u32,
    channels: u32,
    bitrate_kbps: u32,
    output_path: &str,
) -> Result<(), String> {
    use ffmpeg_next as ffmpeg;

    // Find MP3 encoder
    let encoder_codec = ffmpeg::encoder::find(ffmpeg::codec::Id::MP3)
        .ok_or("MP3 encoder not found")?;

    println!("  Using encoder: {}", encoder_codec.name());

    // Create output format context FIRST (like transcode example)
    let mut output = ffmpeg::format::output(&output_path)
        .map_err(|e| format!("Failed to create output file: {}", e))?;

    // Don't use stream parameters - create encoder directly
    // The stream was just added but has no parameters set yet
    let mut encoder = ffmpeg::codec::Context::new_with_codec(encoder_codec)
        .encoder()
        .audio()
        .map_err(|e| format!("Failed to create encoder: {}", e))?;

    println!("  Created encoder directly from codec");

    // Determine channel layout first
    let channel_layout = match channels {
        1 => ffmpeg::channel_layout::ChannelLayout::MONO,
        2 => ffmpeg::channel_layout::ChannelLayout::STEREO,
        _ => return Err(format!("Unsupported channel count: {}", channels)),
    };

    // Configure encoder with explicit format (required in ffmpeg-next 8.0)
    encoder.set_rate(sample_rate as i32);
    encoder.set_channel_layout(channel_layout);

    // Set format to S16 Planar (s16p) which libmp3lame supports
    use ffmpeg_next::format::sample::Type;
    use ffmpeg_next::format::Sample;
    encoder.set_format(Sample::I16(Type::Planar));

    encoder.set_bit_rate((bitrate_kbps * 1000) as usize);
    encoder.set_time_base(ffmpeg::Rational(1, sample_rate as i32));

    println!("  Encoder configured: {}Hz, {} channels, {} kbps",
             sample_rate, channels, bitrate_kbps);
    println!("  Format before open: {:?}", encoder.format());

    // Open encoder (like transcode-audio example)
    let mut encoder = encoder.open_as(encoder_codec)
        .map_err(|e| format!("Failed to open encoder: {}", e))?;

    println!("  ✅ Encoder opened successfully!");
    println!("  Opened encoder format: {:?}", encoder.format());

    // Now add stream and set its parameters from the opened encoder
    let mut stream = output.add_stream(encoder_codec)
        .map_err(|e| format!("Failed to add stream: {}", e))?;
    stream.set_parameters(&encoder);

    // Write header
    output.write_header()
        .map_err(|e| format!("Failed to write header: {}", e))?;

    println!("  Encoding {} samples...", samples.len());

    // Convert interleaved f32 to planar i16
    let num_frames = samples.len() / channels as usize;
    let planar_samples = convert_to_planar_i16(samples, channels);

    // Get encoder frame size
    let frame_size = encoder.frame_size();
    let samples_per_frame = if frame_size > 0 {
        frame_size as usize
    } else {
        1152 // Default MP3 frame size
    };

    println!("  Frame size: {} samples", samples_per_frame);

    // Encode in chunks
    let mut samples_encoded = 0;
    while samples_encoded < num_frames {
        let samples_remaining = num_frames - samples_encoded;
        let chunk_size = samples_remaining.min(samples_per_frame);

        // Create audio frame
        let mut frame = ffmpeg::frame::Audio::new(
            ffmpeg::format::Sample::I16(ffmpeg::format::sample::Type::Planar),
            chunk_size,
            channel_layout,
        );
        frame.set_rate(sample_rate);

        // Copy planar samples to frame
        for ch in 0..channels as usize {
            let plane = frame.data_mut(ch);
            let offset = samples_encoded;
            let src = &planar_samples[ch][offset..offset + chunk_size];

            // Safe byte-level copy
            for (i, &sample) in src.iter().enumerate() {
                let bytes = sample.to_ne_bytes();
                let byte_offset = i * 2;
                plane[byte_offset..byte_offset + 2].copy_from_slice(&bytes);
            }
        }

        // Send frame to encoder
        encoder.send_frame(&frame)
            .map_err(|e| format!("Failed to send frame: {}", e))?;

        // Receive and write packets
        receive_and_write_packets(&mut encoder, &mut output)?;

        samples_encoded += chunk_size;
    }

    // Flush encoder
    encoder.send_eof()
        .map_err(|e| format!("Failed to send EOF: {}", e))?;
    receive_and_write_packets(&mut encoder, &mut output)?;

    // Write trailer
    output.write_trailer()
        .map_err(|e| format!("Failed to write trailer: {}", e))?;

    println!("  Encoding complete - {} frames encoded", num_frames);

    Ok(())
}

/// Convert interleaved f32 samples to planar i16 format
fn convert_to_planar_i16(interleaved: &[f32], channels: u32) -> Vec<Vec<i16>> {
    let num_frames = interleaved.len() / channels as usize;
    let mut planar = vec![vec![0i16; num_frames]; channels as usize];

    for (i, chunk) in interleaved.chunks(channels as usize).enumerate() {
        for (ch, &sample) in chunk.iter().enumerate() {
            // Clamp and convert f32 (-1.0 to 1.0) to i16
            let clamped = sample.max(-1.0).min(1.0);
            planar[ch][i] = (clamped * 32767.0) as i16;
        }
    }

    planar
}

/// Receive encoded packets and write to output
fn receive_and_write_packets(
    encoder: &mut ffmpeg_next::encoder::Audio,
    output: &mut ffmpeg_next::format::context::Output,
) -> Result<(), String> {
    let mut encoded = ffmpeg_next::Packet::empty();

    while encoder.receive_packet(&mut encoded).is_ok() {
        encoded.set_stream(0);
        encoded.write_interleaved(output)
            .map_err(|e| format!("Failed to write packet: {}", e))?;
    }

    Ok(())
}

/// Encode raw PCM samples to AAC using ffmpeg-next
fn encode_pcm_to_aac(
    samples: &[f32],
    sample_rate: u32,
    channels: u32,
    bitrate_kbps: u32,
    output_path: &str,
) -> Result<(), String> {
    use ffmpeg_next as ffmpeg;

    // Find AAC encoder
    let encoder_codec = ffmpeg::encoder::find(ffmpeg::codec::Id::AAC)
        .ok_or("AAC encoder not found")?;

    println!("  Using encoder: {}", encoder_codec.name());

    // Create output format context
    let mut output = ffmpeg::format::output(&output_path)
        .map_err(|e| format!("Failed to create output file: {}", e))?;

    // Create encoder directly from codec
    let mut encoder = ffmpeg::codec::Context::new_with_codec(encoder_codec)
        .encoder()
        .audio()
        .map_err(|e| format!("Failed to create encoder: {}", e))?;

    println!("  Created encoder directly from codec");

    // Determine channel layout
    let channel_layout = match channels {
        1 => ffmpeg::channel_layout::ChannelLayout::MONO,
        2 => ffmpeg::channel_layout::ChannelLayout::STEREO,
        _ => return Err(format!("Unsupported channel count: {}", channels)),
    };

    // Configure encoder - AAC supports F32 Planar (fltp)
    encoder.set_rate(sample_rate as i32);
    encoder.set_channel_layout(channel_layout);
    encoder.set_format(ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Planar));
    encoder.set_bit_rate((bitrate_kbps * 1000) as usize);
    encoder.set_time_base(ffmpeg::Rational(1, sample_rate as i32));

    println!("  Encoder configured: {}Hz, {} channels, {} kbps",
             sample_rate, channels, bitrate_kbps);
    println!("  Format before open: {:?}", encoder.format());

    // Open encoder
    let mut encoder = encoder.open_as(encoder_codec)
        .map_err(|e| format!("Failed to open encoder: {}", e))?;

    println!("  ✅ Encoder opened successfully!");
    println!("  Opened encoder format: {:?}", encoder.format());

    // Add stream and set parameters
    {
        let mut stream = output.add_stream(encoder_codec)
            .map_err(|e| format!("Failed to add stream: {}", e))?;
        stream.set_parameters(&encoder);
    }

    // Write header
    output.write_header()
        .map_err(|e| format!("Failed to write header: {}", e))?;

    println!("  Encoding {} samples...", samples.len());

    // Convert interleaved f32 to planar f32
    let num_frames = samples.len() / channels as usize;
    let planar_samples = convert_to_planar_f32(samples, channels);

    // Get encoder frame size
    let frame_size = encoder.frame_size();
    let samples_per_frame = if frame_size > 0 {
        frame_size as usize
    } else {
        1024 // Default AAC frame size
    };

    println!("  Frame size: {} samples", samples_per_frame);

    // Encode in chunks
    let mut samples_encoded = 0;
    while samples_encoded < num_frames {
        let samples_remaining = num_frames - samples_encoded;
        let chunk_size = samples_remaining.min(samples_per_frame);

        // Create audio frame
        let mut frame = ffmpeg::frame::Audio::new(
            ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Planar),
            chunk_size,
            channel_layout,
        );
        frame.set_rate(sample_rate);

        // Copy planar samples to frame
        for ch in 0..channels as usize {
            let plane = frame.data_mut(ch);
            let offset = samples_encoded;
            let src = &planar_samples[ch][offset..offset + chunk_size];

            // Safe byte-level copy
            for (i, &sample) in src.iter().enumerate() {
                let bytes = sample.to_ne_bytes();
                let byte_offset = i * 4;
                plane[byte_offset..byte_offset + 4].copy_from_slice(&bytes);
            }
        }

        // Send frame to encoder
        encoder.send_frame(&frame)
            .map_err(|e| format!("Failed to send frame: {}", e))?;

        // Receive and write packets
        receive_and_write_packets(&mut encoder, &mut output)?;

        samples_encoded += chunk_size;
    }

    // Flush encoder
    encoder.send_eof()
        .map_err(|e| format!("Failed to send EOF: {}", e))?;
    receive_and_write_packets(&mut encoder, &mut output)?;

    // Write trailer
    output.write_trailer()
        .map_err(|e| format!("Failed to write trailer: {}", e))?;

    println!("  Encoding complete - {} frames encoded", num_frames);

    Ok(())
}

/// Convert interleaved f32 samples to planar f32 format
fn convert_to_planar_f32(interleaved: &[f32], channels: u32) -> Vec<Vec<f32>> {
    let num_frames = interleaved.len() / channels as usize;
    let mut planar = vec![vec![0.0f32; num_frames]; channels as usize];

    for (i, chunk) in interleaved.chunks(channels as usize).enumerate() {
        for (ch, &sample) in chunk.iter().enumerate() {
            planar[ch][i] = sample;
        }
    }

    planar
}

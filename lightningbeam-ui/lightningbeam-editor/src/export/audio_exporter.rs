#![allow(dead_code)]
//! Audio export functionality
//!
//! Exports audio from the timeline to various formats:
//! - WAV and FLAC: Use existing DAW backend export
//! - MP3 and AAC: Use FFmpeg encoding with rendered samples

use lightningbeam_core::export::{AudioExportSettings, AudioFormat};
use daw_backend::audio::{
    export::{ExportFormat, ExportSettings as DawExportSettings, render_to_memory},
    midi_pool::MidiClipPool,
    pool::AudioPool,
    project::Project,
};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Export audio to a file
///
/// This function routes to the appropriate export method based on the format:
/// - WAV/FLAC: Use DAW backend export
/// - MP3/AAC: Use FFmpeg encoding (TODO)
pub fn export_audio<P: AsRef<Path>>(
    project: &mut Project,
    pool: &AudioPool,
    midi_pool: &MidiClipPool,
    settings: &AudioExportSettings,
    output_path: P,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<(), String> {
    // Validate settings
    settings.validate()?;

    // Check for cancellation before starting
    if cancel_flag.load(Ordering::Relaxed) {
        return Err("Export cancelled by user".to_string());
    }

    match settings.format {
        AudioFormat::Wav | AudioFormat::Flac => {
            export_audio_daw_backend(project, pool, midi_pool, settings, output_path)
        }
        AudioFormat::Mp3 => {
            export_audio_ffmpeg_mp3(project, pool, midi_pool, settings, output_path, cancel_flag)
        }
        AudioFormat::Aac => {
            export_audio_ffmpeg_aac(project, pool, midi_pool, settings, output_path, cancel_flag)
        }
    }
}

/// Export audio using the DAW backend (WAV/FLAC)
fn export_audio_daw_backend<P: AsRef<Path>>(
    project: &mut Project,
    pool: &AudioPool,
    _midi_pool: &MidiClipPool,
    settings: &AudioExportSettings,
    output_path: P,
) -> Result<(), String> {
    // Convert our export settings to DAW backend format
    let daw_settings = DawExportSettings {
        format: match settings.format {
            AudioFormat::Wav => ExportFormat::Wav,
            AudioFormat::Flac => ExportFormat::Flac,
            _ => unreachable!(), // This function only handles WAV/FLAC
        },
        sample_rate: settings.sample_rate,
        channels: settings.channels,
        bit_depth: settings.bit_depth,
        mp3_bitrate: 320, // Not used for WAV/FLAC
        start_time: settings.start_time,
        end_time: settings.end_time,
    };

    // Use the existing DAW backend export function
    // No progress reporting for this direct export path
    daw_backend::audio::export::export_audio(
        project,
        pool,
        &daw_settings,
        output_path,
        None,
    )
}

/// Export audio as MP3 using FFmpeg
fn export_audio_ffmpeg_mp3<P: AsRef<Path>>(
    project: &mut Project,
    pool: &AudioPool,
    _midi_pool: &MidiClipPool,
    settings: &AudioExportSettings,
    output_path: P,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<(), String> {
    use ffmpeg_next as ffmpeg;

    // Initialize FFmpeg
    ffmpeg::init().map_err(|e| format!("Failed to initialize FFmpeg: {}", e))?;

    // Convert settings to DAW backend format
    let daw_settings = DawExportSettings {
        format: ExportFormat::Wav, // Unused, but required
        sample_rate: settings.sample_rate,
        channels: settings.channels,
        bit_depth: 16, // Unused
        mp3_bitrate: settings.bitrate_kbps,
        start_time: settings.start_time,
        end_time: settings.end_time,
    };

    // Step 1: Render audio to memory
    let pcm_samples = render_to_memory(
        project,
        pool,
        &daw_settings,
        None, // No progress events for now
    )?;

    // Check for cancellation
    if cancel_flag.load(Ordering::Relaxed) {
        return Err("Export cancelled".to_string());
    }

    // Step 2: Set up FFmpeg encoder
    let encoder_codec = ffmpeg::encoder::find(ffmpeg::codec::Id::MP3)
        .ok_or("MP3 encoder (libmp3lame) not found")?;

    // Create output file
    let mut output = ffmpeg::format::output(&output_path)
        .map_err(|e| format!("Failed to create output file: {}", e))?;

    // Create encoder
    let mut encoder = ffmpeg::codec::Context::new_with_codec(encoder_codec)
        .encoder()
        .audio()
        .map_err(|e| format!("Failed to create encoder: {}", e))?;

    // Configure encoder
    let channel_layout = match settings.channels {
        1 => ffmpeg::channel_layout::ChannelLayout::MONO,
        2 => ffmpeg::channel_layout::ChannelLayout::STEREO,
        _ => return Err(format!("Unsupported channel count: {}", settings.channels)),
    };

    encoder.set_rate(settings.sample_rate as i32);
    encoder.set_channel_layout(channel_layout);
    encoder.set_format(ffmpeg::format::Sample::I16(ffmpeg::format::sample::Type::Planar));
    encoder.set_bit_rate((settings.bitrate_kbps * 1000) as usize);
    encoder.set_time_base(ffmpeg::Rational(1, settings.sample_rate as i32));

    // Open encoder
    let mut encoder = encoder.open_as(encoder_codec)
        .map_err(|e| format!("Failed to open MP3 encoder: {}", e))?;

    // Add stream and set parameters
    {
        let mut stream = output.add_stream(encoder_codec)
            .map_err(|e| format!("Failed to add stream: {}", e))?;
        stream.set_parameters(&encoder);
    } // Drop stream here to release the borrow

    // Write header
    output.write_header()
        .map_err(|e| format!("Failed to write header: {}", e))?;

    // Step 3: Encode frames and write to output
    // Convert interleaved f32 samples to planar i16 format
    let num_frames = pcm_samples.len() / settings.channels as usize;
    let planar_samples = convert_to_planar_i16(&pcm_samples, settings.channels);

    // Get encoder frame size
    let frame_size = encoder.frame_size();
    let samples_per_frame = if frame_size > 0 {
        frame_size as usize
    } else {
        1152 // Default MP3 frame size
    };

    // Encode in chunks
    let mut samples_encoded = 0;
    while samples_encoded < num_frames {
        if cancel_flag.load(Ordering::Relaxed) {
            return Err("Export cancelled".to_string());
        }

        let samples_remaining = num_frames - samples_encoded;
        let chunk_size = samples_remaining.min(samples_per_frame);

        // Create audio frame
        let mut frame = ffmpeg::frame::Audio::new(
            ffmpeg::format::Sample::I16(ffmpeg::format::sample::Type::Planar),
            chunk_size,
            channel_layout,
        );
        frame.set_rate(settings.sample_rate);

        // Copy planar samples to frame
        // Use plane_mut::<i16> instead of data_mut — data_mut(ch) is buggy for planar audio:
        // FFmpeg only sets linesize[0], so data_mut returns 0-length slices for ch > 0.
        // plane_mut uses self.samples() for the length, which is correct for all planes.
        for ch in 0..settings.channels as usize {
            let plane = frame.plane_mut::<i16>(ch);
            let offset = samples_encoded;
            plane.copy_from_slice(&planar_samples[ch][offset..offset + chunk_size]);
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

/// Export audio as AAC using FFmpeg
fn export_audio_ffmpeg_aac<P: AsRef<Path>>(
    project: &mut Project,
    pool: &AudioPool,
    _midi_pool: &MidiClipPool,
    settings: &AudioExportSettings,
    output_path: P,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<(), String> {
    use ffmpeg_next as ffmpeg;

    // Initialize FFmpeg
    ffmpeg::init().map_err(|e| format!("Failed to initialize FFmpeg: {}", e))?;

    // Convert settings to DAW backend format
    let daw_settings = DawExportSettings {
        format: ExportFormat::Wav, // Unused, but required
        sample_rate: settings.sample_rate,
        channels: settings.channels,
        bit_depth: 16, // Unused
        mp3_bitrate: settings.bitrate_kbps,
        start_time: settings.start_time,
        end_time: settings.end_time,
    };

    // Step 1: Render audio to memory
    let pcm_samples = render_to_memory(
        project,
        pool,
        &daw_settings,
        None, // No progress events for now
    )?;

    // Check for cancellation
    if cancel_flag.load(Ordering::Relaxed) {
        return Err("Export cancelled".to_string());
    }

    // Step 2: Set up FFmpeg encoder
    let encoder_codec = ffmpeg::encoder::find(ffmpeg::codec::Id::AAC)
        .ok_or("AAC encoder not found")?;

    // Create output file
    let mut output = ffmpeg::format::output(&output_path)
        .map_err(|e| format!("Failed to create output file: {}", e))?;

    // Create encoder
    let mut encoder = ffmpeg::codec::Context::new_with_codec(encoder_codec)
        .encoder()
        .audio()
        .map_err(|e| format!("Failed to create encoder: {}", e))?;

    // Configure encoder
    let channel_layout = match settings.channels {
        1 => ffmpeg::channel_layout::ChannelLayout::MONO,
        2 => ffmpeg::channel_layout::ChannelLayout::STEREO,
        _ => return Err(format!("Unsupported channel count: {}", settings.channels)),
    };

    encoder.set_rate(settings.sample_rate as i32);
    encoder.set_channel_layout(channel_layout);
    // AAC encoder supports FLTP (F32 Planar) format
    encoder.set_format(ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Planar));
    encoder.set_bit_rate((settings.bitrate_kbps * 1000) as usize);
    encoder.set_time_base(ffmpeg::Rational(1, settings.sample_rate as i32));

    // Open encoder
    let mut encoder = encoder.open_as(encoder_codec)
        .map_err(|e| format!("Failed to open AAC encoder: {}", e))?;

    // Add stream and set parameters
    {
        let mut stream = output.add_stream(encoder_codec)
            .map_err(|e| format!("Failed to add stream: {}", e))?;
        stream.set_parameters(&encoder);
    } // Drop stream here to release the borrow

    // Write header
    output.write_header()
        .map_err(|e| format!("Failed to write header: {}", e))?;

    // Step 3: Encode frames and write to output
    // Convert interleaved f32 samples to planar f32 format (no conversion needed, just rearrange)
    let num_frames = pcm_samples.len() / settings.channels as usize;
    let planar_samples = convert_to_planar_f32(&pcm_samples, settings.channels);

    // Get encoder frame size
    let frame_size = encoder.frame_size();
    let samples_per_frame = if frame_size > 0 {
        frame_size as usize
    } else {
        1024 // Default AAC frame size
    };

    // Encode in chunks
    let mut samples_encoded = 0;
    while samples_encoded < num_frames {
        if cancel_flag.load(Ordering::Relaxed) {
            return Err("Export cancelled".to_string());
        }

        let samples_remaining = num_frames - samples_encoded;
        let chunk_size = samples_remaining.min(samples_per_frame);

        // Create audio frame
        let mut frame = ffmpeg::frame::Audio::new(
            ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Planar),
            chunk_size,
            channel_layout,
        );
        frame.set_rate(settings.sample_rate);

        // Copy planar samples to frame
        unsafe {
            for ch in 0..settings.channels as usize {
                let plane = frame.data_mut(ch);
                let offset = samples_encoded;
                let src = &planar_samples[ch][offset..offset + chunk_size];

                std::ptr::copy_nonoverlapping(
                    src.as_ptr() as *const u8,
                    plane.as_mut_ptr(),
                    chunk_size * std::mem::size_of::<f32>(),
                );
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

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_export_audio_validation() {
        let mut settings = AudioExportSettings::default();
        settings.sample_rate = 0; // Invalid

        let project = Project::new();
        let pool = AudioPool::new();
        let midi_pool = MidiClipPool::new();
        let cancel_flag = Arc::new(AtomicBool::new(false));

        let result = export_audio(
            &mut project.clone(),
            &pool,
            &midi_pool,
            &settings,
            "/tmp/test.wav",
            &cancel_flag,
        );

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Sample rate"));
    }

    #[test]
    fn test_export_audio_cancellation() {
        let settings = AudioExportSettings::default();
        let mut project = Project::new();
        let pool = AudioPool::new();
        let midi_pool = MidiClipPool::new();
        let cancel_flag = Arc::new(AtomicBool::new(true)); // Pre-cancelled

        let result = export_audio(
            &mut project,
            &pool,
            &midi_pool,
            &settings,
            "/tmp/test.wav",
            &cancel_flag,
        );

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("cancelled"));
    }
}

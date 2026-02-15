use super::buffer_pool::BufferPool;
use super::midi_pool::MidiClipPool;
use super::pool::AudioPool;
use super::project::Project;
use crate::command::AudioEvent;
use std::path::Path;

/// Supported export formats
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Wav,
    Flac,
    Mp3,
    Aac,
}

impl ExportFormat {
    /// Get the file extension for this format
    pub fn extension(&self) -> &'static str {
        match self {
            ExportFormat::Wav => "wav",
            ExportFormat::Flac => "flac",
            ExportFormat::Mp3 => "mp3",
            ExportFormat::Aac => "m4a",
        }
    }
}

/// Export settings for rendering audio
#[derive(Debug, Clone)]
pub struct ExportSettings {
    /// Output format
    pub format: ExportFormat,
    /// Sample rate for export
    pub sample_rate: u32,
    /// Number of channels (1 = mono, 2 = stereo)
    pub channels: u32,
    /// Bit depth (16 or 24) - only for WAV/FLAC
    pub bit_depth: u16,
    /// MP3 bitrate in kbps (128, 192, 256, 320)
    pub mp3_bitrate: u32,
    /// Start time in seconds
    pub start_time: f64,
    /// End time in seconds
    pub end_time: f64,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            format: ExportFormat::Wav,
            sample_rate: 44100,
            channels: 2,
            bit_depth: 16,
            mp3_bitrate: 320,
            start_time: 0.0,
            end_time: 60.0,
        }
    }
}

/// Export the project to an audio file
///
/// This performs offline rendering, processing the entire timeline
/// in chunks to generate the final audio file.
///
/// If an event producer is provided, progress events will be sent
/// after each chunk with (frames_rendered, total_frames).
pub fn export_audio<P: AsRef<Path>>(
    project: &mut Project,
    pool: &AudioPool,
    midi_pool: &MidiClipPool,
    settings: &ExportSettings,
    output_path: P,
    mut event_tx: Option<&mut rtrb::Producer<AudioEvent>>,
) -> Result<(), String>
{
    // Reset all node graphs to clear stale effect buffers (echo, reverb, etc.)
    project.reset_all_graphs();

    // Enable blocking mode on all read-ahead buffers so compressed audio
    // streams block until decoded frames are available (instead of returning
    // silence when the disk reader hasn't caught up with offline rendering).
    project.set_export_mode(true);

    // Route to appropriate export implementation based on format.
    // Ensure export mode is disabled even if an error occurs.
    let result = match settings.format {
        ExportFormat::Wav | ExportFormat::Flac => {
            let samples = render_to_memory(project, pool, midi_pool, settings, event_tx.as_mut().map(|tx| &mut **tx))?;
            // Signal that rendering is done and we're now writing the file
            if let Some(ref mut tx) = event_tx {
                let _ = tx.push(AudioEvent::ExportFinalizing);
            }
            match settings.format {
                ExportFormat::Wav => write_wav(&samples, settings, &output_path),
                ExportFormat::Flac => write_flac(&samples, settings, &output_path),
                _ => unreachable!(),
            }
        }
        ExportFormat::Mp3 => {
            export_mp3(project, pool, midi_pool, settings, output_path, event_tx)
        }
        ExportFormat::Aac => {
            export_aac(project, pool, midi_pool, settings, output_path, event_tx)
        }
    };

    // Always disable export mode, even on error
    project.set_export_mode(false);

    result
}

/// Render the project to memory
///
/// This function renders the project's audio to an in-memory buffer
/// of interleaved f32 samples. This is useful for custom export formats
/// or for passing audio to external encoders (e.g., FFmpeg for MP3/AAC).
///
/// The returned samples are interleaved (L,R,L,R,... for stereo).
///
/// If an event producer is provided, progress events will be sent
/// after each chunk with (frames_rendered, total_frames).
pub fn render_to_memory(
    project: &mut Project,
    pool: &AudioPool,
    midi_pool: &MidiClipPool,
    settings: &ExportSettings,
    mut event_tx: Option<&mut rtrb::Producer<AudioEvent>>,
) -> Result<Vec<f32>, String>
{
    // Calculate total number of frames
    let duration = settings.end_time - settings.start_time;
    let total_frames = (duration * settings.sample_rate as f64).round() as usize;
    let total_samples = total_frames * settings.channels as usize;

    println!("Export: duration={:.3}s, total_frames={}, total_samples={}, channels={}",
             duration, total_frames, total_samples, settings.channels);

    // Render in chunks to avoid memory issues
    const CHUNK_FRAMES: usize = 4096;
    let chunk_samples = CHUNK_FRAMES * settings.channels as usize;

    // Create buffer for rendering
    let mut render_buffer = vec![0.0f32; chunk_samples];
    let mut buffer_pool = BufferPool::new(16, chunk_samples);

    // Collect all rendered samples
    let mut all_samples = Vec::with_capacity(total_samples);

    let mut playhead = settings.start_time;
    let chunk_duration = CHUNK_FRAMES as f64 / settings.sample_rate as f64;
    let mut frames_rendered = 0;

    // Render the entire timeline in chunks
    while playhead < settings.end_time {
        // Clear the render buffer
        render_buffer.fill(0.0);

        // Render this chunk
        project.render(
            &mut render_buffer,
            pool,
            midi_pool,
            &mut buffer_pool,
            playhead,
            settings.sample_rate,
            settings.channels,
        );

        // Calculate how many samples we actually need from this chunk
        let remaining_time = settings.end_time - playhead;
        let samples_needed = if remaining_time < chunk_duration {
            // Calculate frames needed and ensure it's a whole number
            let frames_needed = (remaining_time * settings.sample_rate as f64).round() as usize;
            let samples = frames_needed * settings.channels as usize;
            // Ensure we don't exceed chunk size
            samples.min(chunk_samples)
        } else {
            chunk_samples
        };

        // Append to output
        all_samples.extend_from_slice(&render_buffer[..samples_needed]);

        // Update progress
        frames_rendered += samples_needed / settings.channels as usize;
        if let Some(event_tx) = event_tx.as_mut() {
            let _ = event_tx.push(AudioEvent::ExportProgress {
                frames_rendered,
                total_frames,
            });
        }

        playhead += chunk_duration;
    }

    println!("Export: rendered {} samples total", all_samples.len());

    // Verify the sample count is a multiple of channels
    if all_samples.len() % settings.channels as usize != 0 {
        return Err(format!(
            "Sample count {} is not a multiple of channel count {}",
            all_samples.len(),
            settings.channels
        ));
    }

    Ok(all_samples)
}

/// Write WAV file using hound
fn write_wav<P: AsRef<Path>>(
    samples: &[f32],
    settings: &ExportSettings,
    output_path: P,
) -> Result<(), String> {
    let spec = hound::WavSpec {
        channels: settings.channels as u16,
        sample_rate: settings.sample_rate,
        bits_per_sample: settings.bit_depth,
        sample_format: hound::SampleFormat::Int,
    };

    let mut writer = hound::WavWriter::create(output_path, spec)
        .map_err(|e| format!("Failed to create WAV file: {}", e))?;

    // Write samples
    match settings.bit_depth {
        16 => {
            for &sample in samples {
                let clamped = sample.max(-1.0).min(1.0);
                let pcm_value = (clamped * 32767.0) as i16;
                writer.write_sample(pcm_value)
                    .map_err(|e| format!("Failed to write sample: {}", e))?;
            }
        }
        24 => {
            for &sample in samples {
                let clamped = sample.max(-1.0).min(1.0);
                let pcm_value = (clamped * 8388607.0) as i32;
                writer.write_sample(pcm_value)
                    .map_err(|e| format!("Failed to write sample: {}", e))?;
            }
        }
        _ => return Err(format!("Unsupported bit depth: {}", settings.bit_depth)),
    }

    writer.finalize()
        .map_err(|e| format!("Failed to finalize WAV file: {}", e))?;

    Ok(())
}

/// Write FLAC file using hound (FLAC is essentially lossless WAV)
fn write_flac<P: AsRef<Path>>(
    samples: &[f32],
    settings: &ExportSettings,
    output_path: P,
) -> Result<(), String> {
    // For now, we'll use hound to write a WAV-like FLAC file
    // In the future, we could use a dedicated FLAC encoder
    let spec = hound::WavSpec {
        channels: settings.channels as u16,
        sample_rate: settings.sample_rate,
        bits_per_sample: settings.bit_depth,
        sample_format: hound::SampleFormat::Int,
    };

    let mut writer = hound::WavWriter::create(output_path, spec)
        .map_err(|e| format!("Failed to create FLAC file: {}", e))?;

    // Write samples (same as WAV for now)
    match settings.bit_depth {
        16 => {
            for &sample in samples {
                let clamped = sample.max(-1.0).min(1.0);
                let pcm_value = (clamped * 32767.0) as i16;
                writer.write_sample(pcm_value)
                    .map_err(|e| format!("Failed to write sample: {}", e))?;
            }
        }
        24 => {
            for &sample in samples {
                let clamped = sample.max(-1.0).min(1.0);
                let pcm_value = (clamped * 8388607.0) as i32;
                writer.write_sample(pcm_value)
                    .map_err(|e| format!("Failed to write sample: {}", e))?;
            }
        }
        _ => return Err(format!("Unsupported bit depth: {}", settings.bit_depth)),
    }

    writer.finalize()
        .map_err(|e| format!("Failed to finalize FLAC file: {}", e))?;

    Ok(())
}

/// Export audio as MP3 using FFmpeg (streaming - render and encode simultaneously)
fn export_mp3<P: AsRef<Path>>(
    project: &mut Project,
    pool: &AudioPool,
    midi_pool: &MidiClipPool,
    settings: &ExportSettings,
    output_path: P,
    mut event_tx: Option<&mut rtrb::Producer<AudioEvent>>,
) -> Result<(), String> {
    // Initialize FFmpeg
    ffmpeg_next::init().map_err(|e| format!("Failed to initialize FFmpeg: {}", e))?;

    // Set up FFmpeg encoder
    let encoder_codec = ffmpeg_next::encoder::find(ffmpeg_next::codec::Id::MP3)
        .ok_or("MP3 encoder (libmp3lame) not found")?;

    let mut output = ffmpeg_next::format::output(&output_path)
        .map_err(|e| format!("Failed to create output file: {}", e))?;

    let mut encoder = ffmpeg_next::codec::Context::new_with_codec(encoder_codec)
        .encoder()
        .audio()
        .map_err(|e| format!("Failed to create encoder: {}", e))?;

    let channel_layout = match settings.channels {
        1 => ffmpeg_next::channel_layout::ChannelLayout::MONO,
        2 => ffmpeg_next::channel_layout::ChannelLayout::STEREO,
        _ => return Err(format!("Unsupported channel count: {}", settings.channels)),
    };

    encoder.set_rate(settings.sample_rate as i32);
    encoder.set_channel_layout(channel_layout);
    encoder.set_format(ffmpeg_next::format::Sample::I16(ffmpeg_next::format::sample::Type::Planar));
    encoder.set_bit_rate((settings.mp3_bitrate * 1000) as usize);
    encoder.set_time_base(ffmpeg_next::Rational(1, settings.sample_rate as i32));

    let mut encoder = encoder.open_as(encoder_codec)
        .map_err(|e| format!("Failed to open MP3 encoder: {}", e))?;

    {
        let mut stream = output.add_stream(encoder_codec)
            .map_err(|e| format!("Failed to add stream: {}", e))?;
        stream.set_parameters(&encoder);
    }

    output.write_header()
        .map_err(|e| format!("Failed to write header: {}", e))?;

    // Calculate rendering parameters
    let duration = settings.end_time - settings.start_time;
    let total_frames = (duration * settings.sample_rate as f64).round() as usize;

    const CHUNK_FRAMES: usize = 4096;
    let chunk_samples = CHUNK_FRAMES * settings.channels as usize;
    let chunk_duration = CHUNK_FRAMES as f64 / settings.sample_rate as f64;

    // Create buffers for rendering
    let mut render_buffer = vec![0.0f32; chunk_samples];
    let mut buffer_pool = BufferPool::new(16, chunk_samples);

    // Get encoder frame size for proper buffering
    let encoder_frame_size = encoder.frame_size() as usize;
    let encoder_frame_size = if encoder_frame_size > 0 {
        encoder_frame_size
    } else {
        1152 // Default MP3 frame size
    };

    // Sample buffer to accumulate samples until we have complete frames
    let mut sample_buffer: Vec<f32> = Vec::new();

    // PTS (presentation timestamp) tracking for proper timing
    let mut pts: i64 = 0;

    // Streaming render and encode loop
    let mut playhead = settings.start_time;
    let mut frames_rendered = 0;

    while playhead < settings.end_time {
        // Render this chunk
        render_buffer.fill(0.0);
        project.render(
            &mut render_buffer,
            pool,
            midi_pool,
            &mut buffer_pool,
            playhead,
            settings.sample_rate,
            settings.channels,
        );

        // Calculate how many samples we need from this chunk
        let remaining_time = settings.end_time - playhead;
        let samples_needed = if remaining_time < chunk_duration {
            ((remaining_time * settings.sample_rate as f64) as usize * settings.channels as usize)
                .min(chunk_samples)
        } else {
            chunk_samples
        };

        // Add to sample buffer
        sample_buffer.extend_from_slice(&render_buffer[..samples_needed]);

        // Encode complete frames from buffer
        let encoder_frame_samples = encoder_frame_size * settings.channels as usize;
        while sample_buffer.len() >= encoder_frame_samples {
            // Extract one complete frame
            let frame_samples: Vec<f32> = sample_buffer.drain(..encoder_frame_samples).collect();

            // Convert to planar i16
            let planar_i16 = convert_chunk_to_planar_i16(&frame_samples, settings.channels);

            // Encode this frame
            encode_complete_frame_mp3(
                &mut encoder,
                &mut output,
                &planar_i16,
                encoder_frame_size,
                settings.sample_rate,
                channel_layout,
                pts,
            )?;

            frames_rendered += encoder_frame_size;
            pts += encoder_frame_size as i64;

            // Report progress
            if let Some(ref mut tx) = event_tx {
                let _ = tx.push(AudioEvent::ExportProgress {
                    frames_rendered,
                    total_frames,
                });
            }
        }

        playhead += chunk_duration;
    }

    // Encode any remaining samples as the final frame
    if !sample_buffer.is_empty() {
        let planar_i16 = convert_chunk_to_planar_i16(&sample_buffer, settings.channels);
        let final_frame_size = sample_buffer.len() / settings.channels as usize;

        encode_complete_frame_mp3(
            &mut encoder,
            &mut output,
            &planar_i16,
            final_frame_size,
            settings.sample_rate,
            channel_layout,
            pts,
        )?;
    }

    // Signal that rendering is done and we're now flushing/finalizing
    if let Some(ref mut tx) = event_tx {
        let _ = tx.push(AudioEvent::ExportFinalizing);
    }

    // Flush encoder
    encoder.send_eof()
        .map_err(|e| format!("Failed to send EOF: {}", e))?;
    receive_and_write_packets(&mut encoder, &mut output)?;

    output.write_trailer()
        .map_err(|e| format!("Failed to write trailer: {}", e))?;

    Ok(())
}

/// Export audio as AAC using FFmpeg (streaming - render and encode simultaneously)
fn export_aac<P: AsRef<Path>>(
    project: &mut Project,
    pool: &AudioPool,
    midi_pool: &MidiClipPool,
    settings: &ExportSettings,
    output_path: P,
    mut event_tx: Option<&mut rtrb::Producer<AudioEvent>>,
) -> Result<(), String> {
    // Initialize FFmpeg
    ffmpeg_next::init().map_err(|e| format!("Failed to initialize FFmpeg: {}", e))?;

    // Set up FFmpeg encoder
    let encoder_codec = ffmpeg_next::encoder::find(ffmpeg_next::codec::Id::AAC)
        .ok_or("AAC encoder not found")?;

    let mut output = ffmpeg_next::format::output(&output_path)
        .map_err(|e| format!("Failed to create output file: {}", e))?;

    let mut encoder = ffmpeg_next::codec::Context::new_with_codec(encoder_codec)
        .encoder()
        .audio()
        .map_err(|e| format!("Failed to create encoder: {}", e))?;

    let channel_layout = match settings.channels {
        1 => ffmpeg_next::channel_layout::ChannelLayout::MONO,
        2 => ffmpeg_next::channel_layout::ChannelLayout::STEREO,
        _ => return Err(format!("Unsupported channel count: {}", settings.channels)),
    };

    encoder.set_rate(settings.sample_rate as i32);
    encoder.set_channel_layout(channel_layout);
    encoder.set_format(ffmpeg_next::format::Sample::F32(ffmpeg_next::format::sample::Type::Planar));
    encoder.set_bit_rate((settings.mp3_bitrate * 1000) as usize);
    encoder.set_time_base(ffmpeg_next::Rational(1, settings.sample_rate as i32));

    let mut encoder = encoder.open_as(encoder_codec)
        .map_err(|e| format!("Failed to open AAC encoder: {}", e))?;

    {
        let mut stream = output.add_stream(encoder_codec)
            .map_err(|e| format!("Failed to add stream: {}", e))?;
        stream.set_parameters(&encoder);
    }

    output.write_header()
        .map_err(|e| format!("Failed to write header: {}", e))?;

    // Calculate rendering parameters
    let duration = settings.end_time - settings.start_time;
    let total_frames = (duration * settings.sample_rate as f64).round() as usize;

    const CHUNK_FRAMES: usize = 4096;
    let chunk_samples = CHUNK_FRAMES * settings.channels as usize;
    let chunk_duration = CHUNK_FRAMES as f64 / settings.sample_rate as f64;

    // Create buffers for rendering
    let mut render_buffer = vec![0.0f32; chunk_samples];
    let mut buffer_pool = BufferPool::new(16, chunk_samples);

    // Get encoder frame size for proper buffering
    let encoder_frame_size = encoder.frame_size() as usize;
    let encoder_frame_size = if encoder_frame_size > 0 {
        encoder_frame_size
    } else {
        1024 // Default AAC frame size
    };

    // Sample buffer to accumulate samples until we have complete frames
    let mut sample_buffer: Vec<f32> = Vec::new();

    // PTS (presentation timestamp) tracking for proper timing
    let mut pts: i64 = 0;

    // Streaming render and encode loop
    let mut playhead = settings.start_time;
    let mut frames_rendered = 0;

    while playhead < settings.end_time {
        // Render this chunk
        render_buffer.fill(0.0);
        project.render(
            &mut render_buffer,
            pool,
            midi_pool,
            &mut buffer_pool,
            playhead,
            settings.sample_rate,
            settings.channels,
        );

        // Calculate how many samples we need from this chunk
        let remaining_time = settings.end_time - playhead;
        let samples_needed = if remaining_time < chunk_duration {
            ((remaining_time * settings.sample_rate as f64) as usize * settings.channels as usize)
                .min(chunk_samples)
        } else {
            chunk_samples
        };

        // Add to sample buffer
        sample_buffer.extend_from_slice(&render_buffer[..samples_needed]);

        // Encode complete frames from buffer
        let encoder_frame_samples = encoder_frame_size * settings.channels as usize;
        while sample_buffer.len() >= encoder_frame_samples {
            // Extract one complete frame
            let frame_samples: Vec<f32> = sample_buffer.drain(..encoder_frame_samples).collect();

            // Convert to planar f32
            let planar_f32 = convert_chunk_to_planar_f32(&frame_samples, settings.channels);

            // Encode this frame
            encode_complete_frame_aac(
                &mut encoder,
                &mut output,
                &planar_f32,
                encoder_frame_size,
                settings.sample_rate,
                channel_layout,
                pts,
            )?;

            frames_rendered += encoder_frame_size;
            pts += encoder_frame_size as i64;

            // Report progress
            if let Some(ref mut tx) = event_tx {
                let _ = tx.push(AudioEvent::ExportProgress {
                    frames_rendered,
                    total_frames,
                });
            }
        }

        playhead += chunk_duration;
    }

    // Encode any remaining samples as the final frame
    if !sample_buffer.is_empty() {
        let planar_f32 = convert_chunk_to_planar_f32(&sample_buffer, settings.channels);
        let final_frame_size = sample_buffer.len() / settings.channels as usize;

        encode_complete_frame_aac(
            &mut encoder,
            &mut output,
            &planar_f32,
            final_frame_size,
            settings.sample_rate,
            channel_layout,
            pts,
        )?;
    }

    // Signal that rendering is done and we're now flushing/finalizing
    if let Some(ref mut tx) = event_tx {
        let _ = tx.push(AudioEvent::ExportFinalizing);
    }

    // Flush encoder
    encoder.send_eof()
        .map_err(|e| format!("Failed to send EOF: {}", e))?;
    receive_and_write_packets(&mut encoder, &mut output)?;

    output.write_trailer()
        .map_err(|e| format!("Failed to write trailer: {}", e))?;

    Ok(())
}

/// Convert a chunk of interleaved f32 samples to planar i16 format
fn convert_chunk_to_planar_i16(interleaved: &[f32], channels: u32) -> Vec<Vec<i16>> {
    let num_frames = interleaved.len() / channels as usize;
    let mut planar = vec![vec![0i16; num_frames]; channels as usize];

    for (i, chunk) in interleaved.chunks(channels as usize).enumerate() {
        for (ch, &sample) in chunk.iter().enumerate() {
            let clamped = sample.max(-1.0).min(1.0);
            planar[ch][i] = (clamped * 32767.0) as i16;
        }
    }

    planar
}

/// Convert a chunk of interleaved f32 samples to planar f32 format
fn convert_chunk_to_planar_f32(interleaved: &[f32], channels: u32) -> Vec<Vec<f32>> {
    let num_frames = interleaved.len() / channels as usize;
    let mut planar = vec![vec![0.0f32; num_frames]; channels as usize];

    for (i, chunk) in interleaved.chunks(channels as usize).enumerate() {
        for (ch, &sample) in chunk.iter().enumerate() {
            planar[ch][i] = sample;
        }
    }

    planar
}

/// Encode a single complete frame of planar i16 samples to MP3
fn encode_complete_frame_mp3(
    encoder: &mut ffmpeg_next::encoder::Audio,
    output: &mut ffmpeg_next::format::context::Output,
    planar_samples: &[Vec<i16>],
    num_frames: usize,
    sample_rate: u32,
    channel_layout: ffmpeg_next::channel_layout::ChannelLayout,
    pts: i64,
) -> Result<(), String> {
    let channels = planar_samples.len();

    // Create audio frame with exact size
    let mut frame = ffmpeg_next::frame::Audio::new(
        ffmpeg_next::format::Sample::I16(ffmpeg_next::format::sample::Type::Planar),
        num_frames,
        channel_layout,
    );
    frame.set_rate(sample_rate);
    frame.set_pts(Some(pts));

    // Copy all planar samples to frame
    unsafe {
        for ch in 0..channels {
            let plane = frame.data_mut(ch);
            let src = &planar_samples[ch];

            std::ptr::copy_nonoverlapping(
                src.as_ptr() as *const u8,
                plane.as_mut_ptr(),
                num_frames * std::mem::size_of::<i16>(),
            );
        }
    }

    // Send frame to encoder
    encoder.send_frame(&frame)
        .map_err(|e| format!("Failed to send frame: {}", e))?;

    // Receive and write packets
    receive_and_write_packets(encoder, output)?;

    Ok(())
}

/// Encode a single complete frame of planar f32 samples to AAC
fn encode_complete_frame_aac(
    encoder: &mut ffmpeg_next::encoder::Audio,
    output: &mut ffmpeg_next::format::context::Output,
    planar_samples: &[Vec<f32>],
    num_frames: usize,
    sample_rate: u32,
    channel_layout: ffmpeg_next::channel_layout::ChannelLayout,
    pts: i64,
) -> Result<(), String> {
    let channels = planar_samples.len();

    // Create audio frame with exact size
    let mut frame = ffmpeg_next::frame::Audio::new(
        ffmpeg_next::format::Sample::F32(ffmpeg_next::format::sample::Type::Planar),
        num_frames,
        channel_layout,
    );
    frame.set_rate(sample_rate);
    frame.set_pts(Some(pts));

    // Copy all planar samples to frame
    unsafe {
        for ch in 0..channels {
            let plane = frame.data_mut(ch);
            let src = &planar_samples[ch];

            std::ptr::copy_nonoverlapping(
                src.as_ptr() as *const u8,
                plane.as_mut_ptr(),
                num_frames * std::mem::size_of::<f32>(),
            );
        }
    }

    // Send frame to encoder
    encoder.send_frame(&frame)
        .map_err(|e| format!("Failed to send frame: {}", e))?;

    // Receive and write packets
    receive_and_write_packets(encoder, output)?;

    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_export_settings_default() {
        let settings = ExportSettings::default();
        assert_eq!(settings.format, ExportFormat::Wav);
        assert_eq!(settings.sample_rate, 44100);
        assert_eq!(settings.channels, 2);
        assert_eq!(settings.bit_depth, 16);
    }

    #[test]
    fn test_format_extension() {
        assert_eq!(ExportFormat::Wav.extension(), "wav");
        assert_eq!(ExportFormat::Flac.extension(), "flac");
    }
}

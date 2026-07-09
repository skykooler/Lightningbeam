use super::buffer_pool::BufferPool;
use super::pool::AudioPool;
use super::project::Project;
use crate::command::AudioEvent;
use crate::tempo_map::TempoMap;
use crate::time::Seconds;
use std::path::Path;

/// Render chunk size for offline export. Matches the real-time playback buffer size
/// so that MIDI events are processed at the same granularity, avoiding timing jitter.
const EXPORT_CHUNK_FRAMES: usize = 256;

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
    /// Start time
    pub start_time: Seconds,
    /// End time
    pub end_time: Seconds,
    /// Tempo map for beat-position scheduling
    pub tempo_map: TempoMap,
    /// Tag metadata as (ffmpeg-key, value) pairs (e.g. ("title", "…"), ("artist", "…")). Written to
    /// the container's native tags: ID3v2 (MP3), MP4 atoms (M4A), Vorbis comments (FLAC), RIFF INFO
    /// (WAV). Empty = no tags.
    pub metadata: Vec<(String, String)>,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            format: ExportFormat::Wav,
            sample_rate: 44100,
            channels: 2,
            bit_depth: 16,
            mp3_bitrate: 320,
            start_time: Seconds::ZERO,
            end_time: Seconds(60.0),
            tempo_map: TempoMap::constant(120.0),
            metadata: Vec::new(),
        }
    }
}

/// Set tag metadata on an ffmpeg output context (before `write_header`). FFmpeg maps the standard
/// keys to each container's native tags.
fn apply_metadata(output: &mut ffmpeg_next::format::context::Output, metadata: &[(String, String)]) {
    if metadata.is_empty() {
        return;
    }
    let mut dict = ffmpeg_next::Dictionary::new();
    for (k, v) in metadata {
        if !v.is_empty() {
            dict.set(k, v);
        }
    }
    output.set_metadata(dict);
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
    settings: &ExportSettings,
    output_path: P,
    mut event_tx: Option<&mut rtrb::Producer<AudioEvent>>,
) -> Result<(), String>
{
    // Validate duration
    let duration = settings.end_time - settings.start_time;
    if duration <= Seconds::ZERO {
        return Err(format!(
            "Export duration is zero or negative (start={:.3}s, end={:.3}s). \
             Check that the timeline has content.",
            settings.start_time.seconds_to_f64(), settings.end_time.seconds_to_f64()
        ));
    }

    let total_frames = (duration.seconds_to_f64() * settings.sample_rate as f64).round() as usize;
    if total_frames == 0 {
        return Err("Export would produce zero audio frames".to_string());
    }

    // Reset all node graphs to clear stale effect buffers (echo, reverb, etc.)
    project.reset_all_graphs();

    // Enable blocking mode on all read-ahead buffers so compressed audio
    // streams block until decoded frames are available (instead of returning
    // silence when the disk reader hasn't caught up with offline rendering).
    project.set_export_mode(true);

    // Route to appropriate export implementation based on format.
    // Ensure export mode is disabled even if an error occurs.
    let result = match settings.format {
        ExportFormat::Wav => {
            let samples = render_to_memory(project, pool, settings, event_tx.as_mut().map(|tx| &mut **tx))?;
            if let Some(ref mut tx) = event_tx {
                let _ = tx.push(AudioEvent::ExportFinalizing);
            }
            write_wav(&samples, settings, &output_path)
                // hound writes no metadata; append a RIFF INFO chunk for tags.
                .and_then(|_| append_wav_info_chunk(output_path.as_ref(), &settings.metadata))
        }
        ExportFormat::Flac => {
            let samples = render_to_memory(project, pool, settings, event_tx.as_mut().map(|tx| &mut **tx))?;
            if let Some(ref mut tx) = event_tx {
                let _ = tx.push(AudioEvent::ExportFinalizing);
            }
            export_flac(&samples, settings, &output_path)
        }
        ExportFormat::Mp3 => {
            export_mp3(project, pool, settings, output_path, event_tx)
        }
        ExportFormat::Aac => {
            export_aac(project, pool, settings, output_path, event_tx)
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
    settings: &ExportSettings,
    mut event_tx: Option<&mut rtrb::Producer<AudioEvent>>,
) -> Result<Vec<f32>, String>
{
    // Calculate total number of frames
    let duration = settings.end_time - settings.start_time;
    let total_frames = (duration.seconds_to_f64() * settings.sample_rate as f64).round() as usize;
    let total_samples = total_frames * settings.channels as usize;

    println!("Export: duration={:.3}s, total_frames={}, total_samples={}, channels={}",
             duration.seconds_to_f64(), total_frames, total_samples, settings.channels);

    let chunk_samples = EXPORT_CHUNK_FRAMES * settings.channels as usize;

    // Create buffer for rendering
    let mut render_buffer = vec![0.0f32; chunk_samples];
    let mut buffer_pool = BufferPool::new(16, chunk_samples);

    // Collect all rendered samples
    let mut all_samples = Vec::with_capacity(total_samples);

    let mut playhead = settings.start_time;
    let chunk_duration = EXPORT_CHUNK_FRAMES as f64 / settings.sample_rate as f64;
    let mut frames_rendered = 0;

    // Render the entire timeline in chunks
    while playhead < settings.end_time {
        // Clear the render buffer
        render_buffer.fill(0.0);

        // Render this chunk
        project.render(
            &mut render_buffer,
            pool,
            &mut buffer_pool,
            playhead,
            &settings.tempo_map,
            settings.sample_rate,
            settings.channels,
            false,
        );

        // Calculate how many samples we actually need from this chunk
        let remaining_time = settings.end_time - playhead;
        let samples_needed = if remaining_time.seconds_to_f64() < chunk_duration {
            // Calculate frames needed and ensure it's a whole number
            let frames_needed = (remaining_time.seconds_to_f64() * settings.sample_rate as f64).round() as usize;
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

        playhead = playhead + Seconds(chunk_duration);
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

/// Export real FLAC via ffmpeg from already-rendered interleaved f32 samples (Vorbis-comment
/// metadata). Replaces the former `write_flac`, which wrote WAV bytes to a `.flac` file. 16-bit
/// uses S16; 24-bit uses S32 (ffmpeg's flac encoder emits `bits_per_raw_sample = 24` for S32,
/// taking the top 24 bits).
fn export_flac<P: AsRef<Path>>(
    samples: &[f32],
    settings: &ExportSettings,
    output_path: P,
) -> Result<(), String> {
    use ffmpeg_next as ffmpeg;

    ffmpeg::init().map_err(|e| format!("Failed to initialize FFmpeg: {}", e))?;

    let codec = ffmpeg::encoder::find(ffmpeg::codec::Id::FLAC)
        .ok_or("FLAC encoder not found in this ffmpeg build")?;
    let mut output = ffmpeg::format::output(&output_path)
        .map_err(|e| format!("Failed to create output file: {}", e))?;

    let channel_layout = match settings.channels {
        1 => ffmpeg::channel_layout::ChannelLayout::MONO,
        2 => ffmpeg::channel_layout::ChannelLayout::STEREO,
        _ => return Err(format!("Unsupported channel count: {}", settings.channels)),
    };

    // FLAC accepts packed S16 or S32; S32 → 24-bit output.
    let use_24 = settings.bit_depth >= 24;
    let sample_fmt = if use_24 {
        ffmpeg::format::Sample::I32(ffmpeg::format::sample::Type::Packed)
    } else {
        ffmpeg::format::Sample::I16(ffmpeg::format::sample::Type::Packed)
    };

    let mut encoder = ffmpeg::codec::Context::new_with_codec(codec)
        .encoder()
        .audio()
        .map_err(|e| format!("Failed to create FLAC encoder: {}", e))?;
    encoder.set_rate(settings.sample_rate as i32);
    encoder.set_channel_layout(channel_layout);
    encoder.set_format(sample_fmt);
    encoder.set_time_base(ffmpeg::Rational(1, settings.sample_rate as i32));
    let mut encoder = encoder.open_as(codec)
        .map_err(|e| format!("Failed to open FLAC encoder: {}", e))?;

    {
        let mut stream = output.add_stream(codec)
            .map_err(|e| format!("Failed to add stream: {}", e))?;
        stream.set_parameters(&encoder);
    }
    apply_metadata(&mut output, &settings.metadata);
    output.write_header()
        .map_err(|e| format!("Failed to write FLAC header: {}", e))?;

    let channels = settings.channels as usize;
    let num_frames = samples.len() / channels;
    let frame_size = if encoder.frame_size() > 0 { encoder.frame_size() as usize } else { 4096 };

    let mut done = 0usize;
    while done < num_frames {
        let n = (num_frames - done).min(frame_size);
        let mut frame = ffmpeg::frame::Audio::new(sample_fmt, n, channel_layout);
        frame.set_rate(settings.sample_rate);
        frame.set_pts(Some(done as i64)); // samples; the FLAC muxer requires PTS

        let buf = frame.data_mut(0); // packed interleaved → plane 0
        let base = done * channels;
        if use_24 {
            for i in 0..n * channels {
                let s = samples[base + i].clamp(-1.0, 1.0);
                let v = (s as f64 * 2_147_483_647.0) as i32; // full-scale S32; encoder takes top 24
                buf[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
            }
        } else {
            for i in 0..n * channels {
                let s = samples[base + i].clamp(-1.0, 1.0);
                let v = (s * 32767.0) as i16;
                buf[i * 2..i * 2 + 2].copy_from_slice(&v.to_le_bytes());
            }
        }

        encoder.send_frame(&frame).map_err(|e| format!("Failed to send FLAC frame: {}", e))?;
        flac_write_packets(&mut encoder, &mut output)?;
        done += n;
    }
    encoder.send_eof().map_err(|e| format!("Failed to flush FLAC encoder: {}", e))?;
    flac_write_packets(&mut encoder, &mut output)?;
    output.write_trailer().map_err(|e| format!("Failed to finalize FLAC: {}", e))?;
    Ok(())
}

/// Drain encoded FLAC packets and write them (non-interleaved). Skips the trailing empty flush
/// packet, which the FLAC muxer otherwise rejects as "Invalid data". Rescales packet ts from the
/// encoder time base to the stream's.
fn flac_write_packets(
    encoder: &mut ffmpeg_next::encoder::Audio,
    output: &mut ffmpeg_next::format::context::Output,
) -> Result<(), String> {
    let mut pkt = ffmpeg_next::Packet::empty();
    let enc_tb = encoder.time_base();
    let stream_tb = output.stream(0).map(|s| s.time_base()).unwrap_or(enc_tb);
    while encoder.receive_packet(&mut pkt).is_ok() {
        if pkt.size() == 0 {
            continue;
        }
        pkt.set_stream(0);
        pkt.rescale_ts(enc_tb, stream_tb);
        pkt.write(output).map_err(|e| format!("Failed to write FLAC packet: {}", e))?;
    }
    Ok(())
}

/// Append a RIFF `LIST`/`INFO` metadata chunk to a finished WAV file (hound writes no tags), then
/// fix up the top-level RIFF size. Maps ffmpeg-style keys to RIFF INFO sub-chunk IDs. Trailing INFO
/// chunks are ignored by players that don't read them.
fn append_wav_info_chunk(path: &Path, metadata: &[(String, String)]) -> Result<(), String> {
    use std::io::{Seek, SeekFrom, Write};

    let riff_id = |key: &str| -> Option<&'static [u8; 4]> {
        match key {
            "title" => Some(b"INAM"),
            "artist" => Some(b"IART"),
            "album" => Some(b"IPRD"),
            "genre" => Some(b"IGNR"),
            "comment" => Some(b"ICMT"),
            "date" => Some(b"ICRD"),
            "track" => Some(b"ITRK"),
            _ => None,
        }
    };

    let mut info: Vec<u8> = Vec::new();
    info.extend_from_slice(b"INFO");
    for (key, val) in metadata {
        if val.is_empty() {
            continue;
        }
        let Some(id) = riff_id(key) else { continue };
        let mut bytes = val.as_bytes().to_vec();
        bytes.push(0); // NUL-terminate
        if bytes.len() % 2 == 1 {
            bytes.push(0); // pad to even
        }
        info.extend_from_slice(id);
        info.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        info.extend_from_slice(&bytes);
    }
    if info.len() <= 4 {
        return Ok(()); // nothing but the "INFO" tag
    }

    let mut list: Vec<u8> = Vec::with_capacity(info.len() + 8);
    list.extend_from_slice(b"LIST");
    list.extend_from_slice(&(info.len() as u32).to_le_bytes());
    list.extend_from_slice(&info);

    let mut f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| format!("Failed to open WAV for tagging: {}", e))?;
    let end = f.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?;
    if end % 2 == 1 {
        f.write_all(&[0]).map_err(|e| e.to_string())?;
    }
    f.write_all(&list).map_err(|e| format!("Failed to write WAV tags: {}", e))?;
    let new_len = f.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?;
    f.seek(SeekFrom::Start(4)).map_err(|e| e.to_string())?;
    f.write_all(&((new_len - 8) as u32).to_le_bytes())
        .map_err(|e| format!("Failed to update RIFF size: {}", e))?;
    Ok(())
}

/// Export audio as MP3 using FFmpeg (streaming - render and encode simultaneously)
fn export_mp3<P: AsRef<Path>>(
    project: &mut Project,
    pool: &AudioPool,
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

    apply_metadata(&mut output, &settings.metadata);
    output.write_header()
        .map_err(|e| format!("Failed to write header: {}", e))?;

    // Calculate rendering parameters
    let duration = settings.end_time - settings.start_time;
    let total_frames = (duration.seconds_to_f64() * settings.sample_rate as f64).round() as usize;

    let chunk_samples = EXPORT_CHUNK_FRAMES * settings.channels as usize;
    let chunk_duration = EXPORT_CHUNK_FRAMES as f64 / settings.sample_rate as f64;

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
            &mut buffer_pool,
            playhead,
            &settings.tempo_map,
            settings.sample_rate,
            settings.channels,
            false,
        );

        // Calculate how many samples we need from this chunk
        let remaining_time = settings.end_time - playhead;
        let samples_needed = if remaining_time.seconds_to_f64() < chunk_duration {
            ((remaining_time.seconds_to_f64() * settings.sample_rate as f64) as usize * settings.channels as usize)
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

        playhead = playhead + Seconds(chunk_duration);
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

    apply_metadata(&mut output, &settings.metadata);
    output.write_header()
        .map_err(|e| format!("Failed to write header: {}", e))?;

    // Calculate rendering parameters
    let duration = settings.end_time - settings.start_time;
    let total_frames = (duration.seconds_to_f64() * settings.sample_rate as f64).round() as usize;

    let chunk_samples = EXPORT_CHUNK_FRAMES * settings.channels as usize;
    let chunk_duration = EXPORT_CHUNK_FRAMES as f64 / settings.sample_rate as f64;

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
            &mut buffer_pool,
            playhead,
            &settings.tempo_map,
            settings.sample_rate,
            settings.channels,
            false,
        );

        // Calculate how many samples we need from this chunk
        let remaining_time = settings.end_time - playhead;
        let samples_needed = if remaining_time.seconds_to_f64() < chunk_duration {
            ((remaining_time.seconds_to_f64() * settings.sample_rate as f64) as usize * settings.channels as usize)
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

        playhead = playhead + Seconds(chunk_duration);
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

/// Convert a chunk of interleaved f32 samples to planar f32 format.
///
/// Non-finite samples (NaN/±Inf) are replaced with `0.0` and finite samples are
/// clamped to `[-1.0, 1.0]`: the float encoders (e.g. AAC, which takes `fltp`)
/// reject a frame outright on "(near) NaN/+-Inf", failing the whole export, so we
/// sanitize here exactly as the integer paths already clamp.
fn convert_chunk_to_planar_f32(interleaved: &[f32], channels: u32) -> Vec<Vec<f32>> {
    let num_frames = interleaved.len() / channels as usize;
    let mut planar = vec![vec![0.0f32; num_frames]; channels as usize];

    let mut non_finite = 0u64;
    for (i, chunk) in interleaved.chunks(channels as usize).enumerate() {
        for (ch, &sample) in chunk.iter().enumerate() {
            planar[ch][i] = if sample.is_finite() {
                sample.clamp(-1.0, 1.0)
            } else {
                non_finite += 1;
                0.0
            };
        }
    }
    if non_finite > 0 {
        // One-time warning: we sanitized rather than failed, but a non-finite
        // sample reaching here means something upstream (an effect, automation,
        // or a source decode) produced NaN/Inf — worth chasing if audio is wrong.
        use std::sync::atomic::{AtomicBool, Ordering};
        static WARNED: AtomicBool = AtomicBool::new(false);
        if !WARNED.swap(true, Ordering::Relaxed) {
            eprintln!(
                "⚠️ [EXPORT] sanitized {} non-finite (NaN/Inf) audio sample(s) in a chunk — \
                 check effects/automation/source decode",
                non_finite
            );
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
    if num_frames == 0 {
        return Ok(());
    }

    let channels = planar_samples.len();

    // Create audio frame
    let mut frame = ffmpeg_next::frame::Audio::new(
        ffmpeg_next::format::Sample::I16(ffmpeg_next::format::sample::Type::Planar),
        num_frames,
        channel_layout,
    );
    frame.set_rate(sample_rate);
    frame.set_pts(Some(pts));

    // Verify frame was allocated (check linesize[0] via planes())
    if frame.planes() == 0 {
        return Err("FFmpeg failed to allocate audio frame. Try exporting as WAV instead.".to_string());
    }

    // Copy all planar samples to frame
    // Use plane_mut::<i16> instead of data_mut — data_mut(ch) is buggy for planar audio:
    // FFmpeg only sets linesize[0], so data_mut returns 0-length slices for ch > 0.
    // plane_mut uses self.samples() for the length, which is correct for all planes.
    for ch in 0..channels {
        let plane = frame.plane_mut::<i16>(ch);
        plane.copy_from_slice(&planar_samples[ch]);
    }

    encoder.send_frame(&frame)
        .map_err(|e| format!("Failed to send frame: {}", e))?;

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
    if num_frames == 0 {
        return Ok(());
    }

    let channels = planar_samples.len();

    // Create audio frame
    let mut frame = ffmpeg_next::frame::Audio::new(
        ffmpeg_next::format::Sample::F32(ffmpeg_next::format::sample::Type::Planar),
        num_frames,
        channel_layout,
    );
    frame.set_rate(sample_rate);
    frame.set_pts(Some(pts));

    // Verify frame was allocated
    if frame.planes() == 0 {
        return Err("FFmpeg failed to allocate audio frame. Try exporting as WAV instead.".to_string());
    }

    // Copy all planar samples to frame
    // Use plane_mut::<f32> instead of data_mut — data_mut(ch) is buggy for planar audio:
    // FFmpeg only sets linesize[0], so data_mut returns 0-length slices for ch > 0.
    // plane_mut uses self.samples() for the length, which is correct for all planes.
    for ch in 0..channels {
        let plane = frame.plane_mut::<f32>(ch);
        plane.copy_from_slice(&planar_samples[ch]);
    }

    encoder.send_frame(&frame)
        .map_err(|e| format!("Failed to send frame: {}", e))?;

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

    fn tagged_settings(format: ExportFormat) -> ExportSettings {
        ExportSettings {
            format,
            sample_rate: 48000,
            channels: 2,
            bit_depth: 24,
            mp3_bitrate: 192,
            start_time: Seconds::ZERO,
            end_time: Seconds(0.2), // tiny render
            tempo_map: TempoMap::constant(120.0),
            metadata: vec![
                ("title".to_string(), "Test Title".to_string()),
                ("artist".to_string(), "Test Artist".to_string()),
            ],
        }
    }

    /// FLAC export must be a real FLAC container (not WAV bytes) carrying Vorbis-comment tags.
    #[test]
    fn flac_export_is_real_flac_with_tags() {
        let settings = tagged_settings(ExportFormat::Flac);
        let mut project = Project::new(48000);
        let pool = AudioPool::new();
        let path = std::env::temp_dir().join("lb_be_flac_test.flac");

        export_audio(&mut project, &pool, &settings, &path, None).expect("FLAC export failed");
        let bytes = std::fs::read(&path).unwrap();

        assert_eq!(&bytes[0..4], b"fLaC", "not real FLAC (got {:?})", &bytes[0..4]);
        let s = String::from_utf8_lossy(&bytes);
        assert!(s.contains("Test Title"), "title tag missing from FLAC");
        assert!(s.contains("Test Artist"), "artist tag missing from FLAC");
        std::fs::remove_file(&path).ok();
    }

    /// WAV export keeps a valid RIFF container and gains a LIST/INFO tag chunk with a fixed-up size.
    #[test]
    fn wav_export_has_info_chunk() {
        let settings = tagged_settings(ExportFormat::Wav);
        let mut project = Project::new(48000);
        let pool = AudioPool::new();
        let path = std::env::temp_dir().join("lb_be_wav_test.wav");

        export_audio(&mut project, &pool, &settings, &path, None).expect("WAV export failed");
        let bytes = std::fs::read(&path).unwrap();

        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        let riff_size = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
        assert_eq!(riff_size, bytes.len() - 8, "RIFF size not fixed up after tagging");
        let s = String::from_utf8_lossy(&bytes);
        assert!(s.contains("LIST") && s.contains("INFO") && s.contains("INAM"),
            "no RIFF INFO chunk");
        assert!(s.contains("Test Title"), "title not in WAV INFO chunk");
        std::fs::remove_file(&path).ok();
    }
}

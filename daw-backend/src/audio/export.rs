use super::buffer_pool::BufferPool;
use super::midi_pool::MidiClipPool;
use super::pool::AudioPool;
use super::project::Project;
use std::path::Path;

/// Supported export formats
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Wav,
    Flac,
    // TODO: Add MP3 support
}

impl ExportFormat {
    /// Get the file extension for this format
    pub fn extension(&self) -> &'static str {
        match self {
            ExportFormat::Wav => "wav",
            ExportFormat::Flac => "flac",
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
pub fn export_audio<P: AsRef<Path>>(
    project: &mut Project,
    pool: &AudioPool,
    midi_pool: &MidiClipPool,
    settings: &ExportSettings,
    output_path: P,
) -> Result<(), String> {
    // Render the project to memory
    let samples = render_to_memory(project, pool, midi_pool, settings)?;

    // Write to file based on format
    match settings.format {
        ExportFormat::Wav => write_wav(&samples, settings, output_path)?,
        ExportFormat::Flac => write_flac(&samples, settings, output_path)?,
    }

    Ok(())
}

/// Render the project to memory
fn render_to_memory(
    project: &mut Project,
    pool: &AudioPool,
    midi_pool: &MidiClipPool,
    settings: &ExportSettings,
) -> Result<Vec<f32>, String> {
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

// TODO: Add MP3 export support with a better library

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

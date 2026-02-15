use std::path::Path;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WaveformPeak {
    pub min: f32,
    pub max: f32,
}

/// Uniquely identifies a waveform chunk
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WaveformChunkKey {
    pub pool_index: usize,
    pub detail_level: u8,    // 0-4
    pub chunk_index: u32,    // Sequential chunk number
}

/// A chunk of waveform data at a specific detail level
#[derive(Debug, Clone)]
pub struct WaveformChunk {
    pub audio_pool_index: usize,
    pub detail_level: u8,           // 0-4 (overview to max detail)
    pub chunk_index: u32,           // Sequential chunk number
    pub time_range: (f64, f64),     // Start and end time in seconds
    pub peaks: Vec<WaveformPeak>,   // Variable length based on level
}

/// Whether an audio file is uncompressed (WAV/AIFF — can be memory-mapped) or compressed
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    /// Uncompressed PCM (WAV, AIFF) — suitable for memory mapping
    Pcm,
    /// Compressed (MP3, FLAC, OGG, AAC, etc.) — requires decoding
    Compressed,
}

/// Audio file metadata obtained without decoding
#[derive(Debug, Clone)]
pub struct AudioMetadata {
    pub channels: u32,
    pub sample_rate: u32,
    pub duration: f64,
    pub n_frames: Option<u64>,
    pub format: AudioFormat,
}

pub struct AudioFile {
    pub data: Vec<f32>,
    pub channels: u32,
    pub sample_rate: u32,
    pub frames: u64,
}

/// Read only metadata from an audio file without decoding any audio packets.
/// This is fast (sub-millisecond) and suitable for calling on the UI thread.
pub fn read_metadata<P: AsRef<Path>>(path: P) -> Result<AudioMetadata, String> {
    let path = path.as_ref();

    let file = std::fs::File::open(path)
        .map_err(|e| format!("Failed to open file: {}", e))?;

    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    let ext = path.extension().and_then(|e| e.to_str()).map(|s| s.to_lowercase());
    if let Some(ref ext_str) = ext {
        hint.with_extension(ext_str);
    }

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("Failed to probe file: {}", e))?;

    let format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        .ok_or_else(|| "No audio tracks found".to_string())?;

    let codec_params = &track.codec_params;
    let channels = codec_params.channels
        .ok_or_else(|| "Channel count not specified".to_string())?
        .count() as u32;
    let sample_rate = codec_params.sample_rate
        .ok_or_else(|| "Sample rate not specified".to_string())?;
    let n_frames = codec_params.n_frames;

    // Determine duration from frame count or time base
    let duration = if let Some(frames) = n_frames {
        frames as f64 / sample_rate as f64
    } else if let Some(tb) = codec_params.time_base {
        if let Some(dur) = codec_params.n_frames {
            tb.calc_time(dur).seconds as f64 + tb.calc_time(dur).frac
        } else {
            0.0
        }
    } else {
        0.0
    };

    // Determine if this is a PCM format (WAV/AIFF) or compressed
    let audio_format = match ext.as_deref() {
        Some("wav") | Some("wave") | Some("aiff") | Some("aif") => AudioFormat::Pcm,
        _ => AudioFormat::Compressed,
    };

    Ok(AudioMetadata {
        channels,
        sample_rate,
        duration,
        n_frames,
        format: audio_format,
    })
}

/// Parsed WAV header info needed for memory-mapping.
pub struct WavHeaderInfo {
    pub data_offset: usize,
    pub data_size: usize,
    pub sample_format: crate::audio::pool::PcmSampleFormat,
    pub channels: u32,
    pub sample_rate: u32,
    pub total_frames: u64,
}

/// Parse a WAV file header from a byte slice (e.g. from an mmap).
/// Returns the byte offset to PCM data and format details.
pub fn parse_wav_header(data: &[u8]) -> Result<WavHeaderInfo, String> {
    if data.len() < 44 {
        return Err("File too small to be a valid WAV".to_string());
    }

    // RIFF header
    if &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err("Not a valid RIFF/WAVE file".to_string());
    }

    // Walk chunks to find "fmt " and "data"
    let mut pos = 12;
    let mut fmt_found = false;
    let mut channels: u32 = 0;
    let mut sample_rate: u32 = 0;
    let mut bits_per_sample: u16 = 0;
    let mut format_code: u16 = 0;

    let mut data_offset: usize = 0;
    let mut data_size: usize = 0;

    while pos + 8 <= data.len() {
        let chunk_id = &data[pos..pos + 4];
        let chunk_size = u32::from_le_bytes([
            data[pos + 4],
            data[pos + 5],
            data[pos + 6],
            data[pos + 7],
        ]) as usize;

        if chunk_id == b"fmt " {
            if pos + 8 + 16 > data.len() {
                return Err("fmt chunk too small".to_string());
            }
            let base = pos + 8;
            format_code = u16::from_le_bytes([data[base], data[base + 1]]);
            channels = u16::from_le_bytes([data[base + 2], data[base + 3]]) as u32;
            sample_rate = u32::from_le_bytes([
                data[base + 4],
                data[base + 5],
                data[base + 6],
                data[base + 7],
            ]);
            bits_per_sample = u16::from_le_bytes([data[base + 14], data[base + 15]]);
            fmt_found = true;
        } else if chunk_id == b"data" {
            data_offset = pos + 8;
            data_size = chunk_size;
            break;
        }

        // Advance to next chunk (chunks are 2-byte aligned)
        pos += 8 + chunk_size;
        if chunk_size % 2 != 0 {
            pos += 1;
        }
    }

    if !fmt_found {
        return Err("No fmt chunk found".to_string());
    }
    if data_offset == 0 {
        return Err("No data chunk found".to_string());
    }

    // Determine sample format
    let sample_format = match (format_code, bits_per_sample) {
        (1, 16) => crate::audio::pool::PcmSampleFormat::I16,
        (1, 24) => crate::audio::pool::PcmSampleFormat::I24,
        (3, 32) => crate::audio::pool::PcmSampleFormat::F32,
        (1, 32) => crate::audio::pool::PcmSampleFormat::F32, // 32-bit PCM treated as float
        _ => {
            return Err(format!(
                "Unsupported WAV format: code={}, bits={}",
                format_code, bits_per_sample
            ));
        }
    };

    let bytes_per_sample = (bits_per_sample / 8) as usize;
    let bytes_per_frame = bytes_per_sample * channels as usize;
    let total_frames = if bytes_per_frame > 0 {
        (data_size / bytes_per_frame) as u64
    } else {
        0
    };

    Ok(WavHeaderInfo {
        data_offset,
        data_size,
        sample_format,
        channels,
        sample_rate,
        total_frames,
    })
}

impl AudioFile {
    /// Load an audio file from disk and decode it to interleaved f32 samples
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let path = path.as_ref();

        // Open the media source
        let file = std::fs::File::open(path)
            .map_err(|e| format!("Failed to open file: {}", e))?;

        let mss = MediaSourceStream::new(Box::new(file), Default::default());

        // Create a probe hint using the file extension
        let mut hint = Hint::new();
        if let Some(extension) = path.extension() {
            if let Some(ext_str) = extension.to_str() {
                hint.with_extension(ext_str);
            }
        }

        // Probe the media source
        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
            .map_err(|e| format!("Failed to probe file: {}", e))?;

        let mut format = probed.format;

        // Find the default audio track
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
            .ok_or_else(|| "No audio tracks found".to_string())?;

        let track_id = track.id;

        // Get audio parameters
        let codec_params = &track.codec_params;
        let channels = codec_params.channels
            .ok_or_else(|| "Channel count not specified".to_string())?
            .count() as u32;
        let sample_rate = codec_params.sample_rate
            .ok_or_else(|| "Sample rate not specified".to_string())?;

        // Create decoder
        let mut decoder = symphonia::default::get_codecs()
            .make(&codec_params, &DecoderOptions::default())
            .map_err(|e| format!("Failed to create decoder: {}", e))?;

        // Decode all packets
        let mut audio_data = Vec::new();
        let mut sample_buf = None;

        loop {
            let packet = match format.next_packet() {
                Ok(packet) => packet,
                Err(Error::ResetRequired) => {
                    return Err("Decoder reset required (not implemented)".to_string());
                }
                Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    // End of file
                    break;
                }
                Err(e) => {
                    return Err(format!("Failed to read packet: {}", e));
                }
            };

            // Skip packets for other tracks
            if packet.track_id() != track_id {
                continue;
            }

            // Decode the packet
            match decoder.decode(&packet) {
                Ok(decoded) => {
                    // Initialize sample buffer on first packet
                    if sample_buf.is_none() {
                        let spec = *decoded.spec();
                        let duration = decoded.capacity() as u64;
                        sample_buf = Some(SampleBuffer::<f32>::new(duration, spec));
                    }

                    // Copy decoded audio to sample buffer
                    if let Some(ref mut buf) = sample_buf {
                        buf.copy_interleaved_ref(decoded);
                        audio_data.extend_from_slice(buf.samples());
                    }
                }
                Err(Error::DecodeError(e)) => {
                    eprintln!("Decode error: {}", e);
                    continue;
                }
                Err(e) => {
                    return Err(format!("Decode failed: {}", e));
                }
            }
        }

        let frames = (audio_data.len() / channels as usize) as u64;

        Ok(AudioFile {
            data: audio_data,
            channels,
            sample_rate,
            frames,
        })
    }

    /// Decode a compressed audio file progressively, calling `on_progress` with
    /// partial data snapshots so the UI can display waveforms as they decode.
    /// Sends updates roughly every 2 seconds of decoded audio.
    pub fn decode_progressive<P: AsRef<Path>, F>(path: P, total_frames: u64, on_progress: F)
    where
        F: Fn(&[f32], u64, u64),
    {
        let path = path.as_ref();

        let file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("[WAVEFORM DECODE] Failed to open {:?}: {}", path, e);
                return;
            }
        };

        let mss = MediaSourceStream::new(Box::new(file), Default::default());

        let mut hint = Hint::new();
        if let Some(extension) = path.extension() {
            if let Some(ext_str) = extension.to_str() {
                hint.with_extension(ext_str);
            }
        }

        let probed = match symphonia::default::get_probe()
            .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[WAVEFORM DECODE] Failed to probe {:?}: {}", path, e);
                return;
            }
        };

        let mut format = probed.format;

        let track = match format.tracks().iter()
            .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        {
            Some(t) => t,
            None => {
                eprintln!("[WAVEFORM DECODE] No audio tracks in {:?}", path);
                return;
            }
        };

        let track_id = track.id;
        let channels = track.codec_params.channels
            .map(|c| c.count() as u32)
            .unwrap_or(2);
        let sample_rate = track.codec_params.sample_rate.unwrap_or(44100);

        let mut decoder = match symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
        {
            Ok(d) => d,
            Err(e) => {
                eprintln!("[WAVEFORM DECODE] Failed to create decoder for {:?}: {}", path, e);
                return;
            }
        };

        let mut audio_data = Vec::new();
        let mut sample_buf = None;
        // Send a progress update roughly every 2 seconds of audio
        // Send first update quickly (0.25s), then every 2s of audio
        let initial_interval = (sample_rate as usize * channels as usize) / 4;
        let steady_interval = (sample_rate as usize * channels as usize) * 2;
        let mut sent_first = false;
        let mut last_update_len = 0usize;

        loop {
            let packet = match format.next_packet() {
                Ok(packet) => packet,
                Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(Error::ResetRequired) => break,
                Err(_) => break,
            };

            if packet.track_id() != track_id {
                continue;
            }

            match decoder.decode(&packet) {
                Ok(decoded) => {
                    if sample_buf.is_none() {
                        let spec = *decoded.spec();
                        let duration = decoded.capacity() as u64;
                        sample_buf = Some(SampleBuffer::<f32>::new(duration, spec));
                    }
                    if let Some(ref mut buf) = sample_buf {
                        buf.copy_interleaved_ref(decoded);
                        audio_data.extend_from_slice(buf.samples());
                    }

                    // Send progressive update (fast initial, then periodic)
                    // Only send NEW samples since last update (delta) to avoid large copies
                    let interval = if sent_first { steady_interval } else { initial_interval };
                    if audio_data.len() - last_update_len >= interval {
                        let decoded_frames = audio_data.len() as u64 / channels as u64;
                        on_progress(&audio_data[last_update_len..], decoded_frames, total_frames);
                        last_update_len = audio_data.len();
                        sent_first = true;
                    }
                }
                Err(Error::DecodeError(_)) => continue,
                Err(_) => break,
            }
        }

        // Final update with remaining data (delta since last update)
        let decoded_frames = audio_data.len() as u64 / channels as u64;
        on_progress(&audio_data[last_update_len..], decoded_frames, decoded_frames.max(total_frames));
    }

    /// Calculate the duration of the audio file in seconds
    pub fn duration(&self) -> f64 {
        self.frames as f64 / self.sample_rate as f64
    }

    /// Generate a waveform overview with the specified number of peaks
    /// This creates a downsampled representation suitable for timeline visualization
    pub fn generate_waveform_overview(&self, target_peaks: usize) -> Vec<WaveformPeak> {
        self.generate_waveform_overview_range(0, self.frames as usize, target_peaks)
    }

    /// Generate a waveform overview for a specific range of frames
    ///
    /// # Arguments
    /// * `start_frame` - Starting frame index (0-based)
    /// * `end_frame` - Ending frame index (exclusive)
    /// * `target_peaks` - Desired number of peaks to generate
    pub fn generate_waveform_overview_range(
        &self,
        start_frame: usize,
        end_frame: usize,
        target_peaks: usize,
    ) -> Vec<WaveformPeak> {
        if self.frames == 0 || target_peaks == 0 {
            return Vec::new();
        }

        let total_frames = self.frames as usize;
        let start_frame = start_frame.min(total_frames);
        let end_frame = end_frame.min(total_frames);

        if start_frame >= end_frame {
            return Vec::new();
        }

        let range_frames = end_frame - start_frame;
        let frames_per_peak = (range_frames / target_peaks).max(1);
        let actual_peaks = (range_frames + frames_per_peak - 1) / frames_per_peak;

        let mut peaks = Vec::with_capacity(actual_peaks);

        for peak_idx in 0..actual_peaks {
            let peak_start = start_frame + peak_idx * frames_per_peak;
            let peak_end = (start_frame + (peak_idx + 1) * frames_per_peak).min(end_frame);

            let mut min = 0.0f32;
            let mut max = 0.0f32;

            // Scan all samples in this window
            for frame_idx in peak_start..peak_end {
                // For multi-channel audio, combine all channels
                for ch in 0..self.channels as usize {
                    let sample_idx = frame_idx * self.channels as usize + ch;
                    if sample_idx < self.data.len() {
                        let sample = self.data[sample_idx];
                        min = min.min(sample);
                        max = max.max(sample);
                    }
                }
            }

            peaks.push(WaveformPeak { min, max });
        }

        peaks
    }
}

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

pub struct AudioFile {
    pub data: Vec<f32>,
    pub channels: u32,
    pub sample_rate: u32,
    pub frames: u64,
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

    /// Calculate the duration of the audio file in seconds
    pub fn duration(&self) -> f64 {
        self.frames as f64 / self.sample_rate as f64
    }

    /// Generate a waveform overview with the specified number of peaks
    /// This creates a downsampled representation suitable for timeline visualization
    pub fn generate_waveform_overview(&self, target_peaks: usize) -> Vec<WaveformPeak> {
        if self.frames == 0 || target_peaks == 0 {
            return Vec::new();
        }

        let total_frames = self.frames as usize;
        let frames_per_peak = (total_frames / target_peaks).max(1);
        let actual_peaks = (total_frames + frames_per_peak - 1) / frames_per_peak;

        let mut peaks = Vec::with_capacity(actual_peaks);

        for peak_idx in 0..actual_peaks {
            let start_frame = peak_idx * frames_per_peak;
            let end_frame = ((peak_idx + 1) * frames_per_peak).min(total_frames);

            let mut min = 0.0f32;
            let mut max = 0.0f32;

            // Scan all samples in this window
            for frame_idx in start_frame..end_frame {
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

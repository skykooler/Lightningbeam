use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use std::fs::File;
use std::path::Path;

/// Loaded audio sample data
#[derive(Debug, Clone)]
pub struct SampleData {
    /// Audio samples (mono, f32 format)
    pub samples: Vec<f32>,
    /// Original sample rate
    pub sample_rate: u32,
}

/// Load an audio file and decode it to mono f32 samples
pub fn load_audio_file(path: impl AsRef<Path>) -> Result<SampleData, String> {
    let path = path.as_ref();

    // Open the file
    let file = File::open(path)
        .map_err(|e| format!("Failed to open file: {}", e))?;

    // Create a media source stream
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    // Create a hint to help the format registry guess the format
    let mut hint = Hint::new();
    if let Some(extension) = path.extension() {
        if let Some(ext_str) = extension.to_str() {
            hint.with_extension(ext_str);
        }
    }

    // Probe the media source for a format
    let format_opts = FormatOptions::default();
    let metadata_opts = MetadataOptions::default();

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &format_opts, &metadata_opts)
        .map_err(|e| format!("Failed to probe format: {}", e))?;

    let mut format = probed.format;

    // Find the first audio track
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| "No audio tracks found".to_string())?;

    let track_id = track.id;
    let sample_rate = track.codec_params.sample_rate.unwrap_or(48000);

    // Create a decoder for the track
    let dec_opts = DecoderOptions::default();
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &dec_opts)
        .map_err(|e| format!("Failed to create decoder: {}", e))?;

    // Decode all packets
    let mut all_samples = Vec::new();

    loop {
        // Get the next packet
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                // End of stream
                break;
            }
            Err(e) => {
                return Err(format!("Error reading packet: {}", e));
            }
        };

        // Skip packets that don't belong to the selected track
        if packet.track_id() != track_id {
            continue;
        }

        // Decode the packet
        let decoded = decoder
            .decode(&packet)
            .map_err(|e| format!("Failed to decode packet: {}", e))?;

        // Convert to f32 samples and mix to mono
        let samples = convert_to_mono_f32(&decoded);
        all_samples.extend_from_slice(&samples);
    }

    Ok(SampleData {
        samples: all_samples,
        sample_rate,
    })
}

/// Convert an audio buffer to mono f32 samples
fn convert_to_mono_f32(buf: &AudioBufferRef) -> Vec<f32> {
    match buf {
        AudioBufferRef::F32(buf) => {
            let channels = buf.spec().channels.count();
            let frames = buf.frames();
            let mut mono = Vec::with_capacity(frames);

            if channels == 1 {
                // Already mono
                mono.extend_from_slice(buf.chan(0));
            } else {
                // Mix down to mono by averaging all channels
                for frame in 0..frames {
                    let mut sum = 0.0;
                    for ch in 0..channels {
                        sum += buf.chan(ch)[frame];
                    }
                    mono.push(sum / channels as f32);
                }
            }

            mono
        }
        AudioBufferRef::U8(buf) => {
            let channels = buf.spec().channels.count();
            let frames = buf.frames();
            let mut mono = Vec::with_capacity(frames);

            if channels == 1 {
                for &sample in buf.chan(0) {
                    mono.push((sample as f32 - 128.0) / 128.0);
                }
            } else {
                for frame in 0..frames {
                    let mut sum = 0.0;
                    for ch in 0..channels {
                        sum += (buf.chan(ch)[frame] as f32 - 128.0) / 128.0;
                    }
                    mono.push(sum / channels as f32);
                }
            }

            mono
        }
        AudioBufferRef::U16(buf) => {
            let channels = buf.spec().channels.count();
            let frames = buf.frames();
            let mut mono = Vec::with_capacity(frames);

            if channels == 1 {
                for &sample in buf.chan(0) {
                    mono.push((sample as f32 - 32768.0) / 32768.0);
                }
            } else {
                for frame in 0..frames {
                    let mut sum = 0.0;
                    for ch in 0..channels {
                        sum += (buf.chan(ch)[frame] as f32 - 32768.0) / 32768.0;
                    }
                    mono.push(sum / channels as f32);
                }
            }

            mono
        }
        AudioBufferRef::U24(buf) => {
            let channels = buf.spec().channels.count();
            let frames = buf.frames();
            let mut mono = Vec::with_capacity(frames);

            if channels == 1 {
                for &sample in buf.chan(0) {
                    mono.push((sample.inner() as f32 - 8388608.0) / 8388608.0);
                }
            } else {
                for frame in 0..frames {
                    let mut sum = 0.0;
                    for ch in 0..channels {
                        sum += (buf.chan(ch)[frame].inner() as f32 - 8388608.0) / 8388608.0;
                    }
                    mono.push(sum / channels as f32);
                }
            }

            mono
        }
        AudioBufferRef::U32(buf) => {
            let channels = buf.spec().channels.count();
            let frames = buf.frames();
            let mut mono = Vec::with_capacity(frames);

            if channels == 1 {
                for &sample in buf.chan(0) {
                    mono.push((sample as f32 - 2147483648.0) / 2147483648.0);
                }
            } else {
                for frame in 0..frames {
                    let mut sum = 0.0;
                    for ch in 0..channels {
                        sum += (buf.chan(ch)[frame] as f32 - 2147483648.0) / 2147483648.0;
                    }
                    mono.push(sum / channels as f32);
                }
            }

            mono
        }
        AudioBufferRef::S8(buf) => {
            let channels = buf.spec().channels.count();
            let frames = buf.frames();
            let mut mono = Vec::with_capacity(frames);

            if channels == 1 {
                for &sample in buf.chan(0) {
                    mono.push(sample as f32 / 128.0);
                }
            } else {
                for frame in 0..frames {
                    let mut sum = 0.0;
                    for ch in 0..channels {
                        sum += buf.chan(ch)[frame] as f32 / 128.0;
                    }
                    mono.push(sum / channels as f32);
                }
            }

            mono
        }
        AudioBufferRef::S16(buf) => {
            let channels = buf.spec().channels.count();
            let frames = buf.frames();
            let mut mono = Vec::with_capacity(frames);

            if channels == 1 {
                for &sample in buf.chan(0) {
                    mono.push(sample as f32 / 32768.0);
                }
            } else {
                for frame in 0..frames {
                    let mut sum = 0.0;
                    for ch in 0..channels {
                        sum += buf.chan(ch)[frame] as f32 / 32768.0;
                    }
                    mono.push(sum / channels as f32);
                }
            }

            mono
        }
        AudioBufferRef::S24(buf) => {
            let channels = buf.spec().channels.count();
            let frames = buf.frames();
            let mut mono = Vec::with_capacity(frames);

            if channels == 1 {
                for &sample in buf.chan(0) {
                    mono.push(sample.inner() as f32 / 8388608.0);
                }
            } else {
                for frame in 0..frames {
                    let mut sum = 0.0;
                    for ch in 0..channels {
                        sum += buf.chan(ch)[frame].inner() as f32 / 8388608.0;
                    }
                    mono.push(sum / channels as f32);
                }
            }

            mono
        }
        AudioBufferRef::S32(buf) => {
            let channels = buf.spec().channels.count();
            let frames = buf.frames();
            let mut mono = Vec::with_capacity(frames);

            if channels == 1 {
                for &sample in buf.chan(0) {
                    mono.push(sample as f32 / 2147483648.0);
                }
            } else {
                for frame in 0..frames {
                    let mut sum = 0.0;
                    for ch in 0..channels {
                        sum += buf.chan(ch)[frame] as f32 / 2147483648.0;
                    }
                    mono.push(sum / channels as f32);
                }
            }

            mono
        }
        AudioBufferRef::F64(buf) => {
            let channels = buf.spec().channels.count();
            let frames = buf.frames();
            let mut mono = Vec::with_capacity(frames);

            if channels == 1 {
                for &sample in buf.chan(0) {
                    mono.push(sample as f32);
                }
            } else {
                for frame in 0..frames {
                    let mut sum = 0.0;
                    for ch in 0..channels {
                        sum += buf.chan(ch)[frame] as f32;
                    }
                    mono.push(sum / channels as f32);
                }
            }

            mono
        }
    }
}

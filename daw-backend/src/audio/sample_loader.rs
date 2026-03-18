use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use std::fs::File;
use std::io::Cursor;
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
    let file = File::open(path).map_err(|e| format!("Failed to open file: {}", e))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    decode_mss(mss, hint)
}

/// Load audio from an in-memory byte slice and decode it to mono f32 samples.
/// Supports WAV, FLAC, MP3, AAC, and any other format Symphonia recognises.
/// `filename_hint` is used to help Symphonia detect the format (e.g. "kick.wav").
pub fn load_audio_from_bytes(bytes: &[u8], filename_hint: &str) -> Result<SampleData, String> {
    let cursor = Cursor::new(bytes.to_vec());
    let mss = MediaSourceStream::new(Box::new(cursor), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = std::path::Path::new(filename_hint).extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    decode_mss(mss, hint)
}

/// Shared decode logic: probe `mss`, find the first audio track, decode to mono f32.
fn decode_mss(mss: MediaSourceStream, hint: Hint) -> Result<SampleData, String> {
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("Failed to probe format: {}", e))?;

    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| "No audio tracks found".to_string())?;

    let track_id = track.id;
    let sample_rate = track.codec_params.sample_rate.unwrap_or(48000);

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("Failed to create decoder: {}", e))?;

    let mut all_samples = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break;
            }
            Err(e) => return Err(format!("Error reading packet: {}", e)),
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = decoder
            .decode(&packet)
            .map_err(|e| format!("Failed to decode packet: {}", e))?;

        all_samples.extend_from_slice(&convert_to_mono_f32(&decoded));
    }

    Ok(SampleData { samples: all_samples, sample_rate })
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

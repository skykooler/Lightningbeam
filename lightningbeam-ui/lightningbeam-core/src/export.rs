//! Export settings and types for audio and video export
//!
//! This module contains platform-agnostic export settings that can be used
//! across different frontends (native, web, etc.). The actual export implementation
//! is in the platform-specific code (e.g., lightningbeam-editor).

use serde::{Deserialize, Serialize};

/// Audio export formats
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioFormat {
    /// WAV - Uncompressed audio (large files, best quality)
    Wav,
    /// FLAC - Lossless compressed audio (smaller than WAV, same quality)
    Flac,
    /// MP3 - Lossy compressed audio (widely compatible)
    Mp3,
    /// AAC - Lossy compressed audio (better quality than MP3 at same bitrate)
    Aac,
}

impl AudioFormat {
    /// Get the file extension for this format
    pub fn extension(&self) -> &'static str {
        match self {
            AudioFormat::Wav => "wav",
            AudioFormat::Flac => "flac",
            AudioFormat::Mp3 => "mp3",
            AudioFormat::Aac => "m4a",
        }
    }

    /// Get a human-readable name for this format
    pub fn name(&self) -> &'static str {
        match self {
            AudioFormat::Wav => "WAV (Uncompressed)",
            AudioFormat::Flac => "FLAC (Lossless)",
            AudioFormat::Mp3 => "MP3",
            AudioFormat::Aac => "AAC",
        }
    }

    /// Check if this format supports bit depth settings
    pub fn supports_bit_depth(&self) -> bool {
        matches!(self, AudioFormat::Wav | AudioFormat::Flac)
    }

    /// Check if this format uses bitrate settings (lossy formats)
    pub fn uses_bitrate(&self) -> bool {
        matches!(self, AudioFormat::Mp3 | AudioFormat::Aac)
    }
}

/// Optional tag metadata written into the exported audio file. Empty fields are omitted. FFmpeg
/// maps these standard keys to each container's native tags: ID3v2 (MP3), iTunes/MP4 atoms (M4A),
/// Vorbis comments (FLAC), and RIFF INFO (WAV).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AudioMetadata {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub genre: String,
    pub comment: String,
    /// Year or full date (written to the `date` tag).
    pub year: String,
    /// Track number (written to the `track` tag).
    pub track: String,
}

impl AudioMetadata {
    /// True when no field is set (so no metadata need be written).
    pub fn is_empty(&self) -> bool {
        self.title.is_empty()
            && self.artist.is_empty()
            && self.album.is_empty()
            && self.genre.is_empty()
            && self.comment.is_empty()
            && self.year.is_empty()
            && self.track.is_empty()
    }

    /// The set (ffmpeg-key, value) pairs for non-empty fields, in a stable order.
    pub fn pairs(&self) -> Vec<(&'static str, &str)> {
        let mut v = Vec::new();
        for (key, val) in [
            ("title", &self.title),
            ("artist", &self.artist),
            ("album", &self.album),
            ("genre", &self.genre),
            ("comment", &self.comment),
            ("date", &self.year),
            ("track", &self.track),
        ] {
            if !val.is_empty() {
                v.push((key, val.as_str()));
            }
        }
        v
    }
}

/// Audio export settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioExportSettings {
    /// Output format
    pub format: AudioFormat,

    /// Sample rate in Hz (e.g., 44100, 48000)
    pub sample_rate: u32,

    /// Number of channels (1 = mono, 2 = stereo)
    pub channels: u32,

    /// Bit depth for lossless formats (16 or 24)
    /// Only used for WAV and FLAC
    pub bit_depth: u16,

    /// Bitrate in kbps for lossy formats (e.g., 128, 192, 256, 320)
    /// Only used for MP3 and AAC
    pub bitrate_kbps: u32,

    /// Start time in seconds
    pub start_time: f64,

    /// End time in seconds
    pub end_time: f64,

    /// Project BPM (for beat-position scheduling during export)
    pub bpm: f64,

    /// Tag metadata (title/artist/…) written into the file. Empty = none.
    #[serde(default)]
    pub metadata: AudioMetadata,
}

impl Default for AudioExportSettings {
    fn default() -> Self {
        Self {
            format: AudioFormat::Wav,
            sample_rate: 48000,
            channels: 2,
            bit_depth: 24,
            bitrate_kbps: 320,
            start_time: 0.0,
            end_time: 60.0,
            bpm: 120.0,
            metadata: AudioMetadata::default(),
        }
    }
}

impl AudioExportSettings {
    /// Create high quality WAV export settings
    pub fn high_quality_wav() -> Self {
        Self {
            format: AudioFormat::Wav,
            sample_rate: 48000,
            channels: 2,
            bit_depth: 24,
            ..Default::default()
        }
    }

    /// Create high quality FLAC export settings
    pub fn high_quality_flac() -> Self {
        Self {
            format: AudioFormat::Flac,
            sample_rate: 48000,
            channels: 2,
            bit_depth: 24,
            ..Default::default()
        }
    }

    /// Create high quality AAC export settings
    pub fn high_quality_aac() -> Self {
        Self {
            format: AudioFormat::Aac,
            sample_rate: 48000,
            channels: 2,
            bitrate_kbps: 320,
            ..Default::default()
        }
    }

    /// Create high quality MP3 export settings
    pub fn high_quality_mp3() -> Self {
        Self {
            format: AudioFormat::Mp3,
            sample_rate: 44100,
            channels: 2,
            bitrate_kbps: 320,
            ..Default::default()
        }
    }

    /// Create standard quality AAC export settings
    pub fn standard_aac() -> Self {
        Self {
            format: AudioFormat::Aac,
            sample_rate: 44100,
            channels: 2,
            bitrate_kbps: 256,
            ..Default::default()
        }
    }

    /// Create standard quality MP3 export settings
    pub fn standard_mp3() -> Self {
        Self {
            format: AudioFormat::Mp3,
            sample_rate: 44100,
            channels: 2,
            bitrate_kbps: 192,
            ..Default::default()
        }
    }

    /// Create podcast-optimized AAC settings (mono, lower bitrate)
    pub fn podcast_aac() -> Self {
        Self {
            format: AudioFormat::Aac,
            sample_rate: 44100,
            channels: 1,
            bitrate_kbps: 128,
            ..Default::default()
        }
    }

    /// Create podcast-optimized MP3 settings (mono, lower bitrate)
    pub fn podcast_mp3() -> Self {
        Self {
            format: AudioFormat::Mp3,
            sample_rate: 44100,
            channels: 1,
            bitrate_kbps: 128,
            ..Default::default()
        }
    }

    /// Validate the settings
    pub fn validate(&self) -> Result<(), String> {
        // Validate sample rate
        if self.sample_rate == 0 {
            return Err("Sample rate must be greater than 0".to_string());
        }

        // Validate channels
        if self.channels == 0 || self.channels > 2 {
            return Err("Channels must be 1 (mono) or 2 (stereo)".to_string());
        }

        // Validate bit depth for lossless formats
        if self.format.supports_bit_depth() {
            if self.bit_depth != 16 && self.bit_depth != 24 {
                return Err("Bit depth must be 16 or 24".to_string());
            }
        }

        // Validate bitrate for lossy formats
        if self.format.uses_bitrate() {
            if self.bitrate_kbps == 0 {
                return Err("Bitrate must be greater than 0".to_string());
            }
        }

        // Validate time range
        if self.start_time < 0.0 {
            return Err("Start time cannot be negative".to_string());
        }
        if self.end_time <= self.start_time {
            return Err("End time must be greater than start time".to_string());
        }

        Ok(())
    }

    /// Get the duration in seconds
    pub fn duration(&self) -> f64 {
        self.end_time - self.start_time
    }
}

/// Video codec types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoCodec {
    /// H.264 (AVC) - Most widely compatible
    H264,
    /// H.265 (HEVC) - Better compression than H.264
    H265,
    /// VP8 - WebM codec
    VP8,
    /// VP9 - Improved WebM codec
    VP9,
    /// ProRes 422 - Professional editing codec
    ProRes422,
}

impl VideoCodec {
    /// Get the typical container format for this codec
    pub fn container_format(&self) -> &'static str {
        match self {
            VideoCodec::H264 | VideoCodec::H265 => "mp4",
            VideoCodec::VP8 | VideoCodec::VP9 => "webm",
            VideoCodec::ProRes422 => "mov",
        }
    }

    /// Get a human-readable name for this codec
    pub fn name(&self) -> &'static str {
        match self {
            VideoCodec::H264 => "H.264 (MP4)",
            VideoCodec::H265 => "H.265 (MP4)",
            VideoCodec::VP8 => "VP8 (WebM)",
            VideoCodec::VP9 => "VP9 (WebM)",
            VideoCodec::ProRes422 => "ProRes 422 (MOV)",
        }
    }
}

/// YUV color range for the encoded video (currently H.264). Limited/TV (16–235) is what nearly
/// every player assumes; Full/PC (0–255) keeps a bit more precision but only looks right in players
/// that honor the full-range tag — so Limited is the safe default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorRange {
    Limited,
    Full,
}

impl Default for ColorRange {
    fn default() -> Self { ColorRange::Limited }
}

impl ColorRange {
    pub fn is_full(&self) -> bool { matches!(self, ColorRange::Full) }
    pub fn name(&self) -> &'static str {
        match self {
            ColorRange::Limited => "Limited (TV, 16–235)",
            ColorRange::Full => "Full (PC, 0–255)",
        }
    }
}

/// HDR output mode for video export. SDR encodes BT.709 8-bit as before; the HDR modes encode
/// 10-bit BT.2020 with the PQ (HDR10) or HLG transfer, preserving super-white highlights from the
/// linear compositor. HDR requires a 10-bit codec (HEVC Main10) — the exporter forces H.265.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum HdrExportMode {
    #[default]
    Sdr,
    /// PQ (SMPTE ST 2084) — HDR10. Graphics white at 203 nits (matches the compositor convention).
    Pq,
    /// HLG (ARIB STD-B67) — broadcast HDR, also displayable as SDR.
    Hlg,
}

impl HdrExportMode {
    pub fn is_hdr(&self) -> bool { !matches!(self, HdrExportMode::Sdr) }
    pub fn name(&self) -> &'static str {
        match self {
            HdrExportMode::Sdr => "SDR (BT.709, 8-bit)",
            HdrExportMode::Pq => "HDR10 / PQ (BT.2020, 10-bit)",
            HdrExportMode::Hlg => "HLG (BT.2020, 10-bit)",
        }
    }
    /// FFmpeg transfer-characteristic name for the color tags.
    pub fn transfer_name(&self) -> &'static str {
        match self {
            HdrExportMode::Sdr => "bt709",
            HdrExportMode::Pq => "smpte2084",
            HdrExportMode::Hlg => "arib-std-b67",
        }
    }
}

/// How the document is fit into the export frame when the export resolution's aspect ratio differs
/// from the document's. Applied as the export `base_transform` (document space → export pixels).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ExportFitMode {
    /// Scale each axis independently to fill the frame — distorts when aspects differ.
    Stretch,
    /// Scale uniformly to fit, centered, with black bars (letterbox/pillarbox). Preserves aspect.
    #[default]
    Letterbox,
    /// Scale uniformly to fill, centered, cropping the overflow. Preserves aspect, no bars.
    Crop,
}

impl ExportFitMode {
    pub fn name(&self) -> &'static str {
        match self {
            ExportFitMode::Stretch => "Stretch (distort to fill)",
            ExportFitMode::Letterbox => "Letterbox (fit, black bars)",
            ExportFitMode::Crop => "Crop (fill, trim edges)",
        }
    }
}

/// Video quality presets
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoQuality {
    /// Low quality - ~2 Mbps
    Low,
    /// Medium quality - ~5 Mbps
    Medium,
    /// High quality - ~10 Mbps
    High,
    /// Very high quality - ~20 Mbps
    VeryHigh,
    /// Custom bitrate in kbps
    Custom(u32),
}

impl VideoQuality {
    /// Get the bitrate in kbps for this quality preset
    pub fn bitrate_kbps(&self) -> u32 {
        match self {
            VideoQuality::Low => 2000,
            VideoQuality::Medium => 5000,
            VideoQuality::High => 10000,
            VideoQuality::VeryHigh => 20000,
            VideoQuality::Custom(bitrate) => *bitrate,
        }
    }

    /// Get a human-readable name
    pub fn name(&self) -> String {
        match self {
            VideoQuality::Low => "Low (2 Mbps)".to_string(),
            VideoQuality::Medium => "Medium (5 Mbps)".to_string(),
            VideoQuality::High => "High (10 Mbps)".to_string(),
            VideoQuality::VeryHigh => "Very High (20 Mbps)".to_string(),
            VideoQuality::Custom(bitrate) => format!("Custom ({} kbps)", bitrate),
        }
    }
}

/// Video export settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoExportSettings {
    /// Video codec
    pub codec: VideoCodec,

    /// Output width in pixels (None = use document width)
    pub width: Option<u32>,

    /// Output height in pixels (None = use document height)
    pub height: Option<u32>,

    /// Frame rate (fps)
    pub framerate: f64,

    /// Video quality
    pub quality: VideoQuality,

    /// YUV color range (H.264 only; ignored by other codecs).
    #[serde(default)]
    pub color_range: ColorRange,

    /// HDR output mode. HDR forces 10-bit HEVC (BT.2020 + PQ/HLG); SDR is the default.
    #[serde(default)]
    pub hdr: HdrExportMode,

    /// How the document is fit into the export frame when aspect ratios differ (default Letterbox).
    #[serde(default)]
    pub fit: ExportFitMode,

    /// Audio settings (None = no audio)
    pub audio: Option<AudioExportSettings>,

    /// Start time in seconds
    pub start_time: f64,

    /// End time in seconds
    pub end_time: f64,
}

impl Default for VideoExportSettings {
    fn default() -> Self {
        Self {
            codec: VideoCodec::H264,
            width: None,
            height: None,
            framerate: 60.0,
            quality: VideoQuality::High,
            color_range: ColorRange::Limited,
            hdr: HdrExportMode::Sdr,
            fit: ExportFitMode::Letterbox,
            audio: Some(AudioExportSettings::high_quality_aac()),
            start_time: 0.0,
            end_time: 60.0,
        }
    }
}

impl VideoExportSettings {
    /// Validate the settings
    pub fn validate(&self) -> Result<(), String> {
        // Validate dimensions if provided
        if let Some(width) = self.width {
            if width == 0 {
                return Err("Width must be greater than 0".to_string());
            }
        }
        if let Some(height) = self.height {
            if height == 0 {
                return Err("Height must be greater than 0".to_string());
            }
        }

        // Validate framerate
        if self.framerate <= 0.0 {
            return Err("Framerate must be greater than 0".to_string());
        }

        // Validate time range
        if self.start_time < 0.0 {
            return Err("Start time cannot be negative".to_string());
        }
        if self.end_time <= self.start_time {
            return Err("End time must be greater than start time".to_string());
        }

        // Validate audio settings if present
        if let Some(audio) = &self.audio {
            audio.validate()?;
        }

        Ok(())
    }

    /// Get the duration in seconds
    pub fn duration(&self) -> f64 {
        self.end_time - self.start_time
    }

    /// Calculate the total number of frames
    pub fn total_frames(&self) -> usize {
        (self.duration() * self.framerate).ceil() as usize
    }
}

// ── Image export ─────────────────────────────────────────────────────────────

/// Image export formats (single-frame still image)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageFormat {
    Png,
    Jpeg,
    WebP,
}

impl ImageFormat {
    pub fn name(self) -> &'static str {
        match self { Self::Png => "PNG", Self::Jpeg => "JPEG", Self::WebP => "WebP" }
    }
    pub fn extension(self) -> &'static str {
        match self { Self::Png => "png", Self::Jpeg => "jpg", Self::WebP => "webp" }
    }
    /// Whether quality (1–100) applies to this format.
    pub fn has_quality(self) -> bool { matches!(self, Self::Jpeg | Self::WebP) }
}

/// Settings for exporting a single frame as a still image.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageExportSettings {
    pub format: ImageFormat,
    /// Document time (seconds) of the frame to render.
    pub time: f64,
    /// Override width; None = use document canvas width.
    pub width: Option<u32>,
    /// Override height; None = use document canvas height.
    pub height: Option<u32>,
    /// Encode quality 1–100 (JPEG / WebP only).
    pub quality: u8,
    /// Preserve the alpha channel in the output (respect document background alpha).
    /// When false, the image is composited onto an opaque background before encoding.
    /// Only meaningful for formats that support alpha (PNG, WebP).
    pub allow_transparency: bool,
    /// How the document is fit into the output frame when aspect ratios differ (default Letterbox).
    #[serde(default)]
    pub fit: ExportFitMode,
}

impl Default for ImageExportSettings {
    fn default() -> Self {
        Self { format: ImageFormat::Png, time: 0.0, width: None, height: None, quality: 90, allow_transparency: false, fit: ExportFitMode::Letterbox }
    }
}

impl ImageExportSettings {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(w) = self.width  { if w == 0 { return Err("Width must be > 0".into());  } }
        if let Some(h) = self.height { if h == 0 { return Err("Height must be > 0".into()); } }
        Ok(())
    }
}

// ── Animated GIF export ──────────────────────────────────────────────────────

/// Settings for exporting an animated GIF (multi-frame, palette-quantized, no audio).
///
/// GIF stores a per-frame delay in centiseconds (1/100 s), so effective frame rate is quantized to
/// whole centiseconds — [`Self::frame_delay_ms`] rounds accordingly and the dialog offers sensible
/// GIF rates. Each frame is quantized to a 256-color palette by the encoder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GifExportSettings {
    /// Output width in pixels (None = use document width).
    pub width: Option<u32>,
    /// Output height in pixels (None = use document height).
    pub height: Option<u32>,
    /// Frame rate (fps). Snapped to whole-centisecond delays at encode time.
    pub framerate: f64,
    /// Loop the animation forever (GIF `NETSCAPE2.0` infinite loop). False = play once.
    pub loop_forever: bool,
    /// Preserve full alpha as GIF 1-bit transparency (pixels below the alpha threshold become the
    /// transparent index). When false, frames are composited onto an opaque background first.
    pub transparency: bool,
    /// How the document is fit into the output frame when aspect ratios differ (default Letterbox).
    #[serde(default)]
    pub fit: ExportFitMode,
    /// Start time in seconds.
    pub start_time: f64,
    /// End time in seconds.
    pub end_time: f64,
}

impl Default for GifExportSettings {
    fn default() -> Self {
        Self {
            width: None,
            height: None,
            framerate: 15.0,
            loop_forever: true,
            transparency: false,
            fit: ExportFitMode::Letterbox,
            start_time: 0.0,
            end_time: 5.0,
        }
    }
}

impl GifExportSettings {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(w) = self.width  { if w == 0 { return Err("Width must be > 0".into());  } }
        if let Some(h) = self.height { if h == 0 { return Err("Height must be > 0".into()); } }
        if self.framerate <= 0.0 {
            return Err("Framerate must be greater than 0".into());
        }
        if self.start_time < 0.0 {
            return Err("Start time cannot be negative".into());
        }
        if self.end_time <= self.start_time {
            return Err("End time must be greater than start time".into());
        }
        Ok(())
    }

    /// Duration in seconds.
    pub fn duration(&self) -> f64 { self.end_time - self.start_time }

    /// Total number of frames to render.
    pub fn total_frames(&self) -> usize {
        (self.duration() * self.framerate).ceil().max(1.0) as usize
    }

    /// Per-frame delay in milliseconds, from the framerate (GIF stores this at centisecond
    /// resolution, so the effective rate is snapped to the nearest 10 ms, min 10 ms).
    pub fn frame_delay_ms(&self) -> u32 {
        let ms = 1000.0 / self.framerate;
        ((ms / 10.0).round().max(1.0) * 10.0) as u32
    }
}

/// Progress updates during export
#[derive(Debug, Clone)]
pub enum ExportProgress {
    /// Export started
    Started {
        /// Total number of frames (0 for audio-only)
        total_frames: usize,
    },

    /// A frame was rendered (video only)
    FrameRendered {
        /// Current frame number
        frame: usize,
        /// Total frames
        total: usize,
    },

    /// Audio rendering completed
    AudioRendered,

    /// Finalizing the export (writing file, cleanup)
    Finalizing,

    /// Export completed successfully
    Complete {
        /// Path to the exported file
        output_path: std::path::PathBuf,
    },

    /// Export failed
    Error {
        /// Error message
        message: String,
    },
}

impl ExportProgress {
    /// Get a human-readable status message
    pub fn status_message(&self) -> String {
        match self {
            ExportProgress::Started { total_frames } => {
                if *total_frames > 0 {
                    format!("Starting export ({} frames)...", total_frames)
                } else {
                    "Starting audio export...".to_string()
                }
            }
            ExportProgress::FrameRendered { frame, total } => {
                format!("Rendering frame {} of {}...", frame, total)
            }
            ExportProgress::AudioRendered => "Audio rendered successfully".to_string(),
            ExportProgress::Finalizing => "Finalizing export...".to_string(),
            ExportProgress::Complete { output_path } => {
                format!("Export complete: {}", output_path.display())
            }
            ExportProgress::Error { message } => {
                format!("Export failed: {}", message)
            }
        }
    }

    /// Get progress as a percentage (0.0 to 1.0)
    pub fn progress_percentage(&self) -> Option<f32> {
        match self {
            ExportProgress::Started { .. } => Some(0.0),
            ExportProgress::FrameRendered { frame, total } => {
                Some(*frame as f32 / *total as f32)
            }
            ExportProgress::AudioRendered => Some(0.9),
            ExportProgress::Finalizing => Some(0.95),
            ExportProgress::Complete { .. } => Some(1.0),
            ExportProgress::Error { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_format_extension() {
        assert_eq!(AudioFormat::Wav.extension(), "wav");
        assert_eq!(AudioFormat::Flac.extension(), "flac");
        assert_eq!(AudioFormat::Mp3.extension(), "mp3");
        assert_eq!(AudioFormat::Aac.extension(), "m4a");
    }

    #[test]
    fn test_audio_format_capabilities() {
        assert!(AudioFormat::Wav.supports_bit_depth());
        assert!(AudioFormat::Flac.supports_bit_depth());
        assert!(!AudioFormat::Mp3.supports_bit_depth());
        assert!(!AudioFormat::Aac.supports_bit_depth());

        assert!(AudioFormat::Mp3.uses_bitrate());
        assert!(AudioFormat::Aac.uses_bitrate());
        assert!(!AudioFormat::Wav.uses_bitrate());
        assert!(!AudioFormat::Flac.uses_bitrate());
    }

    #[test]
    fn test_audio_export_settings_validation() {
        let mut settings = AudioExportSettings::default();
        assert!(settings.validate().is_ok());

        // Test invalid sample rate
        settings.sample_rate = 0;
        assert!(settings.validate().is_err());
        settings.sample_rate = 48000;

        // Test invalid channels
        settings.channels = 0;
        assert!(settings.validate().is_err());
        settings.channels = 3;
        assert!(settings.validate().is_err());
        settings.channels = 2;

        // Test invalid bit depth for WAV
        settings.format = AudioFormat::Wav;
        settings.bit_depth = 32;
        assert!(settings.validate().is_err());
        settings.bit_depth = 24;
        assert!(settings.validate().is_ok());

        // Test invalid time range
        settings.start_time = -1.0;
        assert!(settings.validate().is_err());
        settings.start_time = 0.0;

        settings.end_time = 0.0;
        assert!(settings.validate().is_err());
        settings.end_time = 60.0;

        assert!(settings.validate().is_ok());
    }

    #[test]
    fn test_audio_presets() {
        let wav = AudioExportSettings::high_quality_wav();
        assert_eq!(wav.format, AudioFormat::Wav);
        assert_eq!(wav.sample_rate, 48000);
        assert_eq!(wav.bit_depth, 24);
        assert_eq!(wav.channels, 2);

        let flac = AudioExportSettings::high_quality_flac();
        assert_eq!(flac.format, AudioFormat::Flac);
        assert_eq!(flac.sample_rate, 48000);
        assert_eq!(flac.bit_depth, 24);

        let aac = AudioExportSettings::high_quality_aac();
        assert_eq!(aac.format, AudioFormat::Aac);
        assert_eq!(aac.bitrate_kbps, 320);

        let mp3 = AudioExportSettings::podcast_mp3();
        assert_eq!(mp3.format, AudioFormat::Mp3);
        assert_eq!(mp3.channels, 1);
        assert_eq!(mp3.bitrate_kbps, 128);
    }

    #[test]
    fn test_video_codec_container() {
        assert_eq!(VideoCodec::H264.container_format(), "mp4");
        assert_eq!(VideoCodec::VP9.container_format(), "webm");
        assert_eq!(VideoCodec::ProRes422.container_format(), "mov");
    }

    #[test]
    fn test_video_quality_bitrate() {
        assert_eq!(VideoQuality::Low.bitrate_kbps(), 2000);
        assert_eq!(VideoQuality::High.bitrate_kbps(), 10000);
        assert_eq!(VideoQuality::Custom(15000).bitrate_kbps(), 15000);
    }

    #[test]
    fn test_video_export_total_frames() {
        let settings = VideoExportSettings {
            framerate: 30.0,
            start_time: 0.0,
            end_time: 10.0,
            ..Default::default()
        };
        assert_eq!(settings.total_frames(), 300);
    }

    #[test]
    fn test_gif_frame_delay_and_frames() {
        // Frame rates that map to clean centisecond delays.
        let mk = |fps: f64| GifExportSettings { framerate: fps, ..Default::default() };
        assert_eq!(mk(10.0).frame_delay_ms(), 100); // 100 ms
        assert_eq!(mk(20.0).frame_delay_ms(), 50);  // 50 ms
        assert_eq!(mk(50.0).frame_delay_ms(), 20);  // 20 ms
        // 15 fps = 66.6 ms rounds to 70 ms (7 cs); 25 fps = 40 ms.
        assert_eq!(mk(15.0).frame_delay_ms(), 70);
        assert_eq!(mk(25.0).frame_delay_ms(), 40);
        // Very high fps clamps to the 10 ms minimum (1 cs).
        assert_eq!(mk(1000.0).frame_delay_ms(), 10);

        let settings = GifExportSettings { framerate: 20.0, start_time: 0.0, end_time: 3.0, ..Default::default() };
        assert_eq!(settings.total_frames(), 60);
    }

    #[test]
    fn test_gif_validate() {
        let mut s = GifExportSettings::default();
        assert!(s.validate().is_ok());
        s.framerate = 0.0;
        assert!(s.validate().is_err());
        s = GifExportSettings { start_time: 5.0, end_time: 2.0, ..Default::default() };
        assert!(s.validate().is_err());
    }

    #[test]
    fn test_export_progress_percentage() {
        let progress = ExportProgress::FrameRendered { frame: 50, total: 100 };
        assert_eq!(progress.progress_percentage(), Some(0.5));

        let complete = ExportProgress::Complete {
            output_path: std::path::PathBuf::from("test.mp4"),
        };
        assert_eq!(complete.progress_percentage(), Some(1.0));
    }
}

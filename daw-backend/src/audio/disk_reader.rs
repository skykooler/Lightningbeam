//! Disk reader for streaming audio playback.
//!
//! Provides lock-free read-ahead buffers for audio files that cannot be kept
//! fully decoded in memory. A background thread fills these buffers ahead of
//! the playhead so the audio callback never blocks on I/O or decoding.
//!
//! **InMemory** files bypass the disk reader entirely — their data is already
//! available as `&[f32]`. **Mapped** files (mmap'd WAV/AIFF) also bypass the
//! disk reader for now (OS page cache handles paging). **Compressed** files
//! (MP3, FLAC, OGG, etc.) use a `CompressedReader` that stream-decodes on
//! demand via Symphonia into a `ReadAheadBuffer`.

use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::{FormatOptions, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Read-ahead distance in seconds.
const PREFETCH_SECONDS: f64 = 2.0;

/// How often the disk reader thread wakes up to check for work (ms).
const POLL_INTERVAL_MS: u64 = 5;

// ---------------------------------------------------------------------------
// ReadAheadBuffer
// ---------------------------------------------------------------------------

/// Lock-free read-ahead buffer shared between the disk reader (writer) and the
/// audio callback (reader).
///
/// # Thread safety
///
/// This is a **single-producer single-consumer** (SPSC) structure:
/// - **Producer** (disk reader thread): calls `write_samples()` and
///   `advance_start()` to fill and reclaim buffer space.
/// - **Consumer** (audio callback): calls `read_sample()` and `has_range()`
///   to access decoded audio.
///
/// The producer only writes to indices **beyond** `valid_frames`, while the
/// consumer only reads indices **within** `[start_frame, start_frame +
/// valid_frames)`. Because the two threads always operate on disjoint regions,
/// the sample data itself requires no locking. Atomics with Acquire/Release
/// ordering on `start_frame` and `valid_frames` provide the happens-before
/// relationship that guarantees the consumer sees completed writes.
///
/// The `UnsafeCell` wrapping the buffer data allows the producer to mutate it
/// through a shared `&self` reference. This is sound because only one thread
/// (the producer) ever writes, and it writes to a region that the consumer
/// cannot yet see (gated by the `valid_frames` atomic).
pub struct ReadAheadBuffer {
    /// Interleaved f32 samples stored as a circular buffer.
    /// Wrapped in `UnsafeCell` to allow the producer to write through `&self`.
    buffer: UnsafeCell<Box<[f32]>>,
    /// The absolute frame number of the oldest valid frame in the ring.
    start_frame: AtomicU64,
    /// Number of valid frames starting from `start_frame`.
    valid_frames: AtomicU64,
    /// Total capacity in frames.
    capacity_frames: usize,
    /// Number of audio channels.
    channels: u32,
    /// Source file sample rate.
    sample_rate: u32,
    /// Last file-local frame requested by the audio callback.
    /// Written by the consumer (render_from_file), read by the disk reader.
    /// The disk reader uses this instead of the global playhead to know
    /// where in the file to buffer around.
    target_frame: AtomicU64,
    /// When true, `render_from_file` will block-wait for frames instead of
    /// returning silence on buffer miss. Used during offline export.
    export_mode: AtomicBool,
    /// Set by the disk reader when the source reaches EOF (no more frames will
    /// ever be decoded past the current valid range). Lets export-mode block-waits
    /// give up immediately for frames past the end of the audio (e.g. a video whose
    /// audio track is shorter than the video) instead of timing out per chunk.
    /// Cleared on `reset` (a seek may decode fresh data again).
    finished: AtomicBool,
}

// SAFETY: See the doc comment on ReadAheadBuffer for the full safety argument.
// In short: SPSC access pattern with atomic coordination means no data races.
// The circular design means advance_start never moves data — it only bumps
// the start pointer, so the consumer never sees partially-shifted memory.
unsafe impl Send for ReadAheadBuffer {}
unsafe impl Sync for ReadAheadBuffer {}

impl std::fmt::Debug for ReadAheadBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReadAheadBuffer")
            .field("capacity_frames", &self.capacity_frames)
            .field("channels", &self.channels)
            .field("sample_rate", &self.sample_rate)
            .field("start_frame", &self.start_frame.load(Ordering::Relaxed))
            .field("valid_frames", &self.valid_frames.load(Ordering::Relaxed))
            .finish()
    }
}

impl ReadAheadBuffer {
    /// Create a new read-ahead buffer with the given capacity (in seconds).
    pub fn new(capacity_seconds: f64, sample_rate: u32, channels: u32) -> Self {
        let capacity_frames = (capacity_seconds * sample_rate as f64) as usize;
        let buffer_len = capacity_frames * channels as usize;
        Self {
            buffer: UnsafeCell::new(vec![0.0f32; buffer_len].into_boxed_slice()),
            start_frame: AtomicU64::new(0),
            valid_frames: AtomicU64::new(0),
            capacity_frames,
            channels,
            sample_rate,
            target_frame: AtomicU64::new(0),
            export_mode: AtomicBool::new(false),
            finished: AtomicBool::new(false),
        }
    }

    /// Map an absolute frame number to a ring-buffer sample index.
    #[inline(always)]
    fn ring_index(&self, frame: u64, channel: usize) -> usize {
        let ring_frame = (frame as usize) % self.capacity_frames;
        ring_frame * self.channels as usize + channel
    }

    /// Snapshot the current valid range. Call once per audio callback, then
    /// pass the returned `(start, end)` to `read_sample` for consistent reads.
    #[inline]
    pub fn snapshot(&self) -> (u64, u64) {
        let start = self.start_frame.load(Ordering::Acquire);
        let valid = self.valid_frames.load(Ordering::Acquire);
        (start, start + valid)
    }

    /// Read a single interleaved sample using a pre-loaded range snapshot.
    /// Returns `0.0` if the frame is outside `[snap_start, snap_end)`.
    /// Called from the **audio callback** (consumer).
    #[inline]
    pub fn read_sample(&self, frame: u64, channel: usize, snap_start: u64, snap_end: u64) -> f32 {
        if frame < snap_start || frame >= snap_end {
            return 0.0;
        }

        let idx = self.ring_index(frame, channel);
        // SAFETY: We only read indices that the producer has already written
        // and published via valid_frames. The circular layout means
        // advance_start never moves data, so no torn reads are possible.
        let buffer = unsafe { &*self.buffer.get() };
        buffer[idx]
    }

    /// Check whether a contiguous range of frames is fully available.
    #[inline]
    pub fn has_range(&self, start: u64, count: u64) -> bool {
        let buf_start = self.start_frame.load(Ordering::Acquire);
        let valid = self.valid_frames.load(Ordering::Acquire);
        start >= buf_start && start + count <= buf_start + valid
    }

    /// Current start frame of the buffer.
    #[inline]
    pub fn start_frame(&self) -> u64 {
        self.start_frame.load(Ordering::Acquire)
    }

    /// Number of valid frames currently in the buffer.
    #[inline]
    pub fn valid_frames_count(&self) -> u64 {
        self.valid_frames.load(Ordering::Acquire)
    }

    /// Update the target frame — the file-local frame the audio callback
    /// is currently reading from. Called by `render_from_file` (consumer).
    /// Each clip instance has its own buffer, so a plain store is sufficient.
    #[inline]
    pub fn set_target_frame(&self, frame: u64) {
        self.target_frame.store(frame, Ordering::Relaxed);
    }

    /// Reset the target frame to MAX before a new render cycle.
    /// If no clip calls `set_target_frame` this cycle, `has_active_target()`
    /// returns false, telling the disk reader to skip this buffer.
    #[inline]
    pub fn reset_target_frame(&self) {
        self.target_frame.store(u64::MAX, Ordering::Relaxed);
    }

    /// Force-set the target frame to an exact value.
    /// Used by the disk reader's seek command where we need an absolute position.
    #[inline]
    pub fn force_target_frame(&self, frame: u64) {
        self.target_frame.store(frame, Ordering::Relaxed);
    }

    /// Get the target frame set by the audio callback.
    /// Called by the disk reader thread (producer).
    #[inline]
    pub fn target_frame(&self) -> u64 {
        self.target_frame.load(Ordering::Relaxed)
    }

    /// Check if any clip set a target this cycle (vs still at reset value).
    #[inline]
    pub fn has_active_target(&self) -> bool {
        self.target_frame.load(Ordering::Relaxed) != u64::MAX
    }

    /// Enable or disable export (blocking) mode. When enabled,
    /// `render_from_file` will spin-wait for frames instead of returning
    /// silence on buffer miss.
    pub fn set_export_mode(&self, export: bool) {
        self.export_mode.store(export, Ordering::Release);
    }

    /// Check if export (blocking) mode is active.
    pub fn is_export_mode(&self) -> bool {
        self.export_mode.load(Ordering::Acquire)
    }

    /// Mark that the source has reached EOF (set by the disk reader).
    pub fn set_finished(&self, finished: bool) {
        self.finished.store(finished, Ordering::Release);
    }

    /// True once the source hit EOF: no frames past the current valid range
    /// will ever be decoded (until a `reset`/seek).
    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    /// Reset the buffer to start at `new_start` with zero valid frames.
    /// Called by the **disk reader thread** (producer) after a seek.
    pub fn reset(&self, new_start: u64) {
        self.valid_frames.store(0, Ordering::Release);
        self.start_frame.store(new_start, Ordering::Release);
        // A seek may decode fresh data past the previous EOF position.
        self.finished.store(false, Ordering::Release);
    }

    /// Write interleaved samples into the buffer, extending the valid range.
    /// Called by the **disk reader thread** (producer only).
    /// Returns the number of frames actually written (may be less than `frames`
    /// if the buffer is full).
    ///
    /// # Safety
    /// Must only be called from the single producer thread.
    pub fn write_samples(&self, samples: &[f32], frames: usize) -> usize {
        let valid = self.valid_frames.load(Ordering::Acquire) as usize;
        let remaining_capacity = self.capacity_frames - valid;
        let write_frames = frames.min(remaining_capacity);
        if write_frames == 0 {
            return 0;
        }

        let ch = self.channels as usize;
        let start = self.start_frame.load(Ordering::Acquire);
        let write_start_frame = start as usize + valid;

        // SAFETY: We only write to ring positions beyond the current valid
        // range, which the consumer cannot access. Only one producer calls this.
        let buffer = unsafe { &mut *self.buffer.get() };

        // Write with wrap-around: the ring position may cross the buffer end.
        let ring_start = (write_start_frame % self.capacity_frames) * ch;
        let total_samples = write_frames * ch;

        let buffer_sample_len = self.capacity_frames * ch;
        let first_chunk = total_samples.min(buffer_sample_len - ring_start);

        buffer[ring_start..ring_start + first_chunk]
            .copy_from_slice(&samples[..first_chunk]);

        if first_chunk < total_samples {
            // Wrap around to the beginning of the buffer.
            let second_chunk = total_samples - first_chunk;
            buffer[..second_chunk]
                .copy_from_slice(&samples[first_chunk..first_chunk + second_chunk]);
        }

        // Make the new samples visible to the consumer.
        self.valid_frames
            .store((valid + write_frames) as u64, Ordering::Release);

        write_frames
    }

    /// Advance the buffer start, discarding frames behind the playhead.
    /// Called by the **disk reader thread** (producer only) to reclaim space.
    ///
    /// Because this is a circular buffer, advancing the start only updates
    /// atomic counters — no data is moved, so the consumer never sees
    /// partially-shifted memory.
    pub fn advance_start(&self, new_start: u64) {
        let old_start = self.start_frame.load(Ordering::Acquire);
        if new_start <= old_start {
            return;
        }

        let advance_frames = (new_start - old_start) as usize;
        let valid = self.valid_frames.load(Ordering::Acquire) as usize;

        if advance_frames >= valid {
            // All data is stale — just reset.
            self.valid_frames.store(0, Ordering::Release);
            self.start_frame.store(new_start, Ordering::Release);
            return;
        }

        let new_valid = valid - advance_frames;
        // Store valid_frames first (shrinking the visible range), then
        // advance start_frame. The consumer always sees a consistent
        // sub-range of valid data.
        self.valid_frames
            .store(new_valid as u64, Ordering::Release);
        self.start_frame.store(new_start, Ordering::Release);
    }
}

// ---------------------------------------------------------------------------
// CompressedReader
// ---------------------------------------------------------------------------

/// Wraps a Symphonia decoder for streaming a single compressed audio file.
///
/// Public (like [`VideoAudioReader`]) only so integration tests can exercise it
/// directly; treat it as crate-internal.
pub struct CompressedReader {
    format_reader: Box<dyn symphonia::core::formats::FormatReader>,
    decoder: Box<dyn symphonia::core::codecs::Decoder>,
    track_id: u32,
    /// Current decoder position in frames.
    current_frame: u64,
    /// Frames still to drop from the front of decoded output, so that after a
    /// (coarse) seek the next emitted sample lands exactly on the target frame.
    pending_discard: u64,
    sample_rate: u32,
    channels: u32,
    total_frames: u64,
    /// Temporary decode buffer.
    sample_buf: Option<SampleBuffer<f32>>,
}

/// A seekable byte stream for packed media held in the host's project container.
///
/// `daw-backend` stays container-agnostic: it never references the `.beam` SQLite
/// store directly. Instead the host (lightningbeam-core) implements this trait over
/// its incremental blob reader and installs a factory ([`AudioBlobSourceFactory`])
/// into the engine, so packed compressed audio can be stream-decoded without ever
/// being fully loaded into RAM.
pub trait MediaByteSource: std::io::Read + std::io::Seek + Send + Sync {
    /// Total length of the stream in bytes (Symphonia needs this for seeking).
    fn byte_len(&self) -> u64;
}

/// Opens fresh byte streams for packed media by id. Installed into the engine by
/// the host; invoked when activating a clip backed by container-packed audio.
/// (`Debug` so it can ride in the `Query` enum, which derives `Debug`.)
pub trait AudioBlobSourceFactory: Send + Sync + std::fmt::Debug {
    /// Open a new independent reader for the packed media item `media_id`
    /// (the UUID string stored on the audio pool entry).
    fn open(&self, media_id: &str) -> Result<Box<dyn MediaByteSource>, String>;
}

/// Adapts a [`MediaByteSource`] to Symphonia's `MediaSource` (adds the seekable +
/// byte-length metadata Symphonia's probe/seek require).
struct SymphoniaByteSource(Box<dyn MediaByteSource>);

impl std::io::Read for SymphoniaByteSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}
impl std::io::Seek for SymphoniaByteSource {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.0.seek(pos)
    }
}
impl symphonia::core::io::MediaSource for SymphoniaByteSource {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.0.byte_len())
    }
}

/// How to open a streaming audio source: a filesystem path (referenced media or a
/// video file) or a host-provided byte stream (container-packed media).
pub enum StreamOpen {
    Path(PathBuf),
    Source {
        src: Box<dyn MediaByteSource>,
        /// Codec/extension hint for the Symphonia probe (e.g. `"mp3"`, `"flac"`).
        ext: Option<String>,
    },
}

impl CompressedReader {
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u32 {
        self.channels
    }

    /// Total frames from the codec header (0 if the format doesn't report it).
    pub fn total_frames(&self) -> u64 {
        self.total_frames
    }

    /// Open a compressed audio file and prepare for streaming decode.
    pub fn open(path: &Path) -> Result<Self, String> {
        let file =
            std::fs::File::open(path).map_err(|e| format!("Failed to open file: {}", e))?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());

        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }
        Self::from_mss(mss, hint)
    }

    /// Open a compressed stream from a host-provided byte source (packed media).
    pub fn open_source(src: Box<dyn MediaByteSource>, ext: Option<&str>) -> Result<Self, String> {
        let mss = MediaSourceStream::new(Box::new(SymphoniaByteSource(src)), Default::default());
        let mut hint = Hint::new();
        if let Some(ext) = ext {
            hint.with_extension(ext);
        }
        Self::from_mss(mss, hint)
    }

    /// Shared probe + decoder setup over an already-built media stream.
    fn from_mss(mss: MediaSourceStream, hint: Hint) -> Result<Self, String> {
        let probed = symphonia::default::get_probe()
            .format(
                &hint,
                mss,
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )
            .map_err(|e| format!("Failed to probe file: {}", e))?;

        let format_reader = probed.format;

        let track = format_reader
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
            .ok_or_else(|| "No audio tracks found".to_string())?;

        let track_id = track.id;
        let codec_params = &track.codec_params;
        let sample_rate = codec_params.sample_rate.unwrap_or(44100);
        let channels = codec_params
            .channels
            .map(|c| c.count())
            .unwrap_or(2) as u32;
        let total_frames = codec_params.n_frames.unwrap_or(0);

        let decoder = symphonia::default::get_codecs()
            .make(codec_params, &DecoderOptions::default())
            .map_err(|e| format!("Failed to create decoder: {}", e))?;

        Ok(Self {
            format_reader,
            decoder,
            track_id,
            current_frame: 0,
            pending_discard: 0,
            sample_rate,
            channels,
            total_frames,
            sample_buf: None,
        })
    }

    /// Seek to `target_frame`, **sample-accurately**. Uses `SeekMode::Accurate`:
    /// for an elementary stream like MP3 a *coarse* seek byte-estimates the
    /// position and seeds the timestamp from that estimate — which for VBR (or a
    /// file whose header padding the estimate ignores) lands off by up to ~1s.
    /// Accurate mode instead counts frame *headers* (no decode) from a true anchor
    /// (the current position, or a rewind to the start for backward seeks), so the
    /// returned `actual_ts` is exact; the small residual to `target_frame` is then
    /// dropped in `decode_next`. Container formats with seek tables (FLAC/OGG) seek
    /// cheaply; a long MP3 walks headers from the anchor (I/O, not decode) — a
    /// per-file seek index would make that O(1) (future work).
    pub fn seek(&mut self, target_frame: u64) -> Result<u64, String> {
        let seek_to = SeekTo::TimeStamp {
            ts: target_frame,
            track_id: self.track_id,
        };

        let seeked = self
            .format_reader
            .seek(SeekMode::Accurate, seek_to)
            .map_err(|e| format!("Seek failed: {}", e))?;

        let actual_frame = seeked.actual_ts;
        self.current_frame = actual_frame;
        // Drop the frames between where the coarse seek landed and the target.
        self.pending_discard = target_frame.saturating_sub(actual_frame);

        // Reset the decoder after seeking.
        self.decoder.reset();

        Ok(target_frame)
    }

    /// Decode the next chunk of audio into `out`. Returns the number of frames
    /// decoded. Returns `Ok(0)` at end-of-file.
    pub fn decode_next(&mut self, out: &mut Vec<f32>) -> Result<usize, String> {
        out.clear();

        loop {
            let packet = match self.format_reader.next_packet() {
                Ok(p) => p,
                Err(symphonia::core::errors::Error::IoError(ref e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    return Ok(0); // EOF
                }
                Err(e) => return Err(format!("Read packet error: {}", e)),
            };

            if packet.track_id() != self.track_id {
                continue;
            }

            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    if self.sample_buf.is_none() {
                        let spec = *decoded.spec();
                        let duration = decoded.capacity() as u64;
                        self.sample_buf = Some(SampleBuffer::new(duration, spec));
                    }

                    if let Some(ref mut buf) = self.sample_buf {
                        buf.copy_interleaved_ref(decoded);
                        let samples = buf.samples();
                        let ch = self.channels as usize;
                        let frames = samples.len() / ch;

                        // Drop leading frames for sample-accurate seek alignment.
                        let discard = self.pending_discard.min(frames as u64) as usize;
                        self.pending_discard -= discard as u64;
                        out.extend_from_slice(&samples[discard * ch..]);
                        let emitted = frames - discard;
                        self.current_frame += emitted as u64;

                        if emitted > 0 {
                            return Ok(emitted);
                        }
                        // Whole packet discarded for alignment — keep decoding so
                        // we never falsely report EOF (Ok(0)).
                        continue;
                    }

                    return Ok(0);
                }
                Err(symphonia::core::errors::Error::DecodeError(_)) => {
                    continue; // Skip corrupt packets.
                }
                Err(e) => return Err(format!("Decode error: {}", e)),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// VideoAudioReader
// ---------------------------------------------------------------------------

/// Streams the audio track out of a media file (a video container, or any audio
/// file) using FFmpeg, decoding on demand. Mirrors [`CompressedReader`]'s
/// interface so the disk reader can drive either through [`StreamSource`].
///
/// Seeking is **sample-accurate**: after `seek(target)`, the next `decode_next`
/// yields samples beginning at exactly `target`. FFmpeg's container seek only
/// lands at-or-before the target, so we decode forward and discard the leading
/// samples to hit the frame precisely — this keeps video audio frame-synced with
/// other (mmap/in-memory) clips.
///
/// Public (vs. the private `CompressedReader`) only so integration tests can
/// exercise it directly; treat it as crate-internal.
/// Adapts a host `MediaByteSource` (Read+Seek+Send+Sync) to the `ffmpeg-blob-io`
/// `BlobSource` (Read+Seek+Send) so a packed video can be demuxed from a blob.
struct ByteSourceAdapter(Box<dyn MediaByteSource>);
impl std::io::Read for ByteSourceAdapter {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}
impl std::io::Seek for ByteSourceAdapter {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.0.seek(pos)
    }
}

/// A demuxer input for video-audio: a video file path, or a container blob streamed
/// via the AVIO shim. Both expose the same ffmpeg `Input`.
enum AudioInput {
    Path(ffmpeg_next::format::context::Input),
    Blob(ffmpeg_blob_io::BlobInput),
}
impl AudioInput {
    fn get(&mut self) -> &mut ffmpeg_next::format::context::Input {
        match self {
            AudioInput::Path(i) => i,
            AudioInput::Blob(b) => b.input_mut(),
        }
    }
}

pub struct VideoAudioReader {
    input: AudioInput,
    decoder: ffmpeg_next::decoder::Audio,
    /// Built lazily from the first decoded frame's format/layout → interleaved f32.
    resampler: Option<ffmpeg_next::software::resampling::Context>,
    stream_index: usize,
    /// Seconds per stream-timestamp unit.
    time_base: f64,
    sample_rate: u32,
    channels: u32,
    total_frames: u64,
    /// Absolute frame index of the next sample `decode_next` will output.
    current_frame: u64,
    /// Frames still to drop from the front of decoded output (seek alignment).
    pending_discard: u64,
    /// When set, the next decoded frame establishes the discard needed to land on
    /// this absolute target frame (sample-accurate seek).
    align_to: Option<u64>,
}

impl VideoAudioReader {
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u32 {
        self.channels
    }

    /// Estimated total audio frames (from the stream/container duration).
    pub fn total_frames(&self) -> u64 {
        self.total_frames
    }

    /// Open the audio track of a video **file**.
    pub fn open(path: &Path) -> Result<Self, String> {
        ffmpeg_next::init().map_err(|e| e.to_string())?;
        let input = ffmpeg_next::format::input(&path)
            .map_err(|e| format!("Failed to open media: {}", e))?;
        Self::from_input(AudioInput::Path(input))
    }

    /// Open the audio track of a video streamed from a **byte source** (a packed
    /// `.beam` video blob), via the `ffmpeg-blob-io` AVIO shim. `ext` is a container
    /// hint (e.g. `"mp4"`).
    pub fn open_source(src: Box<dyn MediaByteSource>, ext: Option<&str>) -> Result<Self, String> {
        ffmpeg_next::init().map_err(|e| e.to_string())?;
        let blob = ffmpeg_blob_io::BlobInput::open(Box::new(ByteSourceAdapter(src)), ext)
            .map_err(|e| format!("Failed to open packed video audio: {}", e))?;
        Self::from_input(AudioInput::Blob(blob))
    }

    fn from_input(mut input: AudioInput) -> Result<Self, String> {
        // Pull stream scalars + build the decoder inside a scope so the stream
        // borrow of `input` ends before we use `input` again.
        let (stream_index, time_base, stream_duration, decoder) = {
            let stream = input
                .get()
                .streams()
                .best(ffmpeg_next::media::Type::Audio)
                .ok_or_else(|| "No audio stream found".to_string())?;
            let stream_index = stream.index();
            let time_base = f64::from(stream.time_base());
            let stream_duration = stream.duration();
            let ctx = ffmpeg_next::codec::context::Context::from_parameters(stream.parameters())
                .map_err(|e| e.to_string())?;
            let decoder = ctx.decoder().audio().map_err(|e| e.to_string())?;
            (stream_index, time_base, stream_duration, decoder)
        };

        let sample_rate = decoder.rate();
        let channels = decoder.channels() as u32;

        let duration_secs = if stream_duration > 0 {
            stream_duration as f64 * time_base
        } else if input.get().duration() > 0 {
            input.get().duration() as f64 / f64::from(ffmpeg_next::ffi::AV_TIME_BASE)
        } else {
            0.0
        };
        let total_frames = (duration_secs * sample_rate as f64).max(0.0) as u64;

        Ok(Self {
            input,
            decoder,
            resampler: None,
            stream_index,
            time_base,
            sample_rate,
            channels,
            total_frames,
            current_frame: 0,
            pending_discard: 0,
            align_to: None,
        })
    }

    pub fn seek(&mut self, target_frame: u64) -> Result<u64, String> {
        let seconds = target_frame as f64 / self.sample_rate.max(1) as f64;
        let ts_av = (seconds * f64::from(ffmpeg_next::ffi::AV_TIME_BASE)) as i64;
        // Seek to at-or-before the target (max_ts = ts_av) so we can decode
        // forward to it exactly. ffmpeg-next's `seek` wants a bounded range.
        self.input
            .get()
            .seek(ts_av, 0..(ts_av + 1))
            .map_err(|e| format!("Seek failed: {}", e))?;
        self.decoder.flush();
        self.pending_discard = 0;
        self.align_to = Some(target_frame);
        self.current_frame = target_frame;
        // We align to the exact frame below, so the effective position IS target.
        Ok(target_frame)
    }

    pub fn decode_next(&mut self, out: &mut Vec<f32>) -> Result<usize, String> {
        out.clear();
        loop {
            // Drain a decoded frame if one is ready.
            let mut decoded = ffmpeg_next::frame::Audio::empty();
            if self.decoder.receive_frame(&mut decoded).is_ok() {
                self.ensure_layout(&mut decoded);
                let n = self.emit(&decoded, out);
                if n > 0 {
                    return Ok(n);
                }
                continue; // frame fully discarded by seek-alignment; keep going
            }

            // Read one packet (owned), releasing the `input` borrow before decoding.
            let packet = self.input.get().packets().next().map(|(_, p)| p);
            match packet {
                Some(packet) => {
                    if packet.stream() == self.stream_index {
                        self.decoder
                            .send_packet(&packet)
                            .map_err(|e| e.to_string())?;
                    }
                }
                None => {
                    // EOF: flush and drain whatever remains.
                    let _ = self.decoder.send_eof();
                    let mut decoded = ffmpeg_next::frame::Audio::empty();
                    if self.decoder.receive_frame(&mut decoded).is_ok() {
                        self.ensure_layout(&mut decoded);
                        return Ok(self.emit(&decoded, out));
                    }
                    return Ok(0);
                }
            }
        }
    }

    /// Decoders for some formats (e.g. raw mono WAV) leave the frame's channel
    /// layout unset. The resampler needs a concrete layout that matches the
    /// frame, so fill one in from the channel count when it's missing.
    fn ensure_layout(&self, frame: &mut ffmpeg_next::frame::Audio) {
        if frame.channel_layout().is_empty() {
            frame.set_channel_layout(
                ffmpeg_next::channel_layout::ChannelLayout::default(self.channels as i32),
            );
        }
    }

    /// Resample one decoded frame to interleaved f32, apply any pending
    /// seek-alignment discard, append to `out`, return frames emitted.
    fn emit(&mut self, frame: &ffmpeg_next::frame::Audio, out: &mut Vec<f32>) -> usize {
        // `frame` already has a non-empty channel layout (set by `ensure_layout`
        // before this call), so the resampler config and the actual frame agree
        // — otherwise swr fails with AVERROR_INPUT_CHANGED.
        if self.resampler.is_none() {
            match ffmpeg_next::software::resampling::Context::get(
                frame.format(),
                frame.channel_layout(),
                self.sample_rate,
                ffmpeg_next::format::Sample::F32(ffmpeg_next::format::sample::Type::Packed),
                frame.channel_layout(),
                self.sample_rate,
            ) {
                Ok(r) => self.resampler = Some(r),
                Err(_) => return 0,
            }
        }

        let mut resampled = ffmpeg_next::frame::Audio::empty();
        if self
            .resampler
            .as_mut()
            .unwrap()
            .run(frame, &mut resampled)
            .is_err()
        {
            return 0;
        }

        // The output is packed (interleaved) f32. Read it from the raw byte plane
        // `data(0)` — its length is correct (`frames * channels * 4`), whereas
        // `plane::<f32>(0)` is a known ffmpeg-next footgun that reports only
        // `samples()` elements (ignoring channels) and would slice out of range
        // for multi-channel audio.
        let ch = self.channels.max(1) as usize;
        let bytes = resampled.data(0);
        let n_frames = (bytes.len() / 4) / ch;
        if n_frames == 0 {
            return 0;
        }

        // On the first frame after a seek, compute how many leading frames to
        // drop so output begins exactly at the seek target.
        if let Some(target) = self.align_to.take() {
            let frame_start = self.pts_to_frame(frame.pts());
            self.pending_discard = target.saturating_sub(frame_start);
        }

        let discard = (self.pending_discard.min(n_frames as u64)) as usize;
        self.pending_discard -= discard as u64;

        let start_byte = discard * ch * 4;
        let end_byte = n_frames * ch * 4;
        out.extend(
            bytes[start_byte..end_byte]
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        );
        let emitted = n_frames - discard;
        self.current_frame += emitted as u64;
        emitted
    }

    /// Convert a stream PTS to an absolute audio frame index.
    fn pts_to_frame(&self, pts: Option<i64>) -> u64 {
        match pts {
            Some(p) if p >= 0 => {
                ((p as f64 * self.time_base) * self.sample_rate as f64).round() as u64
            }
            _ => self.current_frame,
        }
    }
}

// ---------------------------------------------------------------------------
// StreamSource — dispatches the disk reader over either decoder backend.
// (Wired into the reader thread in a later step.)
// ---------------------------------------------------------------------------

/// Which decoder backend a streaming source uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// Symphonia, for compressed audio files (MP3, FLAC, OGG, …).
    CompressedAudio,
    /// FFmpeg, for the audio track of a video container.
    VideoAudio,
}

/// A streaming audio source backing one active clip: either Symphonia
/// ([`CompressedReader`]) or FFmpeg ([`VideoAudioReader`]).
enum StreamSource {
    Compressed(CompressedReader),
    Video(VideoAudioReader),
}

impl StreamSource {
    fn open(open: StreamOpen, kind: SourceKind) -> Result<Self, String> {
        match (kind, open) {
            (SourceKind::CompressedAudio, StreamOpen::Path(p)) => {
                Ok(StreamSource::Compressed(CompressedReader::open(&p)?))
            }
            (SourceKind::CompressedAudio, StreamOpen::Source { src, ext }) => {
                Ok(StreamSource::Compressed(CompressedReader::open_source(src, ext.as_deref())?))
            }
            (SourceKind::VideoAudio, StreamOpen::Path(p)) => {
                Ok(StreamSource::Video(VideoAudioReader::open(&p)?))
            }
            (SourceKind::VideoAudio, StreamOpen::Source { src, ext }) => {
                Ok(StreamSource::Video(VideoAudioReader::open_source(src, ext.as_deref())?))
            }
        }
    }

    fn sample_rate(&self) -> u32 {
        match self {
            StreamSource::Compressed(r) => r.sample_rate,
            StreamSource::Video(r) => r.sample_rate,
        }
    }

    fn channels(&self) -> u32 {
        match self {
            StreamSource::Compressed(r) => r.channels,
            StreamSource::Video(r) => r.channels,
        }
    }

    fn seek(&mut self, target_frame: u64) -> Result<u64, String> {
        match self {
            StreamSource::Compressed(r) => r.seek(target_frame),
            StreamSource::Video(r) => r.seek(target_frame),
        }
    }

    fn decode_next(&mut self, out: &mut Vec<f32>) -> Result<usize, String> {
        match self {
            StreamSource::Compressed(r) => r.decode_next(out),
            StreamSource::Video(r) => r.decode_next(out),
        }
    }

    fn total_frames(&self) -> u64 {
        match self {
            StreamSource::Compressed(r) => r.total_frames(),
            StreamSource::Video(r) => r.total_frames(),
        }
    }
}

/// Decode a media source end-to-end and build its [`WaveformPyramid`] overview,
/// streaming — only one decode chunk plus the (bounded) pyramid are ever in
/// memory, never the full sample buffer. `floor_samples_per_texel` is the
/// finest-level resolution (see [`crate::audio::waveform_pyramid`]).
pub fn build_waveform_pyramid(
    path: &Path,
    kind: SourceKind,
    floor_samples_per_texel: u32,
) -> Result<crate::audio::waveform_pyramid::WaveformPyramid, String> {
    let src = StreamSource::open(StreamOpen::Path(path.to_path_buf()), kind)?;
    build_pyramid_from_streamsource(src, floor_samples_per_texel)
}

/// Build a waveform pyramid from a host-provided byte source (container-packed
/// compressed audio) — the load-time counterpart of [`build_waveform_pyramid`]
/// for media that has no filesystem path.
pub fn build_waveform_pyramid_from_source(
    src: Box<dyn MediaByteSource>,
    ext: Option<&str>,
    floor_samples_per_texel: u32,
) -> Result<crate::audio::waveform_pyramid::WaveformPyramid, String> {
    let src = StreamSource::open(
        StreamOpen::Source { src, ext: ext.map(|s| s.to_string()) },
        SourceKind::CompressedAudio,
    )?;
    build_pyramid_from_streamsource(src, floor_samples_per_texel)
}

fn build_pyramid_from_streamsource(
    mut src: StreamSource,
    floor_samples_per_texel: u32,
) -> Result<crate::audio::waveform_pyramid::WaveformPyramid, String> {
    use crate::audio::waveform_pyramid::WaveformPyramidBuilder;

    let channels = src.channels();
    let mut builder = WaveformPyramidBuilder::new(channels, floor_samples_per_texel);
    builder.reserve_for_frames(src.total_frames());

    let mut buf = Vec::new();
    loop {
        let frames = src.decode_next(&mut buf)?;
        if frames == 0 {
            break;
        }
        builder.push_interleaved(&buf);
    }
    Ok(builder.finish())
}

// ---------------------------------------------------------------------------
// DiskReaderCommand
// ---------------------------------------------------------------------------

/// Commands sent from the engine to the disk reader thread.
pub enum DiskReaderCommand {
    /// Start streaming a file for a clip instance, using the decoder backend
    /// selected by `kind` (compressed audio vs. a video's audio track). `open`
    /// is either a filesystem path (referenced media / video) or a host-provided
    /// byte stream (container-packed media).
    ActivateFile {
        reader_id: u64,
        open: StreamOpen,
        kind: SourceKind,
        buffer: Arc<ReadAheadBuffer>,
    },
    /// Stop streaming for a clip instance.
    DeactivateFile { reader_id: u64 },
    /// The playhead has jumped — refill buffers from the new position.
    Seek { frame: u64 },
    /// Shut down the disk reader thread.
    Shutdown,
}

// ---------------------------------------------------------------------------
// DiskReader
// ---------------------------------------------------------------------------

/// Manages background read-ahead for compressed audio files.
///
/// The engine creates a `DiskReader` at startup. When a compressed file is
/// imported, it sends an `ActivateFile` command. The disk reader opens a
/// Symphonia decoder and starts filling the file's `ReadAheadBuffer` ahead
/// of the shared playhead.
pub struct DiskReader {
    /// Channel to send commands to the background thread.
    command_tx: rtrb::Producer<DiskReaderCommand>,
    /// Shared playhead position (frames). The engine updates this atomically.
    #[allow(dead_code)]
    playhead_frame: Arc<AtomicU64>,
    /// Whether the reader thread is running.
    running: Arc<AtomicBool>,
    /// Background thread handle.
    thread_handle: Option<std::thread::JoinHandle<()>>,
}

impl DiskReader {
    /// Create a new disk reader with a background thread.
    pub fn new(playhead_frame: Arc<AtomicU64>, _sample_rate: u32) -> Self {
        let (command_tx, command_rx) = rtrb::RingBuffer::new(64);
        let running = Arc::new(AtomicBool::new(true));

        let thread_running = running.clone();

        let thread_handle = std::thread::Builder::new()
            .name("disk-reader".into())
            .spawn(move || {
                Self::reader_thread(command_rx, thread_running);
            })
            .expect("Failed to spawn disk reader thread");

        Self {
            command_tx,
            playhead_frame,
            running,
            thread_handle: Some(thread_handle),
        }
    }

    /// Send a command to the disk reader thread.
    pub fn send(&mut self, cmd: DiskReaderCommand) {
        let _ = self.command_tx.push(cmd);
    }

    /// Create a `ReadAheadBuffer` for a compressed file.
    pub fn create_buffer(sample_rate: u32, channels: u32) -> Arc<ReadAheadBuffer> {
        Arc::new(ReadAheadBuffer::new(
            PREFETCH_SECONDS + 1.0, // extra headroom
            sample_rate,
            channels,
        ))
    }

    /// The disk reader background thread.
    fn reader_thread(
        mut command_rx: rtrb::Consumer<DiskReaderCommand>,
        running: Arc<AtomicBool>,
    ) {
        let mut active_files: HashMap<u64, (StreamSource, Arc<ReadAheadBuffer>)> =
            HashMap::new();
        let mut decode_buf = Vec::with_capacity(8192);

        while running.load(Ordering::Relaxed) {
            // Process commands.
            while let Ok(cmd) = command_rx.pop() {
                match cmd {
                    DiskReaderCommand::ActivateFile {
                        reader_id,
                        open,
                        kind,
                        buffer,
                    } => match StreamSource::open(open, kind) {
                        Ok(reader) => {
                            eprintln!("[DiskReader] Activated reader={}, kind={:?}, ch={}, sr={}",
                                reader_id, kind, reader.channels(), reader.sample_rate());
                            active_files.insert(reader_id, (reader, buffer));
                        }
                        Err(e) => {
                            eprintln!(
                                "[DiskReader] Failed to open reader={} ({:?}): {}",
                                reader_id, kind, e
                            );
                        }
                    },
                    DiskReaderCommand::DeactivateFile { reader_id } => {
                        active_files.remove(&reader_id);
                    }
                    DiskReaderCommand::Seek { frame } => {
                        for (_, (reader, buffer)) in active_files.iter_mut() {
                            buffer.force_target_frame(frame);
                            buffer.reset(frame);
                            if let Err(e) = reader.seek(frame) {
                                eprintln!("[DiskReader] Seek error: {}", e);
                            }
                        }
                    }
                    DiskReaderCommand::Shutdown => {
                        return;
                    }
                }
            }

            // Fill each active reader's buffer ahead of its target frame.
            // Each clip instance has its own buffer and target_frame, set by
            // render_from_file during the audio callback.
            for (_reader_id, (reader, buffer)) in active_files.iter_mut() {
                // Skip files where no clip is currently playing
                if !buffer.has_active_target() {
                    continue;
                }

                let target = buffer.target_frame();
                let buf_start = buffer.start_frame();
                let buf_valid = buffer.valid_frames_count();
                let buf_end = buf_start + buf_valid;

                // If the target has jumped behind or far ahead of the buffer,
                // seek the decoder and reset.
                if target < buf_start || target > buf_end + reader.sample_rate() as u64 {
                    buffer.reset(target);
                    let _ = reader.seek(target);
                    continue;
                }

                // Advance the buffer start to reclaim space behind the target.
                // Keep a small lookback for sinc interpolation (~32 frames).
                let lookback = 64u64;
                let advance_to = target.saturating_sub(lookback);
                if advance_to > buf_start {
                    buffer.advance_start(advance_to);
                }

                // Calculate how far ahead we need to fill.
                let buf_start = buffer.start_frame();
                let buf_valid = buffer.valid_frames_count();
                let buf_end = buf_start + buf_valid;
                let prefetch_target =
                    target + (PREFETCH_SECONDS * reader.sample_rate() as f64) as u64;

                if buf_end >= prefetch_target {
                    continue; // Already filled far enough ahead.
                }

                // Decode more data into the buffer.
                match reader.decode_next(&mut decode_buf) {
                    Ok(0) => {
                        // EOF: no more frames will be decoded for this buffer until a
                        // seek. Tell export-mode waiters so they stop blocking on
                        // past-the-end frames (e.g. video longer than its audio).
                        buffer.set_finished(true);
                    }
                    Ok(frames) => {
                        let was_empty = buffer.valid_frames_count() == 0;
                        buffer.write_samples(&decode_buf, frames);
                        if was_empty {
                            eprintln!("[DiskReader] reader={}: first fill, {} frames, buf_start={}, valid={}",
                                _reader_id, frames, buffer.start_frame(), buffer.valid_frames_count());
                        }
                    }
                    Err(e) => {
                        eprintln!("[DiskReader] Decode error: {}", e);
                    }
                }
            }

            // In export mode, skip the sleep so decoding runs at full speed.
            // Otherwise sleep briefly to avoid busy-spinning.
            let any_exporting = active_files.values().any(|(_, buf)| buf.is_export_mode());
            if !any_exporting {
                std::thread::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS));
            }
        }
    }
}

impl Drop for DiskReader {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
        let _ = self.command_tx.push(DiskReaderCommand::Shutdown);
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }
}

// Tests for VideoAudioReader live in `daw-backend/tests/video_audio_stream.rs`
// (integration tests) so they build the lib in normal mode, independent of
// pre-existing breakage in the crate's `#[cfg(test)]` unit tests (automation.rs).

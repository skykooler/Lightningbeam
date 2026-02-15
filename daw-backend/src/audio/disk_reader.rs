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

    /// Reset the buffer to start at `new_start` with zero valid frames.
    /// Called by the **disk reader thread** (producer) after a seek.
    pub fn reset(&self, new_start: u64) {
        self.valid_frames.store(0, Ordering::Release);
        self.start_frame.store(new_start, Ordering::Release);
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
struct CompressedReader {
    format_reader: Box<dyn symphonia::core::formats::FormatReader>,
    decoder: Box<dyn symphonia::core::codecs::Decoder>,
    track_id: u32,
    /// Current decoder position in frames.
    current_frame: u64,
    sample_rate: u32,
    channels: u32,
    #[allow(dead_code)]
    total_frames: u64,
    /// Temporary decode buffer.
    sample_buf: Option<SampleBuffer<f32>>,
}

impl CompressedReader {
    /// Open a compressed audio file and prepare for streaming decode.
    fn open(path: &Path) -> Result<Self, String> {
        let file =
            std::fs::File::open(path).map_err(|e| format!("Failed to open file: {}", e))?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());

        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }

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
            sample_rate,
            channels,
            total_frames,
            sample_buf: None,
        })
    }

    /// Seek to a specific frame. Returns the actual frame reached (may differ
    /// for compressed formats that can only seek to keyframes).
    fn seek(&mut self, target_frame: u64) -> Result<u64, String> {
        let seek_to = SeekTo::TimeStamp {
            ts: target_frame,
            track_id: self.track_id,
        };

        let seeked = self
            .format_reader
            .seek(SeekMode::Coarse, seek_to)
            .map_err(|e| format!("Seek failed: {}", e))?;

        let actual_frame = seeked.actual_ts;
        self.current_frame = actual_frame;

        // Reset the decoder after seeking.
        self.decoder.reset();

        Ok(actual_frame)
    }

    /// Decode the next chunk of audio into `out`. Returns the number of frames
    /// decoded. Returns `Ok(0)` at end-of-file.
    fn decode_next(&mut self, out: &mut Vec<f32>) -> Result<usize, String> {
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
                        out.extend_from_slice(samples);
                        let frames = samples.len() / self.channels as usize;
                        self.current_frame += frames as u64;
                        return Ok(frames);
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
// DiskReaderCommand
// ---------------------------------------------------------------------------

/// Commands sent from the engine to the disk reader thread.
pub enum DiskReaderCommand {
    /// Start streaming a compressed file for a clip instance.
    ActivateFile {
        reader_id: u64,
        path: PathBuf,
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
        let mut active_files: HashMap<u64, (CompressedReader, Arc<ReadAheadBuffer>)> =
            HashMap::new();
        let mut decode_buf = Vec::with_capacity(8192);

        while running.load(Ordering::Relaxed) {
            // Process commands.
            while let Ok(cmd) = command_rx.pop() {
                match cmd {
                    DiskReaderCommand::ActivateFile {
                        reader_id,
                        path,
                        buffer,
                    } => match CompressedReader::open(&path) {
                        Ok(reader) => {
                            eprintln!("[DiskReader] Activated reader={}, ch={}, sr={}, path={:?}",
                                reader_id, reader.channels, reader.sample_rate, path);
                            active_files.insert(reader_id, (reader, buffer));
                        }
                        Err(e) => {
                            eprintln!(
                                "[DiskReader] Failed to open compressed file {:?}: {}",
                                path, e
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
                if target < buf_start || target > buf_end + reader.sample_rate as u64 {
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
                    target + (PREFETCH_SECONDS * reader.sample_rate as f64) as u64;

                if buf_end >= prefetch_target {
                    continue; // Already filled far enough ahead.
                }

                // Decode more data into the buffer.
                match reader.decode_next(&mut decode_buf) {
                    Ok(0) => {} // EOF
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

            // Sleep briefly to avoid busy-spinning when all buffers are full.
            std::thread::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS));
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

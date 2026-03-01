//! Webcam capture and recording for Lightningbeam
//!
//! Cross-platform webcam capture using ffmpeg libavdevice:
//! - Linux: v4l2
//! - macOS: avfoundation
//! - Windows: dshow
//!
//! Capture runs on a dedicated thread. Frames are sent to the main thread
//! via a bounded channel for live preview. Recording encodes directly to
//! disk in real-time (H.264 or FFV1 lossless).

use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;

use ffmpeg_next as ffmpeg;

/// A camera device descriptor (platform-agnostic).
#[derive(Debug, Clone)]
pub struct CameraDevice {
    /// Human-readable name (e.g. "Integrated Webcam")
    pub name: String,
    /// ffmpeg input format name: "v4l2", "avfoundation", "dshow"
    pub format_name: String,
    /// Device path/identifier for ffmpeg: "/dev/video0", "0", "video=..."
    pub path: String,
}

/// A decoded RGBA frame from the webcam.
#[derive(Clone)]
pub struct CaptureFrame {
    pub rgba_data: Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    /// Seconds since capture started.
    pub timestamp: f64,
}

/// Codec to use when recording webcam footage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RecordingCodec {
    /// H.264 in MP4 — small files, lossy
    H264,
    /// FFV1 in MKV — lossless, larger files
    Lossless,
}

impl Default for RecordingCodec {
    fn default() -> Self {
        RecordingCodec::H264
    }
}

/// Command sent from the main thread to the capture thread.
enum CaptureCommand {
    StartRecording {
        path: PathBuf,
        codec: RecordingCodec,
        result_tx: mpsc::Sender<Result<(), String>>,
    },
    StopRecording {
        result_tx: mpsc::Sender<Result<RecordingResult, String>>,
    },
    Shutdown,
}

/// Result returned when recording stops.
pub struct RecordingResult {
    pub file_path: PathBuf,
    pub duration: f64,
}

/// Live webcam capture with optional recording.
///
/// Call `open()` to start capturing from a camera device. Use `poll_frame()`
/// each frame to get the latest preview. Use `start_recording()` /
/// `stop_recording()` to encode to disk.
pub struct WebcamCapture {
    cmd_tx: mpsc::Sender<CaptureCommand>,
    frame_rx: mpsc::Receiver<CaptureFrame>,
    latest_frame: Option<CaptureFrame>,
    pub width: u32,
    pub height: u32,
    recording: bool,
    thread_handle: Option<thread::JoinHandle<()>>,
}

impl WebcamCapture {
    /// Open a webcam device and start the capture thread.
    ///
    /// The camera is opened once on the capture thread (not on the calling
    /// thread) to avoid blocking the UI.  Resolution is reported back via a
    /// oneshot channel.
    pub fn open(device: &CameraDevice) -> Result<Self, String> {
        ffmpeg::init().map_err(|e| format!("ffmpeg init failed: {e}"))?;
        ffmpeg::device::register_all();

        let (cmd_tx, cmd_rx) = mpsc::channel::<CaptureCommand>();
        let (frame_tx, frame_rx) = mpsc::sync_channel::<CaptureFrame>(2);
        // Oneshot for the capture thread to report back resolution
        let (info_tx, info_rx) = mpsc::channel::<Result<(u32, u32), String>>();

        let device_clone = device.clone();
        let thread_handle = thread::Builder::new()
            .name("webcam-capture".into())
            .spawn(move || {
                if let Err(e) = capture_thread_main(&device_clone, frame_tx, cmd_rx, info_tx) {
                    eprintln!("[webcam] capture thread error: {e}");
                }
            })
            .map_err(|e| format!("Failed to spawn capture thread: {e}"))?;

        // Wait for the capture thread to open the camera and report resolution
        let (width, height) = info_rx
            .recv()
            .map_err(|_| "Capture thread died during init".to_string())?
            .map_err(|e| format!("Camera open failed: {e}"))?;

        Ok(Self {
            cmd_tx,
            frame_rx,
            latest_frame: None,
            width,
            height,
            recording: false,
            thread_handle: Some(thread_handle),
        })
    }

    /// Drain the frame channel and return the most recent frame.
    pub fn poll_frame(&mut self) -> Option<&CaptureFrame> {
        while let Ok(frame) = self.frame_rx.try_recv() {
            self.latest_frame = Some(frame);
        }
        self.latest_frame.as_ref()
    }

    /// Start recording to disk.
    pub fn start_recording(&mut self, path: PathBuf, codec: RecordingCodec) -> Result<(), String> {
        if self.recording {
            return Err("Already recording".into());
        }
        let (result_tx, result_rx) = mpsc::channel();
        self.cmd_tx
            .send(CaptureCommand::StartRecording {
                path,
                codec,
                result_tx,
            })
            .map_err(|_| "Capture thread not running")?;

        let result = result_rx
            .recv()
            .map_err(|_| "Capture thread died before responding")?;
        result?;
        self.recording = true;
        Ok(())
    }

    /// Stop recording and return the result.
    pub fn stop_recording(&mut self) -> Result<RecordingResult, String> {
        if !self.recording {
            return Err("Not recording".into());
        }
        let (result_tx, result_rx) = mpsc::channel();
        self.cmd_tx
            .send(CaptureCommand::StopRecording { result_tx })
            .map_err(|_| "Capture thread not running")?;

        let result = result_rx
            .recv()
            .map_err(|_| "Capture thread died before responding")?;
        self.recording = false;
        result
    }

    pub fn is_recording(&self) -> bool {
        self.recording
    }
}

impl Drop for WebcamCapture {
    fn drop(&mut self) {
        let _ = self.cmd_tx.send(CaptureCommand::Shutdown);
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }
}

// ---------------------------------------------------------------------------
// Platform camera enumeration
// ---------------------------------------------------------------------------

/// Find the ffmpeg input format by name (e.g. "v4l2", "avfoundation", "dshow").
///
/// Uses the FFI `av_find_input_format()` directly, since `ffmpeg::device::input::video()`
/// only iterates device-registered formats and may miss demuxers like v4l2.
fn find_input_format(format_name: &str) -> Option<ffmpeg::format::format::Input> {
    // Log what the device iterator sees (for diagnostics)
    let device_formats: Vec<String> = ffmpeg::device::input::video()
        .map(|f| f.name().to_string())
        .collect();
    eprintln!("[WEBCAM] Registered device input formats: {:?}", device_formats);

    let c_name = std::ffi::CString::new(format_name).ok()?;
    unsafe {
        let ptr = ffmpeg::sys::av_find_input_format(c_name.as_ptr());
        if ptr.is_null() {
            eprintln!("[WEBCAM] av_find_input_format('{}') returned null", format_name);
            None
        } else {
            eprintln!("[WEBCAM] av_find_input_format('{}') found format", format_name);
            Some(ffmpeg::format::format::Input::wrap(ptr as *mut _))
        }
    }
}

/// List available camera devices for the current platform.
pub fn list_cameras() -> Vec<CameraDevice> {
    ffmpeg::init().ok();
    ffmpeg::device::register_all();

    let mut devices = Vec::new();

    #[cfg(target_os = "linux")]
    {
        for i in 0..10 {
            let path = format!("/dev/video{i}");
            if std::path::Path::new(&path).exists() {
                devices.push(CameraDevice {
                    name: format!("Camera {i}"),
                    format_name: "v4l2".into(),
                    path,
                });
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        devices.push(CameraDevice {
            name: "Default Camera".into(),
            format_name: "avfoundation".into(),
            path: "0".into(),
        });
    }

    #[cfg(target_os = "windows")]
    {
        devices.push(CameraDevice {
            name: "Default Camera".into(),
            format_name: "dshow".into(),
            path: "video=Integrated Camera".into(),
        });
    }

    devices
}

/// Return the first available camera, if any.
pub fn default_camera() -> Option<CameraDevice> {
    list_cameras().into_iter().next()
}

// ---------------------------------------------------------------------------
// Opening a camera device
// ---------------------------------------------------------------------------

/// Open a camera device via ffmpeg, returning an Input context.
///
/// Requests 640x480 @ 30fps — universally supported by USB webcams and
/// achievable over USB 2.0.  The driver may negotiate different values;
/// the capture thread reads whatever the driver actually provides.
fn open_camera(device: &CameraDevice) -> Result<ffmpeg::format::context::Input, String> {
    let input_format = find_input_format(&device.format_name)
        .ok_or_else(|| format!("Input format '{}' not found — is libavdevice enabled?", device.format_name))?;

    let mut opts = ffmpeg::Dictionary::new();
    opts.set("video_size", "640x480");
    opts.set("framerate", "30");

    let format = ffmpeg::Format::Input(input_format);
    let ctx = ffmpeg::format::open_with(&device.path, &format, opts)
        .map_err(|e| format!("Failed to open camera '{}': {e}", device.path))?;

    if ctx.is_input() {
        Ok(ctx.input())
    } else {
        Err("Expected input context from camera".into())
    }
}

// ---------------------------------------------------------------------------
// Capture thread
// ---------------------------------------------------------------------------

fn capture_thread_main(
    device: &CameraDevice,
    frame_tx: mpsc::SyncSender<CaptureFrame>,
    cmd_rx: mpsc::Receiver<CaptureCommand>,
    info_tx: mpsc::Sender<Result<(u32, u32), String>>,
) -> Result<(), String> {
    let mut input = match open_camera(device) {
        Ok(input) => input,
        Err(e) => {
            let _ = info_tx.send(Err(e.clone()));
            return Err(e);
        }
    };

    let stream_index = input
        .streams()
        .best(ffmpeg::media::Type::Video)
        .ok_or("No video stream")?
        .index();

    let stream = input.stream(stream_index).unwrap();
    let fps = {
        let r = f64::from(stream.avg_frame_rate());
        if r > 0.0 { r } else { 30.0 }
    };
    let codec_params = stream.parameters();
    let decoder_ctx = ffmpeg::codec::context::Context::from_parameters(codec_params)
        .map_err(|e| format!("Codec context: {e}"))?;
    let mut decoder = decoder_ctx
        .decoder()
        .video()
        .map_err(|e| format!("Video decoder: {e}"))?;

    let width = decoder.width();
    let height = decoder.height();
    let src_format = decoder.format();

    eprintln!("[webcam] Camera opened: {}x{} @ {:.1}fps format={:?}",
             width, height, fps, src_format);

    // Report resolution back to the main thread
    let _ = info_tx.send(Ok((width, height)));

    let mut scaler = ffmpeg::software::scaling::Context::get(
        src_format,
        width,
        height,
        ffmpeg::format::Pixel::RGBA,
        width,
        height,
        ffmpeg::software::scaling::Flags::BILINEAR,
    )
    .map_err(|e| format!("Scaler init: {e}"))?;

    let mut recorder: Option<FrameRecorder> = None;
    let start_time = std::time::Instant::now();
    let mut frame_count: u64 = 0;
    /// Number of initial frames to skip (v4l2 first buffers are often corrupt)
    const SKIP_INITIAL_FRAMES: u64 = 2;

    let mut decoded_frame = ffmpeg::frame::Video::empty();
    let mut rgba_frame = ffmpeg::frame::Video::empty();

    'outer: for (stream_ref, packet) in input.packets() {
        if stream_ref.index() != stream_index {
            continue;
        }

        // Check for commands (non-blocking).
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                CaptureCommand::StartRecording {
                    path,
                    codec,
                    result_tx,
                } => {
                    let result = FrameRecorder::new(&path, codec, width, height, fps);
                    match result {
                        Ok(rec) => {
                            recorder = Some(rec);
                            let _ = result_tx.send(Ok(()));
                        }
                        Err(e) => {
                            let _ = result_tx.send(Err(e));
                        }
                    }
                }
                CaptureCommand::StopRecording { result_tx } => {
                    if let Some(rec) = recorder.take() {
                        let _ = result_tx.send(rec.finish());
                    } else {
                        let _ = result_tx.send(Err("Not recording".into()));
                    }
                }
                CaptureCommand::Shutdown => break 'outer,
            }
        }

        decoder.send_packet(&packet).ok();

        while decoder.receive_frame(&mut decoded_frame).is_ok() {
            // Skip initial corrupt frames from v4l2
            if frame_count < SKIP_INITIAL_FRAMES {
                frame_count += 1;
                continue;
            }

            scaler.run(&decoded_frame, &mut rgba_frame).ok();

            let timestamp = start_time.elapsed().as_secs_f64();

            // Build tightly-packed RGBA data (remove stride padding).
            let data = rgba_frame.data(0);
            let stride = rgba_frame.stride(0);
            let row_bytes = (width * 4) as usize;

            let rgba_data = if stride == row_bytes {
                data[..row_bytes * height as usize].to_vec()
            } else {
                let mut buf = Vec::with_capacity(row_bytes * height as usize);
                for y in 0..height as usize {
                    buf.extend_from_slice(&data[y * stride..y * stride + row_bytes]);
                }
                buf
            };

            let rgba_arc = Arc::new(rgba_data);

            let frame = CaptureFrame {
                rgba_data: rgba_arc.clone(),
                width,
                height,
                timestamp,
            };
            let _ = frame_tx.try_send(frame);

            if let Some(ref mut rec) = recorder {
                if let Err(e) = rec.encode_rgba(&rgba_arc, width, height, frame_count) {
                    eprintln!("[webcam] recording encode error: {e}");
                }
            }

            frame_count += 1;
        }
    }

    // Clean up: if still recording when shutting down, finalize.
    if let Some(rec) = recorder.take() {
        let _ = rec.finish();
    }

    decoder.send_eof().ok();

    Ok(())
}

// ---------------------------------------------------------------------------
// Recording encoder
// ---------------------------------------------------------------------------

struct FrameRecorder {
    output: ffmpeg::format::context::Output,
    encoder: ffmpeg::encoder::Video,
    scaler: ffmpeg::software::scaling::Context,
    path: PathBuf,
    frame_count: u64,
    fps: f64,
}

impl FrameRecorder {
    fn new(
        path: &PathBuf,
        codec: RecordingCodec,
        width: u32,
        height: u32,
        fps: f64,
    ) -> Result<Self, String> {
        let path_str = path.to_str().ok_or("Invalid path")?;

        let mut output = ffmpeg::format::output(path_str)
            .map_err(|e| format!("Failed to create output file: {e}"))?;

        let (codec_id, pixel_format) = match codec {
            RecordingCodec::H264 => (ffmpeg::codec::Id::H264, ffmpeg::format::Pixel::YUV420P),
            RecordingCodec::Lossless => (ffmpeg::codec::Id::FFV1, ffmpeg::format::Pixel::YUV444P),
        };

        let ffmpeg_codec = ffmpeg::encoder::find(codec_id)
            .or_else(|| match codec_id {
                ffmpeg::codec::Id::H264 => ffmpeg::encoder::find_by_name("libx264"),
                ffmpeg::codec::Id::FFV1 => ffmpeg::encoder::find_by_name("ffv1"),
                _ => None,
            })
            .ok_or_else(|| format!("Encoder not found for {codec_id:?}"))?;

        let mut encoder = ffmpeg::codec::Context::new_with_codec(ffmpeg_codec)
            .encoder()
            .video()
            .map_err(|e| format!("Failed to create encoder: {e}"))?;

        let aligned_width = if codec_id == ffmpeg::codec::Id::H264 {
            ((width + 15) / 16) * 16
        } else {
            width
        };
        let aligned_height = if codec_id == ffmpeg::codec::Id::H264 {
            ((height + 15) / 16) * 16
        } else {
            height
        };

        encoder.set_width(aligned_width);
        encoder.set_height(aligned_height);
        encoder.set_format(pixel_format);
        encoder.set_time_base(ffmpeg::Rational(1, fps as i32));
        encoder.set_frame_rate(Some(ffmpeg::Rational(fps as i32, 1)));

        if codec_id == ffmpeg::codec::Id::H264 {
            encoder.set_bit_rate(4_000_000);
            encoder.set_gop(fps as u32);
        }

        let encoder = encoder
            .open_as(ffmpeg_codec)
            .map_err(|e| format!("Failed to open encoder: {e}"))?;

        let mut stream = output
            .add_stream(ffmpeg_codec)
            .map_err(|e| format!("Failed to add stream: {e}"))?;
        stream.set_parameters(&encoder);

        output
            .write_header()
            .map_err(|e| format!("Failed to write header: {e}"))?;

        let scaler = ffmpeg::software::scaling::Context::get(
            ffmpeg::format::Pixel::RGBA,
            width,
            height,
            pixel_format,
            aligned_width,
            aligned_height,
            ffmpeg::software::scaling::Flags::BILINEAR,
        )
        .map_err(|e| format!("Scaler init: {e}"))?;

        Ok(Self {
            output,
            encoder,
            scaler,
            path: path.clone(),
            frame_count: 0,
            fps,
        })
    }

    fn encode_rgba(
        &mut self,
        rgba_data: &[u8],
        width: u32,
        height: u32,
        _global_frame: u64,
    ) -> Result<(), String> {
        let mut src_frame =
            ffmpeg::frame::Video::new(ffmpeg::format::Pixel::RGBA, width, height);

        let dst_stride = src_frame.stride(0);
        let row_bytes = (width * 4) as usize;
        for y in 0..height as usize {
            let src_offset = y * row_bytes;
            let dst_offset = y * dst_stride;
            src_frame.data_mut(0)[dst_offset..dst_offset + row_bytes]
                .copy_from_slice(&rgba_data[src_offset..src_offset + row_bytes]);
        }

        let mut dst_frame = ffmpeg::frame::Video::empty();
        self.scaler
            .run(&src_frame, &mut dst_frame)
            .map_err(|e| format!("Scale: {e}"))?;

        dst_frame.set_pts(Some(self.frame_count as i64));
        self.frame_count += 1;

        self.encoder
            .send_frame(&dst_frame)
            .map_err(|e| format!("Send frame: {e}"))?;

        self.receive_packets()?;
        Ok(())
    }

    fn receive_packets(&mut self) -> Result<(), String> {
        let mut packet = ffmpeg::Packet::empty();
        let encoder_tb = self.encoder.time_base();
        let stream_tb = self
            .output
            .stream(0)
            .ok_or("No output stream")?
            .time_base();

        while self.encoder.receive_packet(&mut packet).is_ok() {
            packet.set_stream(0);
            packet.rescale_ts(encoder_tb, stream_tb);
            packet
                .write_interleaved(&mut self.output)
                .map_err(|e| format!("Write packet: {e}"))?;
        }
        Ok(())
    }

    fn finish(mut self) -> Result<RecordingResult, String> {
        self.encoder
            .send_eof()
            .map_err(|e| format!("Send EOF: {e}"))?;
        self.receive_packets()?;

        self.output
            .write_trailer()
            .map_err(|e| format!("Write trailer: {e}"))?;

        let duration = self.frame_count as f64 / self.fps;
        Ok(RecordingResult {
            file_path: self.path,
            duration,
        })
    }
}

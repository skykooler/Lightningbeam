//! Stream a demuxer from an arbitrary `Read + Seek` byte source via a custom
//! FFmpeg `AVIOContext`.
//!
//! `ffmpeg-next`'s high-level API can only open inputs by filesystem path. This
//! crate builds an `AVFormatContext` whose I/O is backed by a Rust reader (e.g. a
//! SQLite `BlobReader`), then hands back a normal [`ffmpeg_next::format::context::Input`]
//! that the rest of the codebase decodes exactly as a file-backed input.
//!
//! All of the `unsafe` FFI and ffmpeg ABI coupling lives here, isolated behind the
//! safe [`BlobInput`] type. See `BEAM_FILE_FORMAT.md` / `STREAMING_TO_DISK_PLAN.md`.
//!
//! # Safety model
//! - The reader is boxed and leaked to FFmpeg as the AVIO `opaque` pointer; the
//!   read/seek callbacks reconstitute it as `&mut`. It is never aliased — only the
//!   owning [`BlobInput`] (on one thread) drives it.
//! - [`BlobInput`] is `Send` but **not** `Sync`: the typical reader (`BlobReader`)
//!   owns a `!Sync` SQLite connection. Each decoder / seek-reopen / off-thread scan
//!   must construct its **own** `BlobInput` from a **fresh** reader.
//! - `Drop` tears down in the one correct order (Input first, then the AVIO buffer +
//!   context, then the reader) so there is no use-after-free, double-free, or leak.

use std::io::{Read, Seek, SeekFrom};
use std::os::raw::{c_char, c_int, c_void};
use std::ptr;

use ffmpeg_next::format::context::Input;
use ffmpeg_sys_next as sys;

/// A byte source FFmpeg can stream from. Blanket-implemented for any
/// `Read + Seek + Send` (e.g. `std::io::Cursor`, a SQLite `BlobReader`).
pub trait BlobSource: Read + Seek + Send {}
impl<T: Read + Seek + Send> BlobSource for T {}

// Stable FFmpeg ABI constants (avoid depending on whether bindgen exported the
// corresponding `#define`s). These are fixed across the libavformat 58–61 ABIs.
const AVSEEK_SIZE: c_int = 0x10000;
const AVSEEK_FORCE: c_int = 0x20000;
const AVFMT_FLAG_CUSTOM_IO: c_int = 0x0080;

/// Size of the AVIO read buffer handed to FFmpeg. FFmpeg may grow/replace it.
const IO_BUFFER_SIZE: usize = 32 * 1024;

/// A demuxer input streaming from a boxed `Read + Seek` source.
///
/// Deref to [`Input`] for the usual ffmpeg-next decode API.
pub struct BlobInput {
    // Dropped FIRST in `Drop` (avformat_close_input). `Option` so we can drop it
    // explicitly before freeing the AVIO resources it points at.
    input: Option<Input>,
    // The custom AVIOContext. With AVFMT_FLAG_CUSTOM_IO set, avformat_close_input
    // does NOT free this — we own it. Its `opaque` field is the boxed reader.
    avio: *mut sys::AVIOContext,
}

// The reader is `Send` and never aliased; we manage its lifetime manually.
unsafe impl Send for BlobInput {}
// Intentionally NOT `Sync`: see the module-level safety notes.

impl BlobInput {
    /// Open a demuxer over `src`.
    ///
    /// `format_hint` is an optional container short-name / extension (e.g. `"mp4"`,
    /// `"mov"`, `"matroska"`) passed to `av_find_input_format` to skip probe
    /// ambiguity; `None` lets FFmpeg probe from the stream.
    pub fn open(src: Box<dyn BlobSource>, format_hint: Option<&str>) -> Result<Self, String> {
        ffmpeg_next::init().map_err(|e| format!("ffmpeg init failed: {e}"))?;

        unsafe {
            // 1. IO buffer, allocated with FFmpeg's allocator (freed with av_freep).
            let buffer = sys::av_malloc(IO_BUFFER_SIZE) as *mut u8;
            if buffer.is_null() {
                return Err("av_malloc failed for AVIO buffer".into());
            }

            // 2. Box the reader and leak it as the opaque pointer. Double-box so the
            //    raw pointer is thin (a `*mut dyn` would be a fat pointer).
            let opaque = Box::into_raw(Box::new(src)) as *mut c_void;

            // 3. AVIOContext over our callbacks (read-only: write_flag = 0).
            let avio = sys::avio_alloc_context(
                buffer,
                IO_BUFFER_SIZE as c_int,
                0,
                opaque,
                Some(read_cb),
                None,
                Some(seek_cb),
            );
            if avio.is_null() {
                sys::av_free(buffer as *mut c_void);
                drop(Box::from_raw(opaque as *mut Box<dyn BlobSource>));
                return Err("avio_alloc_context failed".into());
            }

            // 4. Format context wired to the custom IO.
            let mut fmt = sys::avformat_alloc_context();
            if fmt.is_null() {
                destroy_io(avio);
                return Err("avformat_alloc_context failed".into());
            }
            (*fmt).pb = avio;
            (*fmt).flags |= AVFMT_FLAG_CUSTOM_IO;

            // 5. Optional format hint.
            let hint_cstr = format_hint.and_then(|s| std::ffi::CString::new(s).ok());
            let infmt: *const sys::AVInputFormat = match &hint_cstr {
                Some(c) => sys::av_find_input_format(c.as_ptr() as *const c_char),
                None => ptr::null(),
            };

            // 6. Open. On failure avformat_open_input frees `fmt` itself (and, with
            //    CUSTOM_IO, leaves our pb), so we still own avio+buffer+reader.
            let ret = sys::avformat_open_input(&mut fmt, ptr::null(), infmt, ptr::null_mut());
            if ret < 0 {
                destroy_io(avio);
                return Err(format!("avformat_open_input failed (error {ret})"));
            }

            // 7. Probe streams. On failure close the now-open input, then free IO.
            let ret = sys::avformat_find_stream_info(fmt, ptr::null_mut());
            if ret < 0 {
                sys::avformat_close_input(&mut fmt);
                destroy_io(avio);
                return Err(format!("avformat_find_stream_info failed (error {ret})"));
            }

            // 8. Hand ownership of `fmt` to ffmpeg-next's Input (closes on drop).
            let input = Input::wrap(fmt);
            Ok(BlobInput { input: Some(input), avio })
        }
    }

    /// The underlying demuxer input.
    pub fn input(&self) -> &Input {
        self.input.as_ref().expect("BlobInput input present until drop")
    }

    /// The underlying demuxer input, mutably (for `seek`, `packets`, …).
    pub fn input_mut(&mut self) -> &mut Input {
        self.input.as_mut().expect("BlobInput input present until drop")
    }
}

impl std::ops::Deref for BlobInput {
    type Target = Input;
    fn deref(&self) -> &Input {
        self.input()
    }
}
impl std::ops::DerefMut for BlobInput {
    fn deref_mut(&mut self) -> &mut Input {
        self.input_mut()
    }
}

impl Drop for BlobInput {
    fn drop(&mut self) {
        unsafe {
            // 1. Drop the Input first: avformat_close_input. With CUSTOM_IO this does
            //    NOT touch self.avio or its buffer.
            self.input = None;
            // 2..4. Free the AVIO buffer + context and reclaim the boxed reader.
            if !self.avio.is_null() {
                destroy_io(self.avio);
                self.avio = ptr::null_mut();
            }
        }
    }
}

/// Free a standalone custom AVIOContext: its (possibly reallocated) buffer, the
/// boxed reader behind `opaque`, then the context itself — in that order. Only
/// call when the owning AVFormatContext has already been closed (or never opened).
unsafe fn destroy_io(avio: *mut sys::AVIOContext) {
    if avio.is_null() {
        return;
    }
    // Reclaim the reader box *before* freeing the context (we need `opaque`).
    let opaque = (*avio).opaque;
    // Free the current IO buffer (FFmpeg may have replaced the original).
    sys::av_freep(&mut (*avio).buffer as *mut _ as *mut c_void);
    // Free the AVIOContext struct (nulls the local).
    let mut avio = avio;
    sys::avio_context_free(&mut avio);
    // Drop the reader last — no callback can fire now.
    if !opaque.is_null() {
        drop(Box::from_raw(opaque as *mut Box<dyn BlobSource>));
    }
}

/// FFmpeg read callback: fill `buf` from the Rust reader. Returns bytes read,
/// `AVERROR_EOF` at end of stream, or a negative error.
unsafe extern "C" fn read_cb(opaque: *mut c_void, buf: *mut u8, buf_size: c_int) -> c_int {
    if opaque.is_null() || buf.is_null() || buf_size <= 0 {
        return sys::AVERROR_EOF;
    }
    let reader = &mut *(opaque as *mut Box<dyn BlobSource>);
    let slice = std::slice::from_raw_parts_mut(buf, buf_size as usize);
    match reader.read(slice) {
        Ok(0) => sys::AVERROR_EOF,
        Ok(n) => n as c_int,
        // AVERROR(EIO) == -EIO on Unix; a negative value signals a read error.
        Err(_) => -(libc::EIO as c_int),
    }
}

/// FFmpeg seek callback. Handles `SEEK_SET/CUR/END` and `AVSEEK_SIZE` (report total
/// length). Returns the new position, the size, or `-1` on error.
unsafe extern "C" fn seek_cb(opaque: *mut c_void, offset: i64, whence: c_int) -> i64 {
    if opaque.is_null() {
        return -1;
    }
    let reader = &mut *(opaque as *mut Box<dyn BlobSource>);
    let whence = whence & !AVSEEK_FORCE;
    if whence & AVSEEK_SIZE != 0 {
        // Report total length WITHOUT moving the logical position. FFmpeg does not
        // restore the position after an AVSEEK_SIZE query, so we must: measure by
        // seeking to the end, then seek back. (Failing to restore corrupts reads of
        // containers whose index lives at the end of the file, e.g. MP4 `moov`.)
        let cur = match reader.stream_position() {
            Ok(p) => p,
            Err(_) => return -1,
        };
        let size = match reader.seek(SeekFrom::End(0)) {
            Ok(s) => s,
            Err(_) => return -1,
        };
        if reader.seek(SeekFrom::Start(cur)).is_err() {
            return -1;
        }
        return size as i64;
    }
    let from = match whence {
        libc::SEEK_SET => SeekFrom::Start(offset as u64),
        libc::SEEK_CUR => SeekFrom::Current(offset),
        libc::SEEK_END => SeekFrom::End(offset),
        _ => return -1,
    };
    reader.seek(from).map(|n| n as i64).unwrap_or(-1)
}

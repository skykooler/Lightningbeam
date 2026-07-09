//! Animated GIF encoding.
//!
//! Palette-quantizes a stream of RGBA8 frames and writes them to a `.gif`. The expensive part —
//! per-frame NeuQuant 256-color quantization — is embarrassingly parallel (each frame gets its own
//! local palette), so it's fanned out across a worker pool. A single writer thread collects the
//! quantized frames, reorders them, and LZW-encodes them to the file in sequence.
//!
//! Pipeline (all off the UI thread):
//! ```text
//!   UI render thread ──RGBA──▶ coordinator ──round-robin──▶ N quantizer workers
//!                                                                   │ (idx, gif::Frame)
//!                                                                   ▼
//!                                                             writer thread ──▶ .gif
//! ```
//! Rendering + readback happen on the UI thread (see `render_next_gif_frame`); this module owns
//! everything after a raw RGBA frame arrives.

use lightningbeam_core::export::ExportProgress;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

/// Message from the UI (render) thread to the GIF encoder coordinator.
pub enum GifFrameMessage {
    /// One RGBA8 frame (top-left origin, tightly packed `width*height*4` bytes).
    Frame { frame_num: usize, pixels: Vec<u8> },
    /// All frames have been sent.
    Done,
}

/// gif crate quantization speed (1 = slowest/best, 30 = fastest/worst). 10 balances palette quality
/// against per-frame cost; the parallelism below is what actually recovers the wall-clock.
const QUANT_SPEED: i32 = 10;

/// Run the GIF encoder pipeline. Receives RGBA8 frames from `frame_rx`, quantizes them in parallel,
/// and writes the ordered result to `output_path`, reporting progress. `transparency == false`
/// composites each frame onto opaque black first (GIF's 1-bit transparency would otherwise key out
/// semi-transparent pixels).
#[allow(clippy::too_many_arguments)]
pub fn run_gif_encoder(
    frame_rx: Receiver<GifFrameMessage>,
    output_path: PathBuf,
    width: u32,
    height: u32,
    total_frames: usize,
    delay_ms: u32,
    loop_forever: bool,
    transparency: bool,
    progress_tx: Sender<ExportProgress>,
    cancel_flag: Arc<AtomicBool>,
) {
    let _ = progress_tx.send(ExportProgress::Started { total_frames });

    let delay_cs = ((delay_ms / 10).max(1)) as u16;
    let expected_len = (width as usize) * (height as usize) * 4;

    // One quantizer worker per spare core (leave one for the UI render thread), capped so we don't
    // spawn absurdly many for short exports.
    let n_workers = std::thread::available_parallelism()
        .map(|n| n.get().saturating_sub(1))
        .unwrap_or(1)
        .clamp(1, 8);

    // Per-worker input channels (coordinator dispatches round-robin) + one shared result channel.
    let mut worker_txs: Vec<Sender<(usize, Vec<u8>)>> = Vec::with_capacity(n_workers);
    let (result_tx, result_rx) = channel::<(usize, gif::Frame<'static>)>();
    let mut worker_handles = Vec::with_capacity(n_workers);

    for _ in 0..n_workers {
        let (wtx, wrx) = channel::<(usize, Vec<u8>)>();
        worker_txs.push(wtx);
        let result_tx = result_tx.clone();
        let cancel = Arc::clone(&cancel_flag);
        worker_handles.push(std::thread::spawn(move || {
            while let Ok((idx, mut pixels)) = wrx.recv() {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                // NeuQuant local-palette quantization (the expensive step). `from_rgba_speed` uses
                // the RGBA buffer as scratch, so it's fine that we own `pixels` here.
                let mut frame =
                    gif::Frame::from_rgba_speed(width as u16, height as u16, &mut pixels, QUANT_SPEED);
                frame.delay = delay_cs;
                if result_tx.send((idx, frame)).is_err() {
                    break; // writer gone
                }
            }
        }));
    }
    drop(result_tx); // only the workers hold senders now; writer's rx ends when they all finish

    // Writer thread: order frames by index and LZW-encode them sequentially.
    let writer_progress = progress_tx.clone();
    let writer_cancel = Arc::clone(&cancel_flag);
    let writer_output = output_path.clone();
    let writer = std::thread::spawn(move || -> Result<(), String> {
        let file = std::fs::File::create(&writer_output)
            .map_err(|e| format!("Failed to create GIF file: {e}"))?;
        let mut buf = std::io::BufWriter::new(file);
        let mut encoder = gif::Encoder::new(&mut buf, width as u16, height as u16, &[])
            .map_err(|e| format!("GIF encoder init failed: {e}"))?;
        if loop_forever {
            encoder
                .set_repeat(gif::Repeat::Infinite)
                .map_err(|e| format!("GIF set_repeat failed: {e}"))?;
        }

        // Frames may arrive out of order; hold stragglers until their turn.
        let mut pending: HashMap<usize, gif::Frame<'static>> = HashMap::new();
        let mut next = 0usize;
        let mut written = 0usize;

        while let Ok((idx, frame)) = result_rx.recv() {
            if writer_cancel.load(Ordering::Relaxed) {
                break;
            }
            pending.insert(idx, frame);
            while let Some(f) = pending.remove(&next) {
                encoder
                    .write_frame(&f)
                    .map_err(|e| format!("GIF write_frame failed: {e}"))?;
                next += 1;
                written += 1;
                let _ = writer_progress.send(ExportProgress::FrameRendered {
                    frame: written,
                    total: total_frames,
                });
            }
        }
        // Encoder/BufWriter flush on drop.
        Ok(())
    });

    // Coordinator: pull RGBA frames from the UI thread and dispatch round-robin to the workers.
    let mut dispatched = 0usize;
    let mut fatal: Option<String> = None;
    loop {
        match frame_rx.recv() {
            Ok(GifFrameMessage::Frame { frame_num, mut pixels }) => {
                if cancel_flag.load(Ordering::Relaxed) {
                    break;
                }
                if pixels.len() != expected_len {
                    fatal = Some("GIF frame size mismatch".into());
                    break;
                }
                if !transparency {
                    // Premultiply onto opaque black, then force alpha opaque.
                    for px in pixels.chunks_exact_mut(4) {
                        let a = px[3] as u32;
                        px[0] = (px[0] as u32 * a / 255) as u8;
                        px[1] = (px[1] as u32 * a / 255) as u8;
                        px[2] = (px[2] as u32 * a / 255) as u8;
                        px[3] = 255;
                    }
                }
                let w = dispatched % n_workers;
                if worker_txs[w].send((frame_num, pixels)).is_err() {
                    fatal = Some("GIF quantizer worker died".into());
                    break;
                }
                dispatched += 1;
            }
            Ok(GifFrameMessage::Done) => break,
            Err(_) => break, // UI thread dropped the sender
        }
    }

    let _ = progress_tx.send(ExportProgress::Finalizing);

    // Close worker inputs → workers finish → their result senders drop → writer's loop ends.
    drop(worker_txs);
    for h in worker_handles {
        let _ = h.join();
    }
    let writer_result = writer.join().unwrap_or_else(|_| Err("GIF writer thread panicked".into()));

    if cancel_flag.load(Ordering::Relaxed) {
        std::fs::remove_file(&output_path).ok();
        // Emit Complete so the UI poll loop clears its state; the dialog was closed on cancel.
        let _ = progress_tx.send(ExportProgress::Complete { output_path });
        return;
    }

    match fatal.or_else(|| writer_result.err()) {
        Some(message) => {
            std::fs::remove_file(&output_path).ok();
            let _ = progress_tx.send(ExportProgress::Error { message });
        }
        None => {
            let _ = progress_tx.send(ExportProgress::Complete { output_path });
        }
    }
}

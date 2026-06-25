//! F3 Debug Overlay
//!
//! Displays performance metrics and system info similar to Minecraft's F3 screen.
//! Press F3 to toggle visibility.

use eframe::egui;
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const FRAME_HISTORY_SIZE: usize = 60; // Track last 60 frames for FPS stats

/// Timing breakdown for the GPU prepare() pass, written by the render thread.
#[derive(Debug, Clone, Default)]
pub struct PrepareTiming {
    pub total_ms: f64,
    pub removals_ms: f64,
    pub gpu_dispatches_ms: f64,
    pub scene_build_ms: f64,
    pub composite_ms: f64,
}

static LAST_PREPARE_TIMING: OnceLock<Mutex<PrepareTiming>> = OnceLock::new();

/// Called from `VelloCallback::prepare()` every frame to update the timing snapshot.
pub fn update_prepare_timing(
    total_ms: f64,
    removals_ms: f64,
    gpu_dispatches_ms: f64,
    scene_build_ms: f64,
    composite_ms: f64,
) {
    let cell = LAST_PREPARE_TIMING.get_or_init(|| Mutex::new(PrepareTiming::default()));
    if let Ok(mut t) = cell.lock() {
        t.total_ms         = total_ms;
        t.removals_ms      = removals_ms;
        t.gpu_dispatches_ms = gpu_dispatches_ms;
        t.scene_build_ms   = scene_build_ms;
        t.composite_ms     = composite_ms;
    }
}
/// GPU-measured composite cost (from timestamp queries; see `gpu_timer.rs`).
#[derive(Debug, Clone, Default)]
pub struct GpuCompositeTiming {
    /// True when the adapter supports timestamp queries (else the ms is meaningless).
    pub supported: bool,
    /// GPU time of the whole composite section (Vello render + sRGB→linear +
    /// compositor + tonemap), in milliseconds. Read back asynchronously, so it
    /// lags the displayed frame by a frame or two.
    pub composite_gpu_ms: f64,
    /// Layers composited this frame.
    pub layers: u32,
    /// `queue.submit()` calls in the composite section this frame.
    pub submits: u32,
}

static GPU_COMPOSITE: OnceLock<Mutex<GpuCompositeTiming>> = OnceLock::new();

/// Called from `VelloCallback::prepare()` with the GPU composite measurement.
pub fn update_gpu_composite(supported: bool, composite_gpu_ms: f64, layers: u32, submits: u32) {
    let cell = GPU_COMPOSITE.get_or_init(|| Mutex::new(GpuCompositeTiming::default()));
    if let Ok(mut t) = cell.lock() {
        t.supported = supported;
        t.composite_gpu_ms = composite_gpu_ms;
        t.layers = layers;
        t.submits = submits;
    }
}

fn get_gpu_composite() -> GpuCompositeTiming {
    GPU_COMPOSITE
        .get_or_init(|| Mutex::new(GpuCompositeTiming::default()))
        .lock()
        .map(|t| t.clone())
        .unwrap_or_default()
}

/// CPU-side breakdown of the composite section (wall-clock `Instant` deltas). Since
/// the GPU idles waiting on these CPU operations, this is where the per-frame cost
/// actually lives. Sums should ≈ the CPU `composite_ms` for the doc's active paths.
#[derive(Debug, Clone, Default)]
pub struct CompositeCpuBreakdown {
    /// `renderer.render_to_texture` — Vello scene encode + its internal submit.
    pub vello_ms: f64,
    /// `srgb_to_linear.convert` — recording the conversion pass.
    pub convert_ms: f64,
    /// `canvas_blit.blit` — recording + its internal submit.
    pub blit_ms: f64,
    /// `compositor.composite` — recording + per-call uniforms buffer / bind group alloc.
    pub composite_ms: f64,
    /// Explicit `queue.submit()` calls.
    pub submit_ms: f64,
}

static COMPOSITE_CPU: OnceLock<Mutex<CompositeCpuBreakdown>> = OnceLock::new();

/// Called from `VelloCallback::prepare()` with the composite CPU breakdown.
pub fn update_composite_cpu(b: CompositeCpuBreakdown) {
    let cell = COMPOSITE_CPU.get_or_init(|| Mutex::new(CompositeCpuBreakdown::default()));
    if let Ok(mut t) = cell.lock() {
        *t = b;
    }
}

fn get_composite_cpu() -> CompositeCpuBreakdown {
    COMPOSITE_CPU
        .get_or_init(|| Mutex::new(CompositeCpuBreakdown::default()))
        .lock()
        .map(|t| t.clone())
        .unwrap_or_default()
}

/// GPU memory the editor tracks itself (wgpu has no allocator query). Currently the
/// raster-layer texture cache — the only unbounded-by-default VRAM consumer.
#[derive(Debug, Clone, Default)]
pub struct GpuMemoryStats {
    pub raster_cache_entries: usize,
    pub raster_cache_bytes: usize,
}

static GPU_MEMORY: OnceLock<Mutex<GpuMemoryStats>> = OnceLock::new();

/// Called by the GPU brush whenever the raster-layer cache changes.
pub fn update_gpu_memory(raster_cache_entries: usize, raster_cache_bytes: usize) {
    let cell = GPU_MEMORY.get_or_init(|| Mutex::new(GpuMemoryStats::default()));
    if let Ok(mut s) = cell.lock() {
        s.raster_cache_entries = raster_cache_entries;
        s.raster_cache_bytes = raster_cache_bytes;
    }
}

fn get_gpu_memory() -> GpuMemoryStats {
    GPU_MEMORY
        .get_or_init(|| Mutex::new(GpuMemoryStats::default()))
        .lock()
        .map(|s| s.clone())
        .unwrap_or_default()
}

const DEVICE_REFRESH_INTERVAL: Duration = Duration::from_secs(2); // Refresh devices every 2 seconds
const MEMORY_REFRESH_INTERVAL: Duration = Duration::from_millis(500); // Refresh memory every 500ms

/// Statistics displayed in debug overlay
#[derive(Debug, Clone)]
pub struct DebugStats {
    pub fps_current: f32,      // Current frame FPS (unsmoothed)
    pub fps_min: f32,          // Minimum FPS over last 60 frames
    pub fps_avg: f32,          // Average FPS over last 60 frames
    pub fps_max: f32,          // Maximum FPS over last 60 frames
    pub frame_time_ms: f32,    // Current frame time in milliseconds
    pub memory_physical_mb: usize,
    pub memory_virtual_mb: usize,
    pub gpu_memory: GpuMemoryStats,
    pub gpu_name: String,
    pub gpu_backend: String,
    pub gpu_driver: String,
    pub midi_devices: Vec<String>,
    pub audio_input_devices: Vec<String>,
    pub has_pointer: bool,

    // GPU prepare() timing breakdown (from render thread)
    pub prepare_timing: PrepareTiming,

    // GPU-measured composite cost (timestamp queries)
    pub gpu_composite: GpuCompositeTiming,

    // CPU breakdown of the composite section
    pub composite_cpu: CompositeCpuBreakdown,

    // Performance metrics for each section
    pub timing_memory_us: u64,
    pub timing_gpu_us: u64,
    pub timing_midi_us: u64,
    pub timing_audio_us: u64,
    pub timing_pointer_us: u64,
    pub timing_total_us: u64,
}

/// Collects and aggregates debug statistics
pub struct DebugStatsCollector {
    frame_times: VecDeque<Duration>,
    last_frame_time: Option<Instant>,
    cached_audio_devices: Vec<String>,
    last_device_refresh: Option<Instant>,
    cached_memory_physical_mb: usize,
    cached_memory_virtual_mb: usize,
    last_memory_refresh: Option<Instant>,
}

impl DebugStatsCollector {
    pub fn new() -> Self {
        Self {
            frame_times: VecDeque::with_capacity(FRAME_HISTORY_SIZE),
            last_frame_time: None,
            cached_audio_devices: Vec::new(),
            last_device_refresh: None,
            cached_memory_physical_mb: 0,
            cached_memory_virtual_mb: 0,
            last_memory_refresh: None,
        }
    }

    /// Collect current debug statistics
    pub fn collect(
        &mut self,
        ctx: &egui::Context,
        gpu_info: &Option<wgpu::AdapterInfo>,
        audio_controller: Option<&std::sync::Arc<std::sync::Mutex<daw_backend::EngineController>>>,
    ) -> DebugStats {
        let collection_start = Instant::now();

        // Calculate actual frame time based on real elapsed time
        let now = Instant::now();
        let frame_duration = if let Some(last_time) = self.last_frame_time {
            now.duration_since(last_time)
        } else {
            Duration::from_secs_f32(1.0 / 60.0) // Default to 60 FPS for first frame
        };
        self.last_frame_time = Some(now);

        // Store frame duration in history
        self.frame_times.push_back(frame_duration);
        if self.frame_times.len() > FRAME_HISTORY_SIZE {
            self.frame_times.pop_front();
        }

        // Calculate FPS stats from actual frame durations
        let frame_time_ms = frame_duration.as_secs_f32() * 1000.0;
        let fps_current = 1.0 / frame_duration.as_secs_f32();

        let (fps_min, fps_avg, fps_max) = if !self.frame_times.is_empty() {
            let fps_values: Vec<f32> = self.frame_times
                .iter()
                .map(|dt| 1.0 / dt.as_secs_f32())
                .collect();
            let min = fps_values.iter().copied().fold(f32::INFINITY, f32::min);
            let max = fps_values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let sum: f32 = fps_values.iter().sum();
            let avg = sum / fps_values.len() as f32;
            (min, avg, max)
        } else {
            (fps_current, fps_current, fps_current)
        };

        // Collect memory stats with timing - cache and refresh every 500ms
        let t0 = Instant::now();
        let should_refresh_memory = self.last_memory_refresh
            .map(|last| now.duration_since(last) >= MEMORY_REFRESH_INTERVAL)
            .unwrap_or(true);

        if should_refresh_memory {
            if let Some(usage) = memory_stats::memory_stats() {
                self.cached_memory_physical_mb = usage.physical_mem / 1024 / 1024;
                self.cached_memory_virtual_mb = usage.virtual_mem / 1024 / 1024;
            }
            self.last_memory_refresh = Some(now);
        }

        let memory_physical_mb = self.cached_memory_physical_mb;
        let memory_virtual_mb = self.cached_memory_virtual_mb;
        let timing_memory_us = t0.elapsed().as_micros() as u64;

        // Extract GPU info with timing
        let t1 = Instant::now();
        let (gpu_name, gpu_backend, gpu_driver) = if let Some(info) = gpu_info {
            (
                info.name.clone(),
                format!("{:?}", info.backend),
                format!("{} ({})", info.driver, info.driver_info),
            )
        } else {
            ("Unknown".to_string(), "Unknown".to_string(), "Unknown".to_string())
        };
        let timing_gpu_us = t1.elapsed().as_micros() as u64;

        // Collect MIDI devices with timing
        let t2 = Instant::now();
        let midi_devices = if let Some(_controller) = audio_controller {
            // TODO: Add method to audio controller to get MIDI device names
            // For now, return empty vec
            vec![]
        } else {
            vec![]
        };
        let timing_midi_us = t2.elapsed().as_micros() as u64;

        // Refresh audio input devices only every 2 seconds to avoid performance issues
        let t3 = Instant::now();
        let should_refresh_devices = self.last_device_refresh
            .map(|last| now.duration_since(last) >= DEVICE_REFRESH_INTERVAL)
            .unwrap_or(true);

        if should_refresh_devices {
            self.cached_audio_devices = enumerate_audio_input_devices();
            self.last_device_refresh = Some(now);
        }

        let audio_input_devices = self.cached_audio_devices.clone();
        let timing_audio_us = t3.elapsed().as_micros() as u64;

        // Detect pointer usage with timing
        let t4 = Instant::now();
        let has_pointer = ctx.input(|i| {
            i.pointer.is_decidedly_dragging()
                || i.pointer.any_down()
                || i.pointer.any_pressed()
        });
        let timing_pointer_us = t4.elapsed().as_micros() as u64;

        let timing_total_us = collection_start.elapsed().as_micros() as u64;

        let prepare_timing = LAST_PREPARE_TIMING
            .get()
            .and_then(|m| m.lock().ok())
            .map(|t| t.clone())
            .unwrap_or_default();

        DebugStats {
            fps_current,
            fps_min,
            fps_avg,
            fps_max,
            frame_time_ms,
            memory_physical_mb,
            memory_virtual_mb,
            gpu_memory: get_gpu_memory(),
            gpu_name,
            gpu_backend,
            gpu_driver,
            midi_devices,
            audio_input_devices,
            has_pointer,
            prepare_timing,
            gpu_composite: get_gpu_composite(),
            composite_cpu: get_composite_cpu(),
            timing_memory_us,
            timing_gpu_us,
            timing_midi_us,
            timing_audio_us,
            timing_pointer_us,
            timing_total_us,
        }
    }
}

/// Enumerate audio input devices using cpal
fn enumerate_audio_input_devices() -> Vec<String> {
    use cpal::traits::{HostTrait, DeviceTrait};

    let host = cpal::default_host();
    host.input_devices()
        .ok()
        .map(|devices| {
            devices
                .filter_map(|d| d.description().ok().map(|desc| desc.name().to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// Render the debug overlay in-window using egui::Area
pub fn render_debug_overlay(ctx: &egui::Context, stats: &DebugStats) {
    egui::Area::new(egui::Id::new("debug_overlay_area"))
        .fixed_pos(egui::pos2(10.0, 10.0))
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_black_alpha(200))
                .inner_margin(8.0)
                .show(ui, |ui| {
                    // Use monospace font for alignment
                    ui.style_mut().override_text_style = Some(egui::TextStyle::Monospace);

                    // Performance section
                    ui.colored_label(egui::Color32::YELLOW, "Performance:");
                    ui.label(format!(
                        "FPS: {:.1} (min: {:.1} / avg: {:.1} / max: {:.1})",
                        stats.fps_current, stats.fps_min, stats.fps_avg, stats.fps_max
                    ));
                    ui.label(format!("Frame time: {:.2} ms", stats.frame_time_ms));

                    ui.add_space(8.0);

                    // GPU prepare() timing section
                    let pt = &stats.prepare_timing;
                    ui.colored_label(egui::Color32::YELLOW, format!("GPU prepare: {:.2} ms", pt.total_ms));
                    ui.label(format!("  removals:      {:.2} ms", pt.removals_ms));
                    ui.label(format!("  gpu_dispatch:  {:.2} ms", pt.gpu_dispatches_ms));
                    ui.label(format!("  scene_build:   {:.2} ms (CPU)", pt.scene_build_ms));
                    ui.label(format!("  composite:     {:.2} ms (CPU)", pt.composite_ms));

                    // GPU-measured composite cost (timestamp queries).
                    let gc = &stats.gpu_composite;
                    if gc.supported {
                        ui.colored_label(
                            egui::Color32::LIGHT_GREEN,
                            format!("GPU composite: {:.2} ms (GPU)", gc.composite_gpu_ms),
                        );
                        ui.label(format!("  layers: {}   submits: {}", gc.layers, gc.submits));
                    } else {
                        ui.label(format!(
                            "GPU composite: n/a (no timestamp support)   layers: {}   submits: {}",
                            gc.layers, gc.submits
                        ));
                    }

                    // CPU breakdown of the composite (where the GPU is actually waiting).
                    let cc = &stats.composite_cpu;
                    let cc_sum = cc.vello_ms + cc.convert_ms + cc.blit_ms + cc.composite_ms + cc.submit_ms;
                    ui.colored_label(egui::Color32::LIGHT_BLUE, format!("Composite CPU breakdown: {:.2} ms", cc_sum));
                    ui.label(format!("  vello(render):  {:.2} ms", cc.vello_ms));
                    ui.label(format!("  srgb→linear:    {:.2} ms", cc.convert_ms));
                    ui.label(format!("  blit:           {:.2} ms", cc.blit_ms));
                    ui.label(format!("  compositor:     {:.2} ms", cc.composite_ms));
                    ui.label(format!("  queue.submit:   {:.2} ms", cc.submit_ms));

                    ui.add_space(8.0);

                    // Memory section with timing
                    ui.colored_label(egui::Color32::YELLOW, format!("Memory: ({}µs)", stats.timing_memory_us));
                    ui.label(format!("Physical: {} MB", stats.memory_physical_mb));
                    ui.label(format!("Virtual: {} MB", stats.memory_virtual_mb));
                    ui.label(format!(
                        "VRAM (raster cache): {:.1} MB ({} frames)",
                        stats.gpu_memory.raster_cache_bytes as f64 / (1024.0 * 1024.0),
                        stats.gpu_memory.raster_cache_entries,
                    ));

                    ui.add_space(8.0);

                    // Graphics section with timing
                    ui.colored_label(egui::Color32::YELLOW, format!("Graphics: ({}µs)", stats.timing_gpu_us));
                    ui.label(format!("GPU: {}", stats.gpu_name));
                    ui.label(format!("Backend: {}", stats.gpu_backend));
                    ui.label(format!("Driver: {}", stats.gpu_driver));

                    ui.add_space(8.0);

                    // Input devices section with timing
                    ui.colored_label(egui::Color32::YELLOW, format!("Input Devices: ({}µs)",
                        stats.timing_midi_us + stats.timing_audio_us + stats.timing_pointer_us));

                    if stats.has_pointer {
                        ui.label(format!("• Mouse/Trackpad ({}µs)", stats.timing_pointer_us));
                    }

                    if !stats.audio_input_devices.is_empty() {
                        ui.label(format!("• {} Audio Input(s) ({}µs)",
                            stats.audio_input_devices.len(), stats.timing_audio_us));
                        for device in &stats.audio_input_devices {
                            ui.label(format!("  - {}", device));
                        }
                    }

                    if !stats.midi_devices.is_empty() {
                        ui.label(format!("• {} MIDI Device(s) ({}µs)",
                            stats.midi_devices.len(), stats.timing_midi_us));
                        for device in &stats.midi_devices {
                            ui.label(format!("  - {}", device));
                        }
                    }

                    ui.add_space(8.0);
                    ui.separator();
                    ui.colored_label(egui::Color32::CYAN, format!("Collection time: {}µs ({:.2}ms)",
                        stats.timing_total_us, stats.timing_total_us as f32 / 1000.0));
                    ui.colored_label(egui::Color32::GRAY, "Press F3 to close");
                });
        });
}

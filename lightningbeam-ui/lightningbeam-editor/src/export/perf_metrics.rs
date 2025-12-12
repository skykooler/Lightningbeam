//! Performance instrumentation for video export pipeline
//!
//! Tracks timing for each stage of the export process:
//! - GPU rendering (render_frame_to_gpu_yuv)
//! - Async readback (map_async completion)
//! - YUV plane extraction
//! - FFmpeg encoding
//! - Polling frequency and efficiency

use std::time::{Duration, Instant};

/// Performance metrics for a single frame
#[derive(Debug)]
pub struct FrameMetrics {
    pub frame_num: usize,
    pub render_start: Instant,
    pub render_end: Option<Instant>,
    pub submit_time: Option<Instant>,
    pub readback_complete: Option<Instant>,
    pub extraction_start: Option<Instant>,
    pub extraction_end: Option<Instant>,
    pub conversion_start: Option<Instant>,
    pub conversion_end: Option<Instant>,
    pub encode_start: Option<Instant>,
    pub encode_end: Option<Instant>,
}

impl FrameMetrics {
    pub fn new(frame_num: usize) -> Self {
        Self {
            frame_num,
            render_start: Instant::now(),
            render_end: None,
            submit_time: None,
            readback_complete: None,
            extraction_start: None,
            extraction_end: None,
            conversion_start: None,
            conversion_end: None,
            encode_start: None,
            encode_end: None,
        }
    }

    pub fn render_duration(&self) -> Option<Duration> {
        self.render_end.map(|end| end.duration_since(self.render_start))
    }

    pub fn readback_duration(&self) -> Option<Duration> {
        self.submit_time.and_then(|submit|
            self.readback_complete.map(|complete|
                complete.duration_since(submit)
            )
        )
    }

    pub fn extraction_duration(&self) -> Option<Duration> {
        self.extraction_start.and_then(|start|
            self.extraction_end.map(|end|
                end.duration_since(start)
            )
        )
    }

    pub fn conversion_duration(&self) -> Option<Duration> {
        self.conversion_start.and_then(|start|
            self.conversion_end.map(|end|
                end.duration_since(start)
            )
        )
    }

    pub fn encode_duration(&self) -> Option<Duration> {
        self.encode_start.and_then(|start|
            self.encode_end.map(|end|
                end.duration_since(start)
            )
        )
    }

    pub fn total_duration(&self) -> Option<Duration> {
        self.encode_end.map(|end| end.duration_since(self.render_start))
    }
}

/// Aggregate performance metrics for entire export
pub struct ExportMetrics {
    pub frames: Vec<FrameMetrics>,
    export_start: Instant,
    pub poll_count: usize,
    pub completions_per_poll: Vec<usize>,
}

impl ExportMetrics {
    pub fn new() -> Self {
        Self {
            frames: Vec::new(),
            export_start: Instant::now(),
            poll_count: 0,
            completions_per_poll: Vec::new(),
        }
    }

    /// Print comprehensive performance summary
    pub fn print_summary(&self) {
        println!("\n📊 [PERF] Export Performance Summary");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        // Calculate averages for each stage
        let mut render_times = Vec::new();
        let mut readback_times = Vec::new();
        let mut extraction_times = Vec::new();
        let mut conversion_times = Vec::new();
        let mut encode_times = Vec::new();
        let mut total_times = Vec::new();

        for metrics in &self.frames {
            if let Some(d) = metrics.render_duration() {
                render_times.push(d);
            }
            if let Some(d) = metrics.readback_duration() {
                readback_times.push(d);
            }
            if let Some(d) = metrics.extraction_duration() {
                extraction_times.push(d);
            }
            if let Some(d) = metrics.conversion_duration() {
                conversion_times.push(d);
            }
            if let Some(d) = metrics.encode_duration() {
                encode_times.push(d);
            }
            if let Some(d) = metrics.total_duration() {
                total_times.push(d);
            }
        }

        let avg = |times: &[Duration]| -> f64 {
            if times.is_empty() { return 0.0; }
            times.iter().sum::<Duration>().as_secs_f64() / times.len() as f64 * 1000.0
        };

        println!("Render:     {:.2}ms avg", avg(&render_times));
        println!("Readback:   {:.2}ms avg", avg(&readback_times));
        println!("Extraction: {:.2}ms avg", avg(&extraction_times));
        println!("Conversion: {:.2}ms avg", avg(&conversion_times));
        println!("Encode:     {:.2}ms avg", avg(&encode_times));
        println!("Total:      {:.2}ms avg", avg(&total_times));

        let total_export_time = Instant::now().duration_since(self.export_start).as_secs_f64();
        let fps = self.frames.len() as f64 / total_export_time;
        println!("\nOverall: {:.2} fps ({:.1}s for {} frames)",
                 fps, total_export_time, self.frames.len());

        if self.poll_count > 0 {
            let avg_completions = self.completions_per_poll.iter().sum::<usize>() as f64 / self.poll_count as f64;
            println!("Polls: {} ({:.2} completions/poll avg)",
                     self.poll_count, avg_completions);
        }

        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");
    }

    /// Print detailed per-frame breakdown for last N frames
    pub fn print_per_frame_details(&self, last_n: usize) {
        println!("\n📋 [PERF] Per-Frame Breakdown (last {} frames)", last_n);
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!("{:>5} | {:>8} | {:>8} | {:>8} | {:>8} | {:>8} | {:>8}",
                 "Frame", "Render", "Readback", "Extract", "Convert", "Encode", "Total");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        let start = if self.frames.len() > last_n {
            self.frames.len() - last_n
        } else {
            0
        };

        for metrics in &self.frames[start..] {
            println!("{:5} | {:>7.2}ms | {:>7.2}ms | {:>7.2}ms | {:>7.2}ms | {:>7.2}ms | {:>7.2}ms",
                metrics.frame_num,
                metrics.render_duration().map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0),
                metrics.readback_duration().map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0),
                metrics.extraction_duration().map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0),
                metrics.conversion_duration().map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0),
                metrics.encode_duration().map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0),
                metrics.total_duration().map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0),
            );
        }
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");
    }
}

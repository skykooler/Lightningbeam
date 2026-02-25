//! Debug test mode — input recording, panic capture, and visual replay.
//!
//! Gated behind `#[cfg(debug_assertions)]` at the module level in main.rs.

use eframe::egui;
use lightningbeam_core::test_mode::*;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use vello::kurbo::Point;

/// Maximum events kept in the always-on ring buffer for crash capture
const RING_BUFFER_SIZE: usize = 1000;

/// How often to snapshot state for the panic hook (every N events)
const PANIC_SNAPSHOT_INTERVAL: usize = 50;

// ---- Synthetic input for replay ----

/// Synthetic input data injected during replay, consumed by stage handle_input
pub struct SyntheticInput {
    pub world_pos: Point,
    pub drag_started: bool,
    pub dragged: bool,
    pub drag_stopped: bool,
    #[allow(dead_code)] // Part of the synthetic input API, consumed when replay handles held-button state
    pub primary_down: bool,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

// ---- State machine ----

pub enum TestModeOp {
    Idle,
    Recording(TestRecorder),
    Playing(TestPlayer),
}

pub struct TestRecorder {
    pub test_case: TestCase,
    pub start_time: Instant,
    event_count: usize,
}

impl TestRecorder {
    fn new(name: String, canvas_state: CanvasState) -> Self {
        Self {
            test_case: TestCase::new(name, canvas_state),
            start_time: Instant::now(),
            event_count: 0,
        }
    }

    fn record(&mut self, kind: TestEventKind) {
        let timestamp_ms = self.start_time.elapsed().as_millis() as u64;
        let index = self.event_count;
        self.event_count += 1;
        self.test_case.events.push(TestEvent {
            index,
            timestamp_ms,
            kind,
        });
    }
}

pub struct TestPlayer {
    pub test_case: TestCase,
    /// Next event index to execute
    pub cursor: usize,
    pub auto_playing: bool,
    pub auto_play_delay_ms: u64,
    pub last_step_time: Option<Instant>,
    /// Collapse consecutive move/drag events in the event list and step through them as one batch
    pub skip_consecutive_moves: bool,
    /// When set, auto-play runs at max speed until cursor reaches this index, then stops
    batch_end: Option<usize>,
}

impl TestPlayer {
    fn new(test_case: TestCase) -> Self {
        Self {
            test_case,
            cursor: 0,
            auto_playing: false,
            auto_play_delay_ms: 100,
            last_step_time: None,
            skip_consecutive_moves: true, // on by default
            batch_end: None,
        }
    }

    /// Advance cursor by one event and return it, or None if finished.
    pub fn step_forward(&mut self) -> Option<&TestEvent> {
        if self.cursor >= self.test_case.events.len() {
            self.auto_playing = false;
            self.batch_end = None;
            return None;
        }

        let idx = self.cursor;
        self.cursor += 1;
        self.last_step_time = Some(Instant::now());

        // Check if batch is done
        if let Some(end) = self.batch_end {
            if self.cursor >= end {
                self.auto_playing = false;
                self.batch_end = None;
            }
        }

        Some(&self.test_case.events[idx])
    }

    /// Start a batch-replay of consecutive move/drag events from the current cursor.
    /// Returns the first event in the batch, sets up auto-play for the rest.
    pub fn step_or_batch(&mut self) -> Option<&TestEvent> {
        if self.cursor >= self.test_case.events.len() {
            return None;
        }

        if self.skip_consecutive_moves {
            let disc = move_discriminant(&self.test_case.events[self.cursor].kind);
            if let Some(d) = disc {
                // Find end of consecutive run
                let mut end = self.cursor + 1;
                while end < self.test_case.events.len()
                    && move_discriminant(&self.test_case.events[end].kind) == Some(d)
                {
                    end += 1;
                }
                if end > self.cursor + 1 {
                    // Multi-event batch — auto-play through it at max speed
                    self.batch_end = Some(end);
                    self.auto_playing = true;
                }
            }
        }

        self.step_forward()
    }

    /// Whether auto-play should step this frame
    pub fn should_auto_step(&self) -> bool {
        if !self.auto_playing {
            return false;
        }
        // Batch replay: every frame, no delay
        if self.batch_end.is_some() {
            return true;
        }
        // Normal auto-play: respect delay setting
        match self.last_step_time {
            None => true,
            Some(t) => t.elapsed().as_millis() as u64 >= self.auto_play_delay_ms,
        }
    }

    pub fn progress(&self) -> (usize, usize) {
        (self.cursor, self.test_case.events.len())
    }

    pub fn reset(&mut self) {
        self.cursor = 0;
        self.auto_playing = false;
        self.last_step_time = None;
        self.batch_end = None;
    }
}

// ---- Main state ----

pub struct TestModeState {
    /// Whether the test mode sidebar is visible
    pub active: bool,
    /// Current operation
    pub mode: TestModeOp,
    /// Directory for test case files
    pub test_dir: PathBuf,
    /// List of available test files
    pub available_tests: Vec<PathBuf>,
    /// Name field for new recordings
    pub new_test_name: String,
    /// Transient status message
    pub status_message: Option<(String, Instant)>,
    /// Shared with panic hook — periodically updated with current state
    pub panic_snapshot: Arc<Mutex<Option<TestCase>>>,
    /// Current in-flight event, set before processing. If a panic occurs during
    /// processing, the panic hook appends this to the saved test case.
    pub pending_event: Arc<Mutex<Option<TestEvent>>>,
    /// Always-on ring buffer of last N events (for crash capture outside test mode)
    pub event_ring: VecDeque<TestEvent>,
    pub ring_start_time: Instant,
    ring_event_count: usize,
    /// Counter since last panic snapshot update
    events_since_snapshot: usize,
    /// Last replayed world-space position (for ghost cursor rendering on stage)
    pub replay_cursor_pos: Option<(f64, f64)>,
    /// Shared with panic hook — when true, panics during replay don't save new crash files
    pub is_replaying: Arc<AtomicBool>,
}

impl TestModeState {
    pub fn new(panic_snapshot: Arc<Mutex<Option<TestCase>>>, pending_event: Arc<Mutex<Option<TestEvent>>>, is_replaying: Arc<AtomicBool>) -> Self {
        let test_dir = directories::ProjectDirs::from("", "", "lightningbeam")
            .map(|dirs| dirs.data_dir().join("test_cases"))
            .unwrap_or_else(|| PathBuf::from("test_cases"));

        Self {
            active: false,
            mode: TestModeOp::Idle,
            test_dir,
            available_tests: Vec::new(),
            new_test_name: String::new(),
            status_message: None,
            panic_snapshot,
            pending_event,
            event_ring: VecDeque::with_capacity(RING_BUFFER_SIZE),
            ring_start_time: Instant::now(),
            ring_event_count: 0,
            events_since_snapshot: 0,
            replay_cursor_pos: None,
            is_replaying,
        }
    }

    /// Store the current in-flight event for panic capture.
    /// Called before processing so the panic hook can grab it if processing panics.
    pub fn set_pending_event(&self, kind: TestEventKind) {
        let event = TestEvent {
            index: self.ring_event_count,
            timestamp_ms: self.ring_start_time.elapsed().as_millis() as u64,
            kind,
        };
        if let Ok(mut guard) = self.pending_event.try_lock() {
            *guard = Some(event);
        }
    }

    /// Record an event — always appends to ring buffer, and to active recording if any
    pub fn record_event(&mut self, kind: TestEventKind) {
        let timestamp_ms = self.ring_start_time.elapsed().as_millis() as u64;
        let index = self.ring_event_count;
        self.ring_event_count += 1;

        let event = TestEvent {
            index,
            timestamp_ms,
            kind: kind.clone(),
        };

        // Always append to ring buffer
        if self.event_ring.len() >= RING_BUFFER_SIZE {
            self.event_ring.pop_front();
        }
        self.event_ring.push_back(event);

        // Append to active recording if any
        if let TestModeOp::Recording(ref mut recorder) = self.mode {
            recorder.record(kind);
        }

        // Periodically update panic snapshot
        self.events_since_snapshot += 1;
        if self.events_since_snapshot >= PANIC_SNAPSHOT_INTERVAL {
            self.events_since_snapshot = 0;
            self.update_panic_snapshot();
        }
    }

    /// Start a new recording
    pub fn start_recording(&mut self, name: String, canvas_state: CanvasState) {
        self.mode = TestModeOp::Recording(TestRecorder::new(name, canvas_state));
        self.set_status("Recording started");
    }

    /// Stop recording and save to disk. Returns path if saved successfully.
    pub fn stop_recording(&mut self) -> Option<PathBuf> {
        let recorder = match std::mem::replace(&mut self.mode, TestModeOp::Idle) {
            TestModeOp::Recording(r) => r,
            other => {
                self.mode = other;
                return None;
            }
        };

        let test_case = recorder.test_case;
        let filename = sanitize_filename(&test_case.name);
        let path = self.test_dir.join(format!("{}.json", filename));

        match test_case.save_to_file(&path) {
            Ok(()) => {
                self.set_status(&format!("Saved: {}", path.display()));
                self.refresh_test_list();
                Some(path)
            }
            Err(e) => {
                self.set_status(&format!("Save failed: {}", e));
                None
            }
        }
    }

    /// Discard the current recording
    pub fn discard_recording(&mut self) {
        self.mode = TestModeOp::Idle;
        self.set_status("Recording discarded");
    }

    /// Load a test case for playback
    pub fn load_test(&mut self, path: &PathBuf) {
        match TestCase::load_from_file(path) {
            Ok(test_case) => {
                self.set_status(&format!("Loaded: {} ({} events)", test_case.name, test_case.events.len()));
                self.mode = TestModeOp::Playing(TestPlayer::new(test_case));
                self.is_replaying.store(true, Ordering::SeqCst);
            }
            Err(e) => {
                self.set_status(&format!("Load failed: {}", e));
            }
        }
    }

    /// Stop playback and return to idle
    pub fn stop_playback(&mut self) {
        self.mode = TestModeOp::Idle;
        self.is_replaying.store(false, Ordering::SeqCst);
        self.set_status("Playback stopped");
    }

    /// Called from panic hook — saves ring buffer or active recording as a crash test case.
    /// Also grabs the pending in-flight event (if any) so the crash-triggering event is captured.
    /// Skips saving when replaying a recorded test (to avoid duplicate crash files).
    pub fn record_panic(
        panic_snapshot: &Arc<Mutex<Option<TestCase>>>,
        pending_event: &Arc<Mutex<Option<TestEvent>>>,
        is_replaying: &Arc<AtomicBool>,
        msg: String,
        backtrace: String,
        test_dir: &PathBuf,
    ) {
        if is_replaying.load(Ordering::SeqCst) {
            eprintln!("[TEST MODE] Panic during replay — not saving duplicate crash file");
            return;
        }
        if let Ok(mut guard) = panic_snapshot.lock() {
            let mut test_case = guard.take().unwrap_or_else(|| {
                TestCase::new(
                    "crash_capture".to_string(),
                    CanvasState {
                        zoom: 1.0,
                        pan_offset: (0.0, 0.0),
                        selected_tool: "Unknown".to_string(),
                        fill_color: [0, 0, 0, 255],
                        stroke_color: [0, 0, 0, 255],
                        stroke_width: 3.0,
                        fill_enabled: true,
                        snap_enabled: true,
                        polygon_sides: 5,
                    },
                )
            });

            // Append the in-flight event that was being processed when the panic occurred
            if let Ok(mut pending) = pending_event.try_lock() {
                if let Some(event) = pending.take() {
                    test_case.events.push(event);
                }
            }

            test_case.ended_with_panic = true;
            test_case.panic_message = Some(msg);
            test_case.panic_backtrace = Some(backtrace);

            let timestamp = format_timestamp();
            let path = test_dir.join(format!("crash_{}.json", timestamp));

            if let Err(e) = test_case.save_to_file(&path) {
                eprintln!("[TEST MODE] Failed to save crash test case: {}", e);
            } else {
                eprintln!("[TEST MODE] Crash test case saved to: {}", path.display());
            }
        }
    }

    /// Refresh the list of available test files
    pub fn refresh_test_list(&mut self) {
        self.available_tests.clear();
        if let Ok(entries) = std::fs::read_dir(&self.test_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map_or(false, |ext| ext == "json") {
                    self.available_tests.push(path);
                }
            }
        }
        self.available_tests.sort();
    }

    fn set_status(&mut self, msg: &str) {
        self.status_message = Some((msg.to_string(), Instant::now()));
    }

    /// Update the panic snapshot with current ring buffer state
    fn update_panic_snapshot(&self) {
        if let Ok(mut guard) = self.panic_snapshot.try_lock() {
            let events: Vec<TestEvent> = self.event_ring.iter().cloned().collect();
            let mut snapshot = TestCase::new(
                "ring_buffer_snapshot".to_string(),
                CanvasState {
                    zoom: 1.0,
                    pan_offset: (0.0, 0.0),
                    selected_tool: "Unknown".to_string(),
                    fill_color: [0, 0, 0, 255],
                    stroke_color: [0, 0, 0, 255],
                    stroke_width: 3.0,
                    fill_enabled: true,
                    snap_enabled: true,
                    polygon_sides: 5,
                },
            );
            snapshot.events = events;
            *guard = Some(snapshot);
        }
    }
}

// ---- Replay frame ----

/// Result of stepping a replay frame — carries both mouse input and non-mouse actions
#[derive(Default)]
pub struct ReplayFrame {
    pub synthetic_input: Option<SyntheticInput>,
    pub tool_change: Option<String>,
}

// ---- Sidebar UI ----

/// Render the test mode sidebar panel. Returns a ReplayFrame with actions to apply.
pub fn render_sidebar(
    ctx: &egui::Context,
    state: &mut TestModeState,
) -> ReplayFrame {
    if !state.active {
        return ReplayFrame::default();
    }

    let mut frame = ReplayFrame::default();
    let mut action = SidebarAction::None;

    egui::SidePanel::right("test_mode_panel")
        .default_width(300.0)
        .min_width(250.0)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("TEST MODE");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("X").clicked() {
                        state.active = false;
                    }
                });
            });
            ui.separator();

            // Status message (auto-clear after 5s)
            if let Some((ref msg, when)) = state.status_message {
                if when.elapsed().as_secs() < 5 {
                    ui.colored_label(egui::Color32::YELLOW, msg);
                    ui.separator();
                } else {
                    state.status_message = None;
                }
            }

            match &state.mode {
                TestModeOp::Idle => {
                    render_idle_ui(ui, state, &mut action);
                }
                TestModeOp::Recording(_) => {
                    render_recording_ui(ui, state, &mut action);
                }
                TestModeOp::Playing(_) => {
                    frame = render_playing_ui(ui, state, &mut action);
                }
            }
        });

    // Execute deferred actions (avoid borrow conflicts)
    match action {
        SidebarAction::None => {}
        SidebarAction::StartRecording(name, canvas) => {
            state.start_recording(name, canvas);
        }
        SidebarAction::StopRecording => {
            state.stop_recording();
        }
        SidebarAction::DiscardRecording => {
            state.discard_recording();
        }
        SidebarAction::LoadTest(path) => {
            state.load_test(&path);
        }
        SidebarAction::StopPlayback => {
            state.stop_playback();
        }
    }

    // Update ghost cursor position from the replay frame
    if let Some(ref syn) = frame.synthetic_input {
        state.replay_cursor_pos = Some((syn.world_pos.x, syn.world_pos.y));
    } else if !matches!(state.mode, TestModeOp::Playing(_)) {
        state.replay_cursor_pos = None;
    }

    frame
}

enum SidebarAction {
    None,
    StartRecording(String, CanvasState),
    StopRecording,
    DiscardRecording,
    LoadTest(PathBuf),
    StopPlayback,
}

fn render_idle_ui(ui: &mut egui::Ui, state: &mut TestModeState, action: &mut SidebarAction) {
    ui.horizontal(|ui| {
        if ui.button("Record").clicked() {
            let name = if state.new_test_name.is_empty() {
                format!("test_{}", format_timestamp())
            } else {
                state.new_test_name.clone()
            };
            // Default canvas state — will be overwritten by caller with real values
            let canvas = CanvasState {
                zoom: 1.0,
                pan_offset: (0.0, 0.0),
                selected_tool: "Select".to_string(),
                fill_color: [100, 100, 255, 255],
                stroke_color: [0, 0, 0, 255],
                stroke_width: 3.0,
                fill_enabled: true,
                snap_enabled: true,
                polygon_sides: 5,
            };
            *action = SidebarAction::StartRecording(name, canvas);
        }
        if ui.button("Load Test...").clicked() {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("JSON", &["json"])
                .set_directory(&state.test_dir)
                .pick_file()
            {
                *action = SidebarAction::LoadTest(path);
            }
        }
    });

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label("Name:");
        ui.text_edit_singleline(&mut state.new_test_name);
    });

    // List available tests
    ui.add_space(8.0);
    ui.separator();
    ui.label(egui::RichText::new("Saved Tests").strong());

    if state.available_tests.is_empty() {
        ui.colored_label(egui::Color32::GRAY, "(none)");
    } else {
        egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
            let mut load_path = None;
            for path in &state.available_tests {
                let name = path.file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                if ui.selectable_label(false, &name).clicked() {
                    load_path = Some(path.clone());
                }
            }
            if let Some(path) = load_path {
                *action = SidebarAction::LoadTest(path);
            }
        });
    }

    // Ring buffer info
    ui.add_space(8.0);
    ui.separator();
    ui.colored_label(
        egui::Color32::GRAY,
        format!("Ring buffer: {} events (auto-saved on crash)", state.event_ring.len()),
    );
}

fn render_recording_ui(ui: &mut egui::Ui, state: &mut TestModeState, action: &mut SidebarAction) {
    let event_count = match &state.mode {
        TestModeOp::Recording(r) => r.test_case.events.len(),
        _ => 0,
    };

    ui.colored_label(egui::Color32::from_rgb(255, 80, 80),
        format!("Recording... ({} events)", event_count));

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui.button("Stop & Save").clicked() {
            *action = SidebarAction::StopRecording;
        }
        if ui.button("Discard").clicked() {
            *action = SidebarAction::DiscardRecording;
        }
    });

    // Show recent events
    if let TestModeOp::Recording(ref recorder) = state.mode {
        render_event_list(ui, &recorder.test_case.events, None, false);
    }
}

fn render_playing_ui(
    ui: &mut egui::Ui,
    state: &mut TestModeState,
    action: &mut SidebarAction,
) -> ReplayFrame {
    let mut frame = ReplayFrame::default();

    let (cursor, total, test_name, has_panic, panic_msg, auto_playing, delay_ms) = match &state.mode {
        TestModeOp::Playing(p) => {
            let (c, t) = p.progress();
            (
                c, t,
                p.test_case.name.clone(),
                p.test_case.ended_with_panic,
                p.test_case.panic_message.clone(),
                p.auto_playing,
                p.auto_play_delay_ms,
            )
        }
        _ => return ReplayFrame::default(),
    };

    ui.label(format!("Test: {}", test_name));
    if has_panic {
        ui.colored_label(egui::Color32::RED, "Ended with PANIC");
        if let Some(ref msg) = panic_msg {
            let display_msg = if msg.len() > 120 { &msg[..120] } else { msg.as_str() };
            ui.colored_label(egui::Color32::from_rgb(255, 100, 100), display_msg);
        }
    }
    ui.label(format!("{}/{} events", cursor, total));

    // Transport controls
    ui.horizontal(|ui| {
        // Reset
        if ui.button("|<").clicked() {
            if let TestModeOp::Playing(ref mut p) = state.mode {
                p.reset();
            }
        }
        // Step back (reset to cursor - 1)
        if ui.button("<Step").clicked() {
            if let TestModeOp::Playing(ref mut p) = state.mode {
                if p.cursor > 0 {
                    p.cursor -= 1;
                }
            }
        }
        // Step forward (batches consecutive move/drag events when skip is on)
        if ui.button("Step>").clicked() {
            if let TestModeOp::Playing(ref mut p) = state.mode {
                if let Some(event) = p.step_or_batch() {
                    frame = event_to_replay_frame(event);
                }
            }
        }
        // Auto-play toggle
        let auto_label = if auto_playing { "||Pause" } else { ">>Auto" };
        if ui.button(auto_label).clicked() {
            if let TestModeOp::Playing(ref mut p) = state.mode {
                p.auto_playing = !p.auto_playing;
            }
        }
        // Stop
        if ui.button("Stop").clicked() {
            *action = SidebarAction::StopPlayback;
        }
    });

    // Speed slider
    ui.horizontal(|ui| {
        ui.label("Speed:");
        let mut delay = delay_ms as f32;
        if ui.add(egui::Slider::new(&mut delay, 10.0..=500.0).suffix("ms")).changed() {
            if let TestModeOp::Playing(ref mut p) = state.mode {
                p.auto_play_delay_ms = delay as u64;
            }
        }
    });

    // Skip consecutive moves toggle
    if let TestModeOp::Playing(ref mut p) = state.mode {
        ui.checkbox(&mut p.skip_consecutive_moves, "Skip consecutive moves");
    }

    // Auto-step
    if auto_playing {
        if let TestModeOp::Playing(ref mut p) = state.mode {
            if p.should_auto_step() {
                if let Some(event) = p.step_forward() {
                    frame = event_to_replay_frame(event);
                }
            }
        }
        // Request continuous repaint during auto-play
        ui.ctx().request_repaint();
    }

    // Event list
    if let TestModeOp::Playing(ref player) = state.mode {
        render_event_list(ui, &player.test_case.events, Some(player.cursor), player.skip_consecutive_moves);
    }

    frame
}

fn render_event_list(ui: &mut egui::Ui, events: &[TestEvent], cursor: Option<usize>, skip_moves: bool) {
    ui.add_space(8.0);
    ui.separator();
    ui.label(egui::RichText::new("Events").strong());

    // Build filtered index list when skip_moves is on
    let filtered: Vec<usize> = if skip_moves {
        filter_consecutive_moves(events)
    } else {
        (0..events.len()).collect()
    };

    egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .max_height(ui.available_height() - 20.0)
        .show(ui, |ui| {
            // Find the cursor position within the filtered list
            let cursor_filtered_pos = cursor.and_then(|c| {
                // Find the filtered entry closest to (but not past) cursor
                filtered.iter().rposition(|&idx| idx < c)
            });
            let focus = cursor_filtered_pos.unwrap_or(filtered.len().saturating_sub(1));
            let start = focus.saturating_sub(50);
            let end = (focus + 50).min(filtered.len());

            for &event_idx in &filtered[start..end] {
                let event = &events[event_idx];
                let is_current = cursor.map_or(false, |c| event.index == c.saturating_sub(1));
                let (prefix, color) = event_display_info(&event.kind, is_current);

                let text = format!("{} #{} {}", prefix, event.index, format_event_kind(&event.kind));
                let label = egui::RichText::new(text).color(color).monospace();
                ui.label(label);
            }
        });
}

/// Filter event indices, keeping only the last of each consecutive run of same-type moves/drags
fn filter_consecutive_moves(events: &[TestEvent]) -> Vec<usize> {
    let mut result = Vec::with_capacity(events.len());
    let mut i = 0;
    while i < events.len() {
        let disc = move_discriminant(&events[i].kind);
        if let Some(d) = disc {
            // Scan ahead to find the last in this consecutive run
            let mut last = i;
            while last + 1 < events.len() && move_discriminant(&events[last + 1].kind) == Some(d) {
                last += 1;
            }
            result.push(last);
            i = last + 1;
        } else {
            result.push(i);
            i += 1;
        }
    }
    result
}

fn event_display_info(kind: &TestEventKind, is_current: bool) -> (&'static str, egui::Color32) {
    let prefix = if is_current { ">" } else { " " };
    let color = match kind {
        TestEventKind::MouseMove { .. } | TestEventKind::MouseDrag { .. } => {
            egui::Color32::from_gray(140)
        }
        TestEventKind::MouseDown { .. } | TestEventKind::MouseUp { .. } => {
            egui::Color32::from_gray(200)
        }
        TestEventKind::ToolChanged { .. } => egui::Color32::from_rgb(100, 150, 255),
        TestEventKind::ActionExecuted { .. } => egui::Color32::from_rgb(100, 255, 100),
        TestEventKind::KeyDown { .. } | TestEventKind::KeyUp { .. } => {
            egui::Color32::from_rgb(200, 200, 100)
        }
        TestEventKind::Scroll { .. } => egui::Color32::from_gray(160),
    };
    (prefix, color)
}

fn format_event_kind(kind: &TestEventKind) -> String {
    match kind {
        TestEventKind::MouseDown { pos } => format!("MouseDown ({:.1}, {:.1})", pos.x, pos.y),
        TestEventKind::MouseUp { pos } => format!("MouseUp ({:.1}, {:.1})", pos.x, pos.y),
        TestEventKind::MouseDrag { pos } => format!("MouseDrag ({:.1}, {:.1})", pos.x, pos.y),
        TestEventKind::MouseMove { pos } => format!("MouseMove ({:.1}, {:.1})", pos.x, pos.y),
        TestEventKind::Scroll { delta_x, delta_y } => {
            format!("Scroll ({:.1}, {:.1})", delta_x, delta_y)
        }
        TestEventKind::KeyDown { key, .. } => format!("KeyDown {}", key),
        TestEventKind::KeyUp { key, .. } => format!("KeyUp {}", key),
        TestEventKind::ToolChanged { tool } => format!("ToolChanged: {}", tool),
        TestEventKind::ActionExecuted { description } => {
            format!("ActionExecuted: \"{}\"", description)
        }
    }
}

/// Convert a replayed TestEvent into a ReplayFrame carrying mouse input and/or tool changes
fn event_to_replay_frame(event: &TestEvent) -> ReplayFrame {
    let mut frame = ReplayFrame::default();
    match &event.kind {
        TestEventKind::ToolChanged { tool } => {
            frame.tool_change = Some(tool.clone());
        }
        other => {
            frame.synthetic_input = event_kind_to_synthetic(other);
        }
    }
    frame
}

/// Convert a mouse event kind into a SyntheticInput for the stage pane
fn event_kind_to_synthetic(kind: &TestEventKind) -> Option<SyntheticInput> {
    match kind {
        TestEventKind::MouseDown { pos } => Some(SyntheticInput {
            world_pos: Point::new(pos.x, pos.y),
            drag_started: true,
            dragged: true, // In egui, drag_started() implies dragged() on the same frame
            drag_stopped: false,
            primary_down: true,
            shift: false,
            ctrl: false,
            alt: false,
        }),
        TestEventKind::MouseDrag { pos } => Some(SyntheticInput {
            world_pos: Point::new(pos.x, pos.y),
            drag_started: false,
            dragged: true,
            drag_stopped: false,
            primary_down: true,
            shift: false,
            ctrl: false,
            alt: false,
        }),
        TestEventKind::MouseUp { pos } => Some(SyntheticInput {
            world_pos: Point::new(pos.x, pos.y),
            drag_started: false,
            dragged: true, // In egui, dragged() is still true on the release frame
            drag_stopped: true,
            primary_down: false,
            shift: false,
            ctrl: false,
            alt: false,
        }),
        TestEventKind::MouseMove { pos } => Some(SyntheticInput {
            world_pos: Point::new(pos.x, pos.y),
            drag_started: false,
            dragged: false,
            drag_stopped: false,
            primary_down: false,
            shift: false,
            ctrl: false,
            alt: false,
        }),
        // Non-mouse events don't produce synthetic input (handled elsewhere)
        _ => None,
    }
}

/// Returns a discriminant for "batchable" mouse motion event types.
/// Same-discriminant events are collapsed in the event list display
/// and replayed as a single batch when stepping.
/// Returns None for non-batchable events (clicks, tool changes, actions, etc.)
fn move_discriminant(kind: &TestEventKind) -> Option<u8> {
    match kind {
        TestEventKind::MouseMove { .. } => Some(0),
        TestEventKind::MouseDrag { .. } => Some(1),
        _ => None,
    }
}

/// Sanitize a string for use as a filename
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

/// Parse a tool name (from Debug format) back into a Tool enum value
pub fn parse_tool(name: &str) -> Option<lightningbeam_core::tool::Tool> {
    use lightningbeam_core::tool::Tool;
    match name {
        "Select" => Some(Tool::Select),
        "Draw" => Some(Tool::Draw),
        "Transform" => Some(Tool::Transform),
        "Rectangle" => Some(Tool::Rectangle),
        "Ellipse" => Some(Tool::Ellipse),
        "PaintBucket" => Some(Tool::PaintBucket),
        "Eyedropper" => Some(Tool::Eyedropper),
        "Line" => Some(Tool::Line),
        "Polygon" => Some(Tool::Polygon),
        "BezierEdit" => Some(Tool::BezierEdit),
        "Text" => Some(Tool::Text),
        "RegionSelect" => Some(Tool::RegionSelect),
        _ => None,
    }
}

/// Format current time as a compact timestamp (no chrono dependency in editor crate)
fn format_timestamp() -> String {
    use std::time::SystemTime;
    let duration = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    // Simple but unique timestamp: seconds since epoch
    // For human-readable format we'd need chrono, but this is fine for filenames
    format!("{}", secs)
}

use crate::audio::EngineController;
use crate::command::AudioEvent;
use crate::io::load_midi_file;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame, Terminal,
};
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// TUI application mode
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AppMode {
    /// Command mode - type vim-style commands
    Command,
    /// Play mode - use keyboard to play MIDI notes
    Play,
}

/// TUI application state
pub struct TuiApp {
    /// Current application mode
    mode: AppMode,
    /// Command input buffer (for Command mode)
    command_input: String,
    /// Current playback position (seconds)
    playback_position: f64,
    /// Whether playback is active
    is_playing: bool,
    /// Status message to display
    status_message: String,
    /// List of tracks (track_id, name)
    tracks: Vec<(u32, String)>,
    /// Currently selected track for MIDI input
    selected_track: Option<u32>,
    /// Active MIDI notes (currently held down)
    active_notes: Vec<u8>,
    /// Command history for up/down navigation
    command_history: Vec<String>,
    /// Current position in command history
    history_index: Option<usize>,
    /// Clips on timeline: (track_id, clip_id, start_time, duration, name, notes)
    /// Notes: Vec<(pitch, time_offset, duration)>
    clips: Vec<(u32, u32, f64, f64, String, Vec<(u8, f64, f64)>)>,
    /// Next clip ID for locally created clips
    next_clip_id: u32,
    /// Timeline scroll offset in seconds (start of visible window)
    timeline_scroll: f64,
    /// Timeline visible duration in seconds (zoom level)
    timeline_visible_duration: f64,
}

impl TuiApp {
    pub fn new() -> Self {
        Self {
            mode: AppMode::Command,
            command_input: String::new(),
            playback_position: 0.0,
            is_playing: false,
            status_message: "SPACE=play/pause | ←/→ scroll | -/+ zoom | 'i'=Play mode | Type 'help'".to_string(),
            tracks: Vec::new(),
            selected_track: None,
            active_notes: Vec::new(),
            command_history: Vec::new(),
            history_index: None,
            clips: Vec::new(),
            next_clip_id: 0,
            timeline_scroll: 0.0,
            timeline_visible_duration: 10.0, // Show 10 seconds at a time by default
        }
    }

    /// Switch to command mode
    pub fn enter_command_mode(&mut self) {
        self.mode = AppMode::Command;
        self.command_input.clear();
        self.history_index = None;
        self.status_message = "-- COMMAND -- SPACE=play/pause | ←/→ scroll | -/+ zoom | 'i' for Play mode | Type 'help'".to_string();
    }

    /// Switch to play mode
    pub fn enter_play_mode(&mut self) {
        self.mode = AppMode::Play;
        self.command_input.clear();
        self.status_message = "-- PLAY -- Press '?' for help, 'ESC' for Command mode".to_string();
    }

    /// Add a character to command input
    pub fn push_command_char(&mut self, c: char) {
        self.command_input.push(c);
    }

    /// Remove last character from command input
    pub fn pop_command_char(&mut self) {
        self.command_input.pop();
    }

    /// Get the current command input
    pub fn command_input(&self) -> &str {
        &self.command_input
    }

    /// Clear command input
    pub fn clear_command(&mut self) {
        self.command_input.clear();
        self.history_index = None;
    }

    /// Add command to history
    pub fn add_to_history(&mut self, command: String) {
        if !command.is_empty() && self.command_history.last() != Some(&command) {
            self.command_history.push(command);
        }
    }

    /// Navigate command history up
    pub fn history_up(&mut self) {
        if self.command_history.is_empty() {
            return;
        }

        let new_index = match self.history_index {
            None => Some(self.command_history.len() - 1),
            Some(0) => Some(0),
            Some(i) => Some(i - 1),
        };

        if let Some(idx) = new_index {
            self.history_index = Some(idx);
            self.command_input = self.command_history[idx].clone();
        }
    }

    /// Navigate command history down
    pub fn history_down(&mut self) {
        match self.history_index {
            None => {}
            Some(i) if i >= self.command_history.len() - 1 => {
                self.history_index = None;
                self.command_input.clear();
            }
            Some(i) => {
                let new_idx = i + 1;
                self.history_index = Some(new_idx);
                self.command_input = self.command_history[new_idx].clone();
            }
        }
    }

    /// Update playback position and auto-scroll timeline if needed
    pub fn update_playback_position(&mut self, position: f64) {
        self.playback_position = position;

        // Auto-scroll to keep playhead in view when playing
        if self.is_playing {
            // Keep playhead in the visible window, with some margin
            let margin = self.timeline_visible_duration * 0.1; // 10% margin

            // If playhead is ahead of visible window, scroll forward
            if position > self.timeline_scroll + self.timeline_visible_duration - margin {
                self.timeline_scroll = (position - self.timeline_visible_duration * 0.5).max(0.0);
            }
            // If playhead is behind visible window, scroll backward
            else if position < self.timeline_scroll + margin {
                self.timeline_scroll = (position - margin).max(0.0);
            }
        }
    }

    /// Set playing state
    pub fn set_playing(&mut self, playing: bool) {
        self.is_playing = playing;
    }

    /// Set status message
    pub fn set_status(&mut self, message: String) {
        self.status_message = message;
    }

    /// Add a track to the UI
    pub fn add_track(&mut self, track_id: u32, name: String) {
        self.tracks.push((track_id, name));
        // Auto-select first MIDI track for playing
        if self.selected_track.is_none() {
            self.selected_track = Some(track_id);
        }
    }

    /// Clear all tracks
    pub fn clear_tracks(&mut self) {
        self.tracks.clear();
        self.clips.clear();
        self.selected_track = None;
        self.next_clip_id = 0;
        self.timeline_scroll = 0.0;
    }

    /// Select a track by index
    pub fn select_track(&mut self, index: usize) {
        if let Some((track_id, _)) = self.tracks.get(index) {
            self.selected_track = Some(*track_id);
        }
    }

    /// Get selected track
    pub fn selected_track(&self) -> Option<u32> {
        self.selected_track
    }

    /// Add a clip to the timeline
    pub fn add_clip(&mut self, track_id: u32, clip_id: u32, start_time: f64, duration: f64, name: String, notes: Vec<(u8, f64, f64)>) {
        self.clips.push((track_id, clip_id, start_time, duration, name, notes));
    }

    /// Get max timeline duration based on clips
    pub fn get_timeline_duration(&self) -> f64 {
        self.clips
            .iter()
            .map(|(_, _, start, dur, _, _)| start + dur)
            .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or(10.0) // Default to 10 seconds if no clips
    }

    /// Add an active MIDI note
    pub fn add_active_note(&mut self, note: u8) {
        if !self.active_notes.contains(&note) {
            self.active_notes.push(note);
        }
    }

    /// Remove an active MIDI note
    pub fn remove_active_note(&mut self, note: u8) {
        self.active_notes.retain(|&n| n != note);
    }

    /// Get current mode
    pub fn mode(&self) -> AppMode {
        self.mode
    }

    /// Scroll timeline left
    pub fn scroll_timeline_left(&mut self) {
        let scroll_amount = self.timeline_visible_duration * 0.2; // Scroll by 20% of visible duration
        self.timeline_scroll = (self.timeline_scroll - scroll_amount).max(0.0);
    }

    /// Scroll timeline right
    pub fn scroll_timeline_right(&mut self) {
        let scroll_amount = self.timeline_visible_duration * 0.2; // Scroll by 20% of visible duration
        let max_duration = self.get_timeline_duration();
        self.timeline_scroll = (self.timeline_scroll + scroll_amount).min(max_duration - self.timeline_visible_duration).max(0.0);
    }

    /// Zoom timeline in (show less time, more detail)
    pub fn zoom_timeline_in(&mut self) {
        self.timeline_visible_duration = (self.timeline_visible_duration * 0.8).max(1.0); // Min 1 second visible
    }

    /// Zoom timeline out (show more time, less detail)
    pub fn zoom_timeline_out(&mut self) {
        let max_duration = self.get_timeline_duration();
        self.timeline_visible_duration = (self.timeline_visible_duration * 1.25).min(max_duration).max(1.0);
    }
}

/// Map keyboard keys to MIDI notes
/// Uses chromatic layout: awsedftgyhujkolp;'
/// This provides 1.5 octaves starting from C4 (MIDI note 60)
pub fn key_to_midi_note(key: KeyCode) -> Option<u8> {
    let base = 60; // C4

    match key {
        KeyCode::Char('a') => Some(base),      // C4
        KeyCode::Char('w') => Some(base + 1),  // C#4
        KeyCode::Char('s') => Some(base + 2),  // D4
        KeyCode::Char('e') => Some(base + 3),  // D#4
        KeyCode::Char('d') => Some(base + 4),  // E4
        KeyCode::Char('f') => Some(base + 5),  // F4
        KeyCode::Char('t') => Some(base + 6),  // F#4
        KeyCode::Char('g') => Some(base + 7),  // G4
        KeyCode::Char('y') => Some(base + 8),  // G#4
        KeyCode::Char('h') => Some(base + 9),  // A4
        KeyCode::Char('u') => Some(base + 10), // A#4
        KeyCode::Char('j') => Some(base + 11), // B4
        KeyCode::Char('k') => Some(base + 12), // C5
        KeyCode::Char('o') => Some(base + 13), // C#5
        KeyCode::Char('l') => Some(base + 14), // D5
        KeyCode::Char('p') => Some(base + 15), // D#5
        KeyCode::Char(';') => Some(base + 16), // E5
        KeyCode::Char('\'') => Some(base + 17), // F5

        _ => None,
    }
}

/// Convert pitch % 8 to braille dot bit position
fn pitch_to_braille_bit(pitch_mod_8: u8) -> u8 {
    match pitch_mod_8 {
        0 => 0x01, // Dot 1
        1 => 0x02, // Dot 2
        2 => 0x04, // Dot 3
        3 => 0x40, // Dot 7
        4 => 0x08, // Dot 4
        5 => 0x10, // Dot 5
        6 => 0x20, // Dot 6
        7 => 0x80, // Dot 8
        _ => 0x00,
    }
}

/// Draw the timeline view with clips
fn draw_timeline(f: &mut Frame, area: ratatui::layout::Rect, app: &TuiApp) {
    let num_tracks = app.tracks.len();

    // Use visible duration for the timeline window
    let visible_start = app.timeline_scroll;
    let visible_end = app.timeline_scroll + app.timeline_visible_duration;

    // Create the timeline block with visible range
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!("Timeline ({:.1}s - {:.1}s) | ←/→ scroll | -/+ zoom", visible_start, visible_end));

    let inner_area = block.inner(area);
    f.render_widget(block, area);

    // Calculate dimensions
    let width = inner_area.width as usize;

    if width == 0 || num_tracks == 0 {
        return;
    }

    // Fixed track height: 2 lines per track
    let track_height = 2;

    // Build timeline content with braille characters
    let mut lines: Vec<Line> = Vec::new();

    for track_idx in 0..num_tracks {
        let track_id = if let Some((id, _)) = app.tracks.get(track_idx) {
            *id
        } else {
            continue;
        };

        // Create exactly 2 lines for this track
        for _ in 0..track_height {
            let mut spans = Vec::new();

            // Build the timeline character by character
            for char_x in 0..width {
                // Map character position to time, using scroll offset
                let time_pos = visible_start + (char_x as f64 / width as f64) * app.timeline_visible_duration;

                // Check if playhead is at this position
                let is_playhead = (time_pos - app.playback_position).abs() < (app.timeline_visible_duration / width as f64);

                // Find all notes active at this time position on this track
                let mut braille_pattern: u8 = 0;
                let mut has_notes = false;

                for (clip_track_id, _clip_id, clip_start, _clip_duration, _name, notes) in &app.clips {
                    if *clip_track_id == track_id {
                        // Check each note in this clip
                        for (pitch, note_offset, note_duration) in notes {
                            let note_start = clip_start + note_offset;
                            let note_end = note_start + note_duration;

                            // Is this note active at current time position?
                            if time_pos >= note_start && time_pos < note_end {
                                let pitch_mod = pitch % 8;
                                braille_pattern |= pitch_to_braille_bit(pitch_mod);
                                has_notes = true;
                            }
                        }
                    }
                }

                // Determine color
                let color = if Some(track_id) == app.selected_track {
                    Color::Yellow
                } else {
                    Color::Cyan
                };

                // Create span
                if is_playhead {
                    // Playhead: red background
                    if has_notes {
                        // Show white notes with red background
                        let braille_char = char::from_u32(0x2800 + braille_pattern as u32).unwrap_or(' ');
                        spans.push(Span::styled(braille_char.to_string(), Style::default().fg(Color::White).bg(Color::Red)));
                    } else {
                        spans.push(Span::styled(" ", Style::default().bg(Color::Red)));
                    }
                } else if has_notes {
                    // Show white braille pattern on colored background
                    let braille_char = char::from_u32(0x2800 + braille_pattern as u32).unwrap_or(' ');
                    spans.push(Span::styled(braille_char.to_string(), Style::default().fg(Color::White).bg(color)));
                } else {
                    // Empty space
                    spans.push(Span::raw(" "));
                }
            }

            lines.push(Line::from(spans));
        }
    }

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, inner_area);
}

/// Draw the TUI
pub fn draw_ui(f: &mut Frame, app: &TuiApp) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Title bar
            Constraint::Min(10),     // Main content
            Constraint::Length(3),  // Status bar
            Constraint::Length(1),  // Command line
        ])
        .split(f.size());

    // Title bar
    let title = Paragraph::new("Lightningbeam DAW")
        .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(title, chunks[0]);

    // Main content area - split into tracks and timeline
    let content_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(20), Constraint::Percentage(80)])
        .split(chunks[1]);

    // Tracks list - each track gets 2 lines to match timeline
    let track_items: Vec<ListItem> = app
        .tracks
        .iter()
        .map(|(id, name)| {
            let style = if app.selected_track == Some(*id) {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            // Create a 2-line item: track info on first line, empty second line
            let lines = vec![
                Line::from(format!("T{}: {}", id, name)),
                Line::from(""),
            ];
            ListItem::new(lines).style(style)
        })
        .collect();

    let tracks_list = List::new(track_items)
        .block(Block::default().borders(Borders::ALL).title("Tracks"));
    f.render_widget(tracks_list, content_chunks[0]);

    // Timeline area - split vertically into playback info and timeline view
    let timeline_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(5)])
        .split(content_chunks[1]);

    // Playback info
    let playback_info = vec![
        Line::from(vec![
            Span::raw("Position: "),
            Span::styled(
                format!("{:.2}s", app.playback_position),
                Style::default().fg(Color::Green),
            ),
            Span::raw(" | Status: "),
            Span::styled(
                if app.is_playing { "Playing" } else { "Stopped" },
                if app.is_playing {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default().fg(Color::Red)
                },
            ),
        ]),
        Line::from(format!("Active Notes: {}",
            app.active_notes
                .iter()
                .map(|n| format!("{} ", n))
                .collect::<String>()
        )),
    ];

    let info = Paragraph::new(playback_info)
        .block(Block::default().borders(Borders::ALL).title("Playback"));
    f.render_widget(info, timeline_chunks[0]);

    // Draw timeline
    draw_timeline(f, timeline_chunks[1], app);

    // Status bar
    let mode_indicator = match app.mode {
        AppMode::Command => "COMMAND",
        AppMode::Play => "PLAY",
    };

    let status_text = format!("Mode: {} | {}", mode_indicator, app.status_message);
    let status_bar = Paragraph::new(status_text)
        .style(Style::default().fg(Color::White))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(status_bar, chunks[2]);

    // Command line
    let command_line = if app.mode == AppMode::Command {
        format!(":{}", app.command_input)
    } else {
        String::from("ESC=cmd mode | awsedftgyhujkolp;'=notes | R=release notes | ?=help | SPACE=play/pause")
    };

    let cmd_widget = Paragraph::new(command_line).style(Style::default().fg(Color::Yellow));
    f.render_widget(cmd_widget, chunks[3]);
}

/// Run the TUI application
pub fn run_tui(
    mut controller: EngineController,
    event_rx: Arc<Mutex<rtrb::Consumer<AudioEvent>>>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app state
    let mut app = TuiApp::new();

    // Main loop
    loop {
        // Draw UI
        terminal.draw(|f| draw_ui(f, &app))?;

        // Poll for audio events
        if let Ok(mut rx) = event_rx.lock() {
            while let Ok(event) = rx.pop() {
                match event {
                    AudioEvent::PlaybackPosition(pos) => {
                        app.update_playback_position(pos);
                    }
                    AudioEvent::PlaybackStopped => {
                        app.set_playing(false);
                    }
                    AudioEvent::TrackCreated(track_id, _, name) => {
                        app.add_track(track_id, name);
                    }
                    AudioEvent::RecordingStopped(clip_id, _pool_index, _waveform) => {
                        // Update status
                        app.set_status(format!("Recording stopped - Clip {}", clip_id));
                    }
                    AudioEvent::ProjectReset => {
                        app.clear_tracks();
                        app.set_status("Project reset".to_string());
                    }
                    _ => {}
                }
            }
        }

        // Handle keyboard input with timeout
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match app.mode() {
                    AppMode::Command => {
                        match key.code {
                            KeyCode::Left => {
                                // Scroll timeline left only if command buffer is empty
                                if app.command_input().is_empty() {
                                    app.scroll_timeline_left();
                                }
                            }
                            KeyCode::Right => {
                                // Scroll timeline right only if command buffer is empty
                                if app.command_input().is_empty() {
                                    app.scroll_timeline_right();
                                }
                            }
                            KeyCode::Char('-') | KeyCode::Char('_') => {
                                // Zoom out only if command buffer is empty
                                if app.command_input().is_empty() {
                                    app.zoom_timeline_out();
                                }
                            }
                            KeyCode::Char('+') | KeyCode::Char('=') => {
                                // Zoom in only if command buffer is empty
                                if app.command_input().is_empty() {
                                    app.zoom_timeline_in();
                                }
                            }
                            KeyCode::Char(' ') => {
                                // Spacebar toggles play/pause only if command buffer is empty
                                // Otherwise, add space to command
                                if app.command_input().is_empty() {
                                    if app.is_playing {
                                        controller.pause();
                                        app.set_playing(false);
                                        app.set_status("Paused".to_string());
                                    } else {
                                        controller.play();
                                        app.set_playing(true);
                                        app.set_status("Playing".to_string());
                                    }
                                } else {
                                    app.push_command_char(' ');
                                }
                            }
                            KeyCode::Esc => {
                                app.clear_command();
                            }
                            KeyCode::Enter => {
                                let command = app.command_input().to_string();
                                app.add_to_history(command.clone());

                                // Execute command
                                match execute_command(&command, &mut controller, &mut app) {
                                    Err(e) if e == "Quit requested" => {
                                        break; // Exit the application
                                    }
                                    Err(e) => {
                                        app.set_status(format!("Error: {}", e));
                                    }
                                    Ok(_) => {}
                                }

                                app.clear_command();
                            }
                            KeyCode::Backspace => {
                                app.pop_command_char();
                            }
                            KeyCode::Up => {
                                app.history_up();
                            }
                            KeyCode::Down => {
                                app.history_down();
                            }
                            KeyCode::Char('i') => {
                                // Only switch to Play mode if command buffer is empty
                                if app.command_input().is_empty() {
                                    app.enter_play_mode();
                                } else {
                                    app.push_command_char('i');
                                }
                            }
                            KeyCode::Char(c) => {
                                app.push_command_char(c);
                            }
                            _ => {}
                        }
                    }
                    AppMode::Play => {
                        // Check for mode switch first
                        if key.code == KeyCode::Esc {
                            app.enter_command_mode();
                            continue;
                        }

                        // Check for quit
                        if key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL) {
                            break;
                        }

                        // Handle MIDI note playing
                        if let Some(note) = key_to_midi_note(key.code) {
                            if let Some(track_id) = app.selected_track() {
                                // Release all previous notes before playing new one
                                for prev_note in app.active_notes.clone() {
                                    controller.send_midi_note_off(track_id, prev_note);
                                }
                                app.active_notes.clear();

                                // Play the new note
                                controller.send_midi_note_on(track_id, note, 100);
                                app.add_active_note(note);
                            }
                        } else {
                            // Handle other play mode shortcuts
                            match key.code {
                                KeyCode::Char(' ') => {
                                    // Release all notes and toggle play/pause
                                    if let Some(track_id) = app.selected_track() {
                                        for note in app.active_notes.clone() {
                                            controller.send_midi_note_off(track_id, note);
                                        }
                                        app.active_notes.clear();
                                    }

                                    if app.is_playing {
                                        controller.pause();
                                        app.set_playing(false);
                                    } else {
                                        controller.play();
                                        app.set_playing(true);
                                    }
                                }
                                KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                    // Release all notes and stop
                                    if let Some(track_id) = app.selected_track() {
                                        for note in app.active_notes.clone() {
                                            controller.send_midi_note_off(track_id, note);
                                        }
                                        app.active_notes.clear();
                                    }
                                    controller.stop();
                                    app.set_playing(false);
                                }
                                KeyCode::Char('r') | KeyCode::Char('R') => {
                                    // Release all notes manually (r for release)
                                    if let Some(track_id) = app.selected_track() {
                                        for note in app.active_notes.clone() {
                                            controller.send_midi_note_off(track_id, note);
                                        }
                                        app.active_notes.clear();
                                    }
                                    app.set_status("All notes released".to_string());
                                }
                                KeyCode::Char('?') | KeyCode::Char('h') | KeyCode::Char('H') => {
                                    app.set_status("Play Mode: awsedftgyhujkolp;'=notes | R=release | SPACE=play/pause | ESC=command | Ctrl+Q=quit".to_string());
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}

/// Execute a command string
fn execute_command(
    command: &str,
    controller: &mut EngineController,
    app: &mut TuiApp,
) -> Result<(), String> {
    let parts: Vec<&str> = command.trim().split_whitespace().collect();

    if parts.is_empty() {
        return Ok(());
    }

    match parts[0] {
        "play" => {
            controller.play();
            app.set_playing(true);
            app.set_status("Playing".to_string());
        }
        "pause" => {
            controller.pause();
            app.set_playing(false);
            app.set_status("Paused".to_string());
        }
        "stop" => {
            controller.stop();
            app.set_playing(false);
            app.set_status("Stopped".to_string());
        }
        "seek" => {
            if parts.len() < 2 {
                return Err("Usage: seek <seconds>".to_string());
            }
            let pos: f64 = parts[1].parse().map_err(|_| "Invalid position")?;
            controller.seek(pos);
            app.set_status(format!("Seeked to {:.2}s", pos));
        }
        "track" => {
            if parts.len() < 2 {
                return Err("Usage: track <name>".to_string());
            }
            let name = parts[1..].join(" ");
            controller.create_midi_track(name.clone());
            app.set_status(format!("Created MIDI track: {}", name));
        }
        "audiotrack" => {
            if parts.len() < 2 {
                return Err("Usage: audiotrack <name>".to_string());
            }
            let name = parts[1..].join(" ");
            controller.create_audio_track(name.clone());
            app.set_status(format!("Created audio track: {}", name));
        }
        "select" => {
            if parts.len() < 2 {
                return Err("Usage: select <track_number>".to_string());
            }
            let idx: usize = parts[1].parse().map_err(|_| "Invalid track number")?;
            app.select_track(idx);
            app.set_status(format!("Selected track {}", idx));
        }
        "clip" => {
            if parts.len() < 4 {
                return Err("Usage: clip <track_id> <start_time> <duration>".to_string());
            }
            let track_id: u32 = parts[1].parse().map_err(|_| "Invalid track ID")?;
            let start_time: f64 = parts[2].parse().map_err(|_| "Invalid start time")?;
            let duration: f64 = parts[3].parse().map_err(|_| "Invalid duration")?;

            // Add clip to local UI state (empty clip, no notes)
            let clip_id = app.next_clip_id;
            app.next_clip_id += 1;
            app.add_clip(track_id, clip_id, start_time, duration, format!("Clip {}", clip_id), Vec::new());

            controller.create_midi_clip(track_id, start_time, duration);
            app.set_status(format!("Created MIDI clip on track {} at {:.2}s for {:.2}s", track_id, start_time, duration));
        }
        "loadmidi" => {
            if parts.len() < 3 {
                return Err("Usage: loadmidi <track_id> <file_path> [start_time]".to_string());
            }
            let track_id: u32 = parts[1].parse().map_err(|_| "Invalid track ID")?;
            let file_path = parts[2];
            let start_time: f64 = if parts.len() >= 4 {
                parts[3].parse().unwrap_or(0.0)
            } else {
                0.0
            };

            // Load the MIDI file
            match load_midi_file(file_path, app.next_clip_id, 48000) {
                Ok(mut midi_clip) => {
                    midi_clip.start_time = start_time;
                    let clip_id = midi_clip.id;
                    let duration = midi_clip.duration;
                    let event_count = midi_clip.events.len();

                    // Extract note data for visualization
                    let mut notes = Vec::new();
                    let mut active_notes: std::collections::HashMap<u8, f64> = std::collections::HashMap::new();
                    let sample_rate = 48000.0; // Sample rate used for loading MIDI

                    for event in &midi_clip.events {
                        let status = event.status & 0xF0;
                        let time_seconds = event.timestamp as f64 / sample_rate;

                        match status {
                            0x90 if event.data2 > 0 => {
                                // Note on
                                active_notes.insert(event.data1, time_seconds);
                            }
                            0x80 | 0x90 => {
                                // Note off (or note on with velocity 0)
                                if let Some(start) = active_notes.remove(&event.data1) {
                                    let note_duration = time_seconds - start;
                                    notes.push((event.data1, start, note_duration));
                                }
                            }
                            _ => {}
                        }
                    }

                    // Add to local UI state with note data
                    app.add_clip(track_id, clip_id, start_time, duration, file_path.to_string(), notes);
                    app.next_clip_id += 1;

                    // Send to audio engine
                    controller.add_loaded_midi_clip(track_id, midi_clip);

                    app.set_status(format!("Loaded {} ({} events, {:.2}s) to track {} at {:.2}s",
                        file_path, event_count, duration, track_id, start_time));
                }
                Err(e) => {
                    return Err(format!("Failed to load MIDI file: {}", e));
                }
            }
        }
        "reset" => {
            controller.reset();
            app.clear_tracks();
            app.set_status("Project reset".to_string());
        }
        "q" | "quit" => {
            return Err("Quit requested".to_string());
        }
        "help" | "h" | "?" => {
            // Show comprehensive help
            let help_msg = concat!(
                "Commands: ",
                "play | pause | stop | seek <s> | ",
                "track <name> | audiotrack <name> | select <idx> | ",
                "clip <track_id> <start> <dur> | ",
                "loadmidi <track_id> <file> [start] | ",
                "reset | quit | help | ",
                "Keys: ←/→ scroll | -/+ zoom"
            );
            app.set_status(help_msg.to_string());
        }
        _ => {
            return Err(format!("Unknown command: '{}'. Type 'help' for commands", parts[0]));
        }
    }

    Ok(())
}

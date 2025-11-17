/// Timeline pane - Modern GarageBand-style timeline
///
/// Phase 1 Implementation: Time Ruler & Playhead
/// - Time-based ruler (seconds, not frames)
/// - Playhead for current time
/// - Zoom/pan controls
/// - Basic layer visualization

use eframe::egui;
use super::{NodePath, PaneRenderer, SharedPaneState};

const RULER_HEIGHT: f32 = 30.0;
const LAYER_HEIGHT: f32 = 60.0;
const LAYER_HEADER_WIDTH: f32 = 200.0;
const MIN_PIXELS_PER_SECOND: f32 = 20.0;
const MAX_PIXELS_PER_SECOND: f32 = 500.0;

pub struct TimelinePane {
    /// Current playback time in seconds
    current_time: f64,

    /// Horizontal zoom level (pixels per second)
    pixels_per_second: f32,

    /// Horizontal scroll offset (in seconds)
    viewport_start_time: f64,

    /// Vertical scroll offset (in pixels)
    viewport_scroll_y: f32,

    /// Total duration of the animation
    duration: f64,

    /// Is the user currently dragging the playhead?
    is_scrubbing: bool,

    /// Is the user panning the timeline?
    is_panning: bool,
    last_pan_pos: Option<egui::Pos2>,

    /// Is playback currently active?
    is_playing: bool,

    /// Currently selected/active layer index
    active_layer: usize,
}

impl TimelinePane {
    pub fn new() -> Self {
        Self {
            current_time: 0.0,
            pixels_per_second: 100.0,
            viewport_start_time: 0.0,
            active_layer: 0,
            viewport_scroll_y: 0.0,
            duration: 10.0,  // Default 10 seconds
            is_scrubbing: false,
            is_panning: false,
            last_pan_pos: None,
            is_playing: false,
        }
    }

    /// Execute a view action with the given parameters
    /// Called from main.rs after determining this is the best handler
    pub fn execute_view_action(&mut self, action: &crate::menu::MenuAction, zoom_center: egui::Vec2) {
        use crate::menu::MenuAction;
        match action {
            MenuAction::ZoomIn => self.zoom_in(zoom_center.x),
            MenuAction::ZoomOut => self.zoom_out(zoom_center.x),
            MenuAction::ActualSize => self.actual_size(),
            MenuAction::RecenterView => self.recenter(),
            _ => {} // Not a view action we handle
        }
    }

    /// Zoom in by a fixed increment
    pub fn zoom_in(&mut self, center_x: f32) {
        self.apply_zoom_at_point(0.2, center_x);
    }

    /// Zoom out by a fixed increment
    pub fn zoom_out(&mut self, center_x: f32) {
        self.apply_zoom_at_point(-0.2, center_x);
    }

    /// Reset zoom to 100 pixels per second
    pub fn actual_size(&mut self) {
        self.pixels_per_second = 100.0;
    }

    /// Reset pan to start and zoom to default
    pub fn recenter(&mut self) {
        self.viewport_start_time = 0.0;
        self.viewport_scroll_y = 0.0;
        self.pixels_per_second = 100.0;
    }

    /// Apply zoom while keeping the time under the cursor stationary
    fn apply_zoom_at_point(&mut self, zoom_delta: f32, mouse_x: f32) {
        let old_zoom = self.pixels_per_second;

        // Calculate time position under mouse before zoom
        let time_at_mouse = self.x_to_time(mouse_x);

        // Apply zoom
        let new_zoom = (old_zoom * (1.0 + zoom_delta)).clamp(MIN_PIXELS_PER_SECOND, MAX_PIXELS_PER_SECOND);
        self.pixels_per_second = new_zoom;

        // Adjust viewport so the same time stays under the mouse
        let new_mouse_x = self.time_to_x(time_at_mouse);
        let time_delta = (new_mouse_x - mouse_x) / new_zoom;
        self.viewport_start_time = (self.viewport_start_time + time_delta as f64).max(0.0);
    }

    /// Convert time (seconds) to pixel x-coordinate
    fn time_to_x(&self, time: f64) -> f32 {
        ((time - self.viewport_start_time) * self.pixels_per_second as f64) as f32
    }

    /// Convert pixel x-coordinate to time (seconds)
    fn x_to_time(&self, x: f32) -> f64 {
        self.viewport_start_time + (x / self.pixels_per_second) as f64
    }

    /// Calculate appropriate interval for time ruler based on zoom level
    fn calculate_ruler_interval(&self) -> f64 {
        // Target: 50-100px between major ticks
        let target_px = 75.0;
        let target_seconds = target_px / self.pixels_per_second;

        // Standard intervals: 0.1, 0.2, 0.5, 1, 2, 5, 10, 20, 50, 100...
        let intervals = [0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0];

        // Find the interval closest to our target
        intervals.iter()
            .min_by_key(|&&interval| ((interval - target_seconds as f64).abs() * 1000.0) as i32)
            .copied()
            .unwrap_or(1.0)
    }

    /// Render the time ruler at the top
    fn render_ruler(&self, ui: &mut egui::Ui, rect: egui::Rect, theme: &crate::theme::Theme) {
        let painter = ui.painter();

        // Background
        let bg_style = theme.style(".timeline-background", ui.ctx());
        let bg_color = bg_style.background_color.unwrap_or(egui::Color32::from_rgb(34, 34, 34));
        painter.rect_filled(
            rect,
            0.0,
            bg_color,
        );

        // Get text color from theme
        let text_style = theme.style(".text-primary", ui.ctx());
        let text_color = text_style.text_color.unwrap_or(egui::Color32::from_gray(200));

        // Calculate interval for tick marks
        let interval = self.calculate_ruler_interval();

        // Draw tick marks and labels
        let start_time = (self.viewport_start_time / interval).floor() * interval;
        let end_time = self.x_to_time(rect.width());

        let mut time = start_time;
        while time <= end_time {
            let x = self.time_to_x(time);

            if x >= 0.0 && x <= rect.width() {
                // Major tick mark
                painter.line_segment(
                    [
                        rect.min + egui::vec2(x, rect.height() - 10.0),
                        rect.min + egui::vec2(x, rect.height()),
                    ],
                    egui::Stroke::new(1.0, egui::Color32::from_gray(100)),
                );

                // Time label
                let label = format!("{:.1}s", time);
                painter.text(
                    rect.min + egui::vec2(x + 2.0, 5.0),
                    egui::Align2::LEFT_TOP,
                    label,
                    egui::FontId::proportional(12.0),
                    text_color,
                );
            }

            // Minor tick marks (subdivisions)
            let minor_interval = interval / 5.0;
            for i in 1..5 {
                let minor_time = time + minor_interval * i as f64;
                let minor_x = self.time_to_x(minor_time);

                if minor_x >= 0.0 && minor_x <= rect.width() {
                    painter.line_segment(
                        [
                            rect.min + egui::vec2(minor_x, rect.height() - 5.0),
                            rect.min + egui::vec2(minor_x, rect.height()),
                        ],
                        egui::Stroke::new(1.0, egui::Color32::from_gray(60)),
                    );
                }
            }

            time += interval;
        }
    }

    /// Render the playhead (current time indicator)
    fn render_playhead(&self, ui: &mut egui::Ui, rect: egui::Rect, theme: &crate::theme::Theme) {
        let x = self.time_to_x(self.current_time);

        if x >= 0.0 && x <= rect.width() {
            let painter = ui.painter();
            let scrubber_style = theme.style(".timeline-scrubber", ui.ctx());
            let scrubber_color = scrubber_style.background_color.unwrap_or(egui::Color32::from_rgb(204, 34, 34));

            // Red vertical line
            painter.line_segment(
                [
                    rect.min + egui::vec2(x, 0.0),
                    egui::pos2(rect.min.x + x, rect.max.y),
                ],
                egui::Stroke::new(2.0, scrubber_color),
            );

            // Playhead handle (triangle at top)
            let handle_size = 8.0;
            let points = vec![
                rect.min + egui::vec2(x, 0.0),
                rect.min + egui::vec2(x - handle_size / 2.0, handle_size),
                rect.min + egui::vec2(x + handle_size / 2.0, handle_size),
            ];
            painter.add(egui::Shape::convex_polygon(
                points,
                scrubber_color,
                egui::Stroke::NONE,
            ));
        }
    }

    /// Render layer header column (left side with track names and controls)
    fn render_layer_headers(&self, ui: &mut egui::Ui, rect: egui::Rect, theme: &crate::theme::Theme) {
        let painter = ui.painter();

        // Background for header column
        let header_style = theme.style(".timeline-header", ui.ctx());
        let header_bg = header_style.background_color.unwrap_or(egui::Color32::from_rgb(17, 17, 17));
        painter.rect_filled(
            rect,
            0.0,
            header_bg,
        );

        // Theme colors for active/inactive layers
        let active_style = theme.style(".timeline-layer-active", ui.ctx());
        let inactive_style = theme.style(".timeline-layer-inactive", ui.ctx());
        let active_color = active_style.background_color.unwrap_or(egui::Color32::from_rgb(79, 79, 79));
        let inactive_color = inactive_style.background_color.unwrap_or(egui::Color32::from_rgb(51, 51, 51));

        // Get text color from theme
        let text_style = theme.style(".text-primary", ui.ctx());
        let text_color = text_style.text_color.unwrap_or(egui::Color32::from_gray(200));

        // Test: Draw 3 layer headers
        for i in 0..3 {
            let y = rect.min.y + i as f32 * LAYER_HEIGHT - self.viewport_scroll_y;

            // Skip if layer is outside visible area
            if y + LAYER_HEIGHT < rect.min.y || y > rect.max.y {
                continue;
            }

            let header_rect = egui::Rect::from_min_size(
                egui::pos2(rect.min.x, y),
                egui::vec2(LAYER_HEADER_WIDTH, LAYER_HEIGHT),
            );

            // Active vs inactive background colors
            let bg_color = if i == self.active_layer {
                active_color
            } else {
                inactive_color
            };

            painter.rect_filled(header_rect, 0.0, bg_color);

            // Layer name
            painter.text(
                header_rect.min + egui::vec2(10.0, 10.0),
                egui::Align2::LEFT_TOP,
                format!("Layer {}", i + 1),
                egui::FontId::proportional(14.0),
                text_color,
            );

            // Separator line at bottom
            painter.line_segment(
                [
                    egui::pos2(header_rect.min.x, header_rect.max.y),
                    egui::pos2(header_rect.max.x, header_rect.max.y),
                ],
                egui::Stroke::new(1.0, egui::Color32::from_gray(20)),
            );
        }

        // Right border for header column
        painter.line_segment(
            [
                egui::pos2(rect.max.x, rect.min.y),
                egui::pos2(rect.max.x, rect.max.y),
            ],
            egui::Stroke::new(1.0, egui::Color32::from_gray(20)),
        );
    }

    /// Render layer rows (timeline content area)
    fn render_layers(&self, ui: &mut egui::Ui, rect: egui::Rect, theme: &crate::theme::Theme) {
        let painter = ui.painter();

        // Theme colors for active/inactive layers
        let active_style = theme.style(".timeline-row-active", ui.ctx());
        let inactive_style = theme.style(".timeline-row-inactive", ui.ctx());
        let active_color = active_style.background_color.unwrap_or(egui::Color32::from_rgb(85, 85, 85));
        let inactive_color = inactive_style.background_color.unwrap_or(egui::Color32::from_rgb(136, 136, 136));

        // Test: Draw 3 layer rows
        for i in 0..3 {
            let y = rect.min.y + i as f32 * LAYER_HEIGHT - self.viewport_scroll_y;

            // Skip if layer is outside visible area
            if y + LAYER_HEIGHT < rect.min.y || y > rect.max.y {
                continue;
            }

            let layer_rect = egui::Rect::from_min_size(
                egui::pos2(rect.min.x, y),
                egui::vec2(rect.width(), LAYER_HEIGHT),
            );

            // Active vs inactive background colors
            let bg_color = if i == self.active_layer {
                active_color
            } else {
                inactive_color
            };

            painter.rect_filled(layer_rect, 0.0, bg_color);

            // Grid lines matching ruler
            let interval = self.calculate_ruler_interval();
            let start_time = (self.viewport_start_time / interval).floor() * interval;
            let end_time = self.x_to_time(rect.width());

            let mut time = start_time;
            while time <= end_time {
                let x = self.time_to_x(time);

                if x >= 0.0 && x <= rect.width() {
                    painter.line_segment(
                        [
                            egui::pos2(rect.min.x + x, y),
                            egui::pos2(rect.min.x + x, y + LAYER_HEIGHT),
                        ],
                        egui::Stroke::new(1.0, egui::Color32::from_gray(30)),
                    );
                }

                time += interval;
            }

            // Separator line at bottom
            painter.line_segment(
                [
                    egui::pos2(layer_rect.min.x, layer_rect.max.y),
                    egui::pos2(layer_rect.max.x, layer_rect.max.y),
                ],
                egui::Stroke::new(1.0, egui::Color32::from_gray(20)),
            );
        }
    }

    /// Handle mouse input for scrubbing, panning, and zooming
    fn handle_input(&mut self, ui: &mut egui::Ui, full_timeline_rect: egui::Rect, ruler_rect: egui::Rect, content_rect: egui::Rect) {
        let response = ui.allocate_rect(full_timeline_rect, egui::Sense::click_and_drag());

        // Only process input if mouse is over the timeline pane
        if !response.hovered() {
            self.is_panning = false;
            self.last_pan_pos = None;
            self.is_scrubbing = false;
            return;
        }

        let alt_held = ui.input(|i| i.modifiers.alt);
        let ctrl_held = ui.input(|i| i.modifiers.ctrl || i.modifiers.command);

        // Get mouse position relative to content area
        let mouse_pos = response.hover_pos().unwrap_or(content_rect.center());
        let mouse_x = (mouse_pos.x - content_rect.min.x).max(0.0);

        // Calculate max vertical scroll based on number of layers
        // TODO: Get actual layer count from document - for now using test count of 3
        const TEST_LAYER_COUNT: usize = 3;
        let total_content_height = TEST_LAYER_COUNT as f32 * LAYER_HEIGHT;
        let visible_height = content_rect.height();
        let max_scroll_y = (total_content_height - visible_height).max(0.0);

        // Scrubbing (clicking/dragging on ruler, but only when not panning)
        if ruler_rect.contains(ui.input(|i| i.pointer.hover_pos().unwrap_or_default())) && !alt_held {
            if response.clicked() || (response.dragged() && !self.is_panning) {
                if let Some(pos) = response.interact_pointer_pos() {
                    let x = (pos.x - content_rect.min.x).max(0.0);
                    self.current_time = self.x_to_time(x).max(0.0).min(self.duration);
                    self.is_scrubbing = true;
                }
            } else if !response.dragged() {
                self.is_scrubbing = false;
            }
        } else {
            if !response.dragged() {
                self.is_scrubbing = false;
            }
        }

        // Distinguish between mouse wheel (discrete) and trackpad (smooth)
        let mut handled = false;
        ui.input(|i| {
            for event in &i.raw.events {
                if let egui::Event::MouseWheel { unit, delta, modifiers, .. } = event {
                    match unit {
                        egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => {
                            // Real mouse wheel (discrete clicks) -> always zoom horizontally
                            let zoom_delta = if ctrl_held || modifiers.ctrl {
                                delta.y * 0.01 // Ctrl+wheel: faster zoom
                            } else {
                                delta.y * 0.005 // Normal zoom
                            };
                            self.apply_zoom_at_point(zoom_delta, mouse_x);
                            handled = true;
                        }
                        egui::MouseWheelUnit::Point => {
                            // Trackpad (smooth scrolling)
                            if ctrl_held || modifiers.ctrl {
                                // Ctrl held: zoom
                                let zoom_delta = delta.y * 0.005;
                                self.apply_zoom_at_point(zoom_delta, mouse_x);
                                handled = true;
                            }
                            // Otherwise let scroll_delta handle panning (below)
                        }
                    }
                }
            }
        });

        // Handle scroll_delta for trackpad panning (when Ctrl not held)
        if !handled {
            let scroll_delta = ui.input(|i| i.smooth_scroll_delta);
            if scroll_delta.x.abs() > 0.0 || scroll_delta.y.abs() > 0.0 {
                // Horizontal scroll: pan timeline (inverted: positive delta scrolls left/earlier in time)
                let delta_time = scroll_delta.x / self.pixels_per_second;
                self.viewport_start_time = (self.viewport_start_time - delta_time as f64).max(0.0);

                // Vertical scroll: scroll layers vertically (clamped to content bounds)
                self.viewport_scroll_y = (self.viewport_scroll_y - scroll_delta.y).clamp(0.0, max_scroll_y);
            }
        }

        // Handle panning with Alt+Drag (timeline scrolls left/right, layers scroll up/down)
        if alt_held && response.dragged() && !self.is_scrubbing {
            if let Some(last_pos) = self.last_pan_pos {
                if let Some(current_pos) = response.interact_pointer_pos() {
                    let delta = current_pos - last_pos;

                    // Horizontal pan: timeline
                    let delta_time = delta.x / self.pixels_per_second;
                    self.viewport_start_time = (self.viewport_start_time - delta_time as f64).max(0.0);

                    // Vertical pan: layers (clamped to content bounds)
                    self.viewport_scroll_y = (self.viewport_scroll_y - delta.y).clamp(0.0, max_scroll_y);
                }
            }
            self.last_pan_pos = response.interact_pointer_pos();
            self.is_panning = true;
        } else {
            if !response.dragged() {
                self.is_panning = false;
                self.last_pan_pos = None;
            }
        }
    }
}

impl PaneRenderer for TimelinePane {
    fn render_header(&mut self, ui: &mut egui::Ui, shared: &mut SharedPaneState) -> bool {
        ui.spacing_mut().item_spacing.x = 2.0; // Small spacing between button groups

        // Main playback controls group
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0; // No spacing between buttons
                let button_size = egui::vec2(32.0, 28.0); // Larger buttons

                // Go to start
                if ui.add_sized(button_size, egui::Button::new("|◀")).clicked() {
                    self.current_time = 0.0;
                }

                // Rewind (step backward)
                if ui.add_sized(button_size, egui::Button::new("◀◀")).clicked() {
                    self.current_time = (self.current_time - 0.1).max(0.0);
                }

                // Play/Pause toggle
                let play_pause_text = if self.is_playing { "⏸" } else { "▶" };
                if ui.add_sized(button_size, egui::Button::new(play_pause_text)).clicked() {
                    self.is_playing = !self.is_playing;
                    // TODO: Actually start/stop playback
                }

                // Fast forward (step forward)
                if ui.add_sized(button_size, egui::Button::new("▶▶")).clicked() {
                    self.current_time = (self.current_time + 0.1).min(self.duration);
                }

                // Go to end
                if ui.add_sized(button_size, egui::Button::new("▶|")).clicked() {
                    self.current_time = self.duration;
                }
            });
        });

        ui.separator();

        // Get text color from theme
        let text_style = shared.theme.style(".text-primary", ui.ctx());
        let text_color = text_style.text_color.unwrap_or(egui::Color32::from_gray(200));

        // Time display
        ui.colored_label(text_color, format!("Time: {:.2}s / {:.2}s", self.current_time, self.duration));

        ui.separator();

        // Zoom display
        ui.colored_label(text_color, format!("Zoom: {:.0}px/s", self.pixels_per_second));

        true
    }

    fn render_content(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        _path: &NodePath,
        shared: &mut SharedPaneState,
    ) {
        // Split into layer header column (left) and timeline content (right)
        let header_column_rect = egui::Rect::from_min_size(
            rect.min,
            egui::vec2(LAYER_HEADER_WIDTH, rect.height()),
        );

        let timeline_rect = egui::Rect::from_min_size(
            rect.min + egui::vec2(LAYER_HEADER_WIDTH, 0.0),
            egui::vec2(rect.width() - LAYER_HEADER_WIDTH, rect.height()),
        );

        // Split timeline into ruler and content areas
        let ruler_rect = egui::Rect::from_min_size(
            timeline_rect.min,
            egui::vec2(timeline_rect.width(), RULER_HEIGHT),
        );

        let content_rect = egui::Rect::from_min_size(
            timeline_rect.min + egui::vec2(0.0, RULER_HEIGHT),
            egui::vec2(timeline_rect.width(), timeline_rect.height() - RULER_HEIGHT),
        );

        // Split header column into ruler area (top) and layer headers (bottom)
        let header_ruler_spacer = egui::Rect::from_min_size(
            header_column_rect.min,
            egui::vec2(LAYER_HEADER_WIDTH, RULER_HEIGHT),
        );

        let layer_headers_rect = egui::Rect::from_min_size(
            header_column_rect.min + egui::vec2(0.0, RULER_HEIGHT),
            egui::vec2(LAYER_HEADER_WIDTH, header_column_rect.height() - RULER_HEIGHT),
        );

        // Save original clip rect to restore at the end
        let original_clip_rect = ui.clip_rect();

        // Render spacer above layer headers (same height as ruler)
        let spacer_style = shared.theme.style(".timeline-spacer", ui.ctx());
        let spacer_bg = spacer_style.background_color.unwrap_or(egui::Color32::from_rgb(17, 17, 17));
        ui.painter().rect_filled(
            header_ruler_spacer,
            0.0,
            spacer_bg,
        );

        // Render layer header column with clipping
        ui.set_clip_rect(layer_headers_rect.intersect(original_clip_rect));
        self.render_layer_headers(ui, layer_headers_rect, shared.theme);

        // Render time ruler (clip to ruler rect)
        ui.set_clip_rect(ruler_rect.intersect(original_clip_rect));
        self.render_ruler(ui, ruler_rect, shared.theme);

        // Render layer rows with clipping
        ui.set_clip_rect(content_rect.intersect(original_clip_rect));
        self.render_layers(ui, content_rect, shared.theme);

        // Render playhead on top (clip to timeline area)
        ui.set_clip_rect(timeline_rect.intersect(original_clip_rect));
        self.render_playhead(ui, timeline_rect, shared.theme);

        // Restore original clip rect
        ui.set_clip_rect(original_clip_rect);

        // Handle input (use full rect including header column)
        self.handle_input(ui, rect, ruler_rect, content_rect);

        // Register handler for pending view actions (two-phase dispatch)
        // Priority: Mouse-over (0-99) > Fallback Timeline(1001)
        const TIMELINE_MOUSE_OVER_PRIORITY: u32 = 0;
        const TIMELINE_FALLBACK_PRIORITY: u32 = 1001;

        let mouse_over = ui.rect_contains_pointer(rect);

        // Determine our priority for this action
        let our_priority = if mouse_over {
            TIMELINE_MOUSE_OVER_PRIORITY  // High priority - mouse is over this pane
        } else {
            TIMELINE_FALLBACK_PRIORITY    // Low priority - just a fallback option
        };

        // Check if we should register as a handler (better priority than current best)
        let should_register = shared.pending_view_action.is_some() &&
            shared.fallback_pane_priority.map_or(true, |p| our_priority < p);

        if should_register {
            // Update fallback priority tracker
            *shared.fallback_pane_priority = Some(our_priority);

            // Register as a handler (don't execute yet - that happens after all panes render)
            if let Some(action) = &shared.pending_view_action {
                use crate::menu::MenuAction;

                // Determine zoom center point (use x-position only for timeline horizontal zoom)
                let center = if mouse_over {
                    // Use mouse position for zoom-to-cursor
                    let mouse_pos = ui.input(|i| i.pointer.hover_pos()).unwrap_or(rect.center());
                    mouse_pos - rect.min
                } else {
                    // Use center of viewport for fallback
                    rect.size() / 2.0
                };

                // Only register for actions we can handle
                match action {
                    MenuAction::ZoomIn | MenuAction::ZoomOut |
                    MenuAction::ActualSize | MenuAction::RecenterView => {
                        shared.pending_handlers.push(super::ViewActionHandler {
                            priority: our_priority,
                            pane_path: _path.clone(),
                            zoom_center: center,
                        });
                    }
                    _ => {
                        // Not a view action we handle - reset priority so others can try
                        *shared.fallback_pane_priority = None;
                    }
                }
            }
        }
    }

    fn name(&self) -> &str {
        "Timeline"
    }
}

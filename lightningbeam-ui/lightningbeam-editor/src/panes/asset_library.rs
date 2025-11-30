//! Asset Library pane - browse and manage project assets
//!
//! Displays all clips in the document organized by category:
//! - Vector Clips (animations)
//! - Video Clips (imported video files)
//! - Audio Clips (sampled audio and MIDI)

use eframe::egui;
use lightningbeam_core::clip::AudioClipType;
use lightningbeam_core::document::Document;
use uuid::Uuid;

use super::{DragClipType, DraggingAsset, NodePath, PaneRenderer, SharedPaneState};
use crate::widgets::ImeTextField;

// Layout constants
const SEARCH_BAR_HEIGHT: f32 = 30.0;
const CATEGORY_TAB_HEIGHT: f32 = 28.0;
const ITEM_HEIGHT: f32 = 40.0;
const ITEM_PADDING: f32 = 4.0;

/// Asset category for filtering
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetCategory {
    All,
    Vector,
    Video,
    Audio,
    Images,
}

impl AssetCategory {
    pub fn display_name(&self) -> &'static str {
        match self {
            AssetCategory::All => "All",
            AssetCategory::Vector => "Vector",
            AssetCategory::Video => "Video",
            AssetCategory::Audio => "Audio",
            AssetCategory::Images => "Images",
        }
    }

    pub fn all() -> &'static [AssetCategory] {
        &[
            AssetCategory::All,
            AssetCategory::Vector,
            AssetCategory::Video,
            AssetCategory::Audio,
            AssetCategory::Images,
        ]
    }

    /// Get the color associated with this category
    pub fn color(&self) -> egui::Color32 {
        match self {
            AssetCategory::All => egui::Color32::from_gray(150),
            AssetCategory::Vector => egui::Color32::from_rgb(100, 150, 255), // Blue
            AssetCategory::Video => egui::Color32::from_rgb(255, 150, 100),  // Orange
            AssetCategory::Audio => egui::Color32::from_rgb(100, 255, 150),  // Green
            AssetCategory::Images => egui::Color32::from_rgb(255, 200, 100), // Yellow/Gold
        }
    }
}

/// Unified asset entry for display
#[derive(Debug, Clone)]
pub struct AssetEntry {
    pub id: Uuid,
    pub name: String,
    pub category: AssetCategory,
    /// More specific clip type for drag-and-drop compatibility
    pub drag_clip_type: DragClipType,
    pub duration: f64,
    pub dimensions: Option<(f64, f64)>,
    pub extra_info: String,
}

/// Pending delete confirmation state
#[derive(Debug, Clone)]
struct PendingDelete {
    asset_id: Uuid,
    asset_name: String,
    category: AssetCategory,
    in_use: bool,
}

/// Inline rename editing state
#[derive(Debug, Clone)]
struct RenameState {
    asset_id: Uuid,
    category: AssetCategory,
    edit_text: String,
}

/// Context menu state with position
#[derive(Debug, Clone)]
struct ContextMenuState {
    asset_id: Uuid,
    position: egui::Pos2,
}

pub struct AssetLibraryPane {
    /// Current search filter text
    search_filter: String,

    /// Currently selected category tab
    selected_category: AssetCategory,

    /// Currently selected asset ID (for future drag-to-timeline)
    selected_asset: Option<Uuid>,

    /// Context menu state with position
    context_menu: Option<ContextMenuState>,

    /// Pending delete confirmation
    pending_delete: Option<PendingDelete>,

    /// Active rename state
    rename_state: Option<RenameState>,
}

impl AssetLibraryPane {
    pub fn new() -> Self {
        Self {
            search_filter: String::new(),
            selected_category: AssetCategory::All,
            selected_asset: None,
            context_menu: None,
            pending_delete: None,
            rename_state: None,
        }
    }

    /// Collect all assets from the document into a unified list
    fn collect_assets(&self, document: &Document) -> Vec<AssetEntry> {
        let mut assets = Vec::new();

        // Collect vector clips
        for (id, clip) in &document.vector_clips {
            assets.push(AssetEntry {
                id: *id,
                name: clip.name.clone(),
                category: AssetCategory::Vector,
                drag_clip_type: DragClipType::Vector,
                duration: clip.duration,
                dimensions: Some((clip.width, clip.height)),
                extra_info: format!("{}x{}", clip.width as u32, clip.height as u32),
            });
        }

        // Collect video clips
        for (id, clip) in &document.video_clips {
            assets.push(AssetEntry {
                id: *id,
                name: clip.name.clone(),
                category: AssetCategory::Video,
                drag_clip_type: DragClipType::Video,
                duration: clip.duration,
                dimensions: Some((clip.width, clip.height)),
                extra_info: format!("{:.0}fps", clip.frame_rate),
            });
        }

        // Collect audio clips
        for (id, clip) in &document.audio_clips {
            let (extra_info, drag_clip_type) = match &clip.clip_type {
                AudioClipType::Sampled { .. } => ("Sampled".to_string(), DragClipType::AudioSampled),
                AudioClipType::Midi { .. } => ("MIDI".to_string(), DragClipType::AudioMidi),
            };

            assets.push(AssetEntry {
                id: *id,
                name: clip.name.clone(),
                category: AssetCategory::Audio,
                drag_clip_type,
                duration: clip.duration,
                dimensions: None,
                extra_info,
            });
        }

        // Collect image assets
        for (id, asset) in &document.image_assets {
            assets.push(AssetEntry {
                id: *id,
                name: asset.name.clone(),
                category: AssetCategory::Images,
                drag_clip_type: DragClipType::Image,
                duration: 0.0, // Images don't have duration
                dimensions: Some((asset.width as f64, asset.height as f64)),
                extra_info: format!("{}x{}", asset.width, asset.height),
            });
        }

        // Sort alphabetically by name
        assets.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

        assets
    }

    /// Filter assets based on current category and search text
    fn filter_assets<'a>(&self, assets: &'a [AssetEntry]) -> Vec<&'a AssetEntry> {
        let search_lower = self.search_filter.to_lowercase();

        assets
            .iter()
            .filter(|asset| {
                // Category filter
                let category_matches = self.selected_category == AssetCategory::All
                    || asset.category == self.selected_category;

                // Search filter
                let search_matches =
                    search_lower.is_empty() || asset.name.to_lowercase().contains(&search_lower);

                category_matches && search_matches
            })
            .collect()
    }

    /// Check if an asset is currently in use (has clip instances on layers)
    fn is_asset_in_use(document: &Document, asset_id: Uuid, category: AssetCategory) -> bool {
        // Check all layers for clip instances referencing this asset
        for layer in &document.root.children {
            match layer {
                lightningbeam_core::layer::AnyLayer::Vector(vl) => {
                    if category == AssetCategory::Vector {
                        for instance in &vl.clip_instances {
                            if instance.clip_id == asset_id {
                                return true;
                            }
                        }
                    }
                }
                lightningbeam_core::layer::AnyLayer::Video(vl) => {
                    if category == AssetCategory::Video {
                        for instance in &vl.clip_instances {
                            if instance.clip_id == asset_id {
                                return true;
                            }
                        }
                    }
                }
                lightningbeam_core::layer::AnyLayer::Audio(al) => {
                    if category == AssetCategory::Audio {
                        for instance in &al.clip_instances {
                            if instance.clip_id == asset_id {
                                return true;
                            }
                        }
                    }
                }
            }
        }
        false
    }

    /// Delete an asset from the document
    fn delete_asset(document: &mut Document, asset_id: Uuid, category: AssetCategory) {
        match category {
            AssetCategory::Vector => {
                document.remove_vector_clip(&asset_id);
            }
            AssetCategory::Video => {
                document.remove_video_clip(&asset_id);
            }
            AssetCategory::Audio => {
                document.remove_audio_clip(&asset_id);
            }
            AssetCategory::Images => {
                document.remove_image_asset(&asset_id);
            }
            AssetCategory::All => {} // Not a real category for deletion
        }
    }

    /// Rename an asset in the document
    fn rename_asset(document: &mut Document, asset_id: Uuid, category: AssetCategory, new_name: &str) {
        match category {
            AssetCategory::Vector => {
                if let Some(clip) = document.get_vector_clip_mut(&asset_id) {
                    clip.name = new_name.to_string();
                }
            }
            AssetCategory::Video => {
                if let Some(clip) = document.get_video_clip_mut(&asset_id) {
                    clip.name = new_name.to_string();
                }
            }
            AssetCategory::Audio => {
                if let Some(clip) = document.get_audio_clip_mut(&asset_id) {
                    clip.name = new_name.to_string();
                }
            }
            AssetCategory::Images => {
                if let Some(asset) = document.get_image_asset_mut(&asset_id) {
                    asset.name = new_name.to_string();
                }
            }
            AssetCategory::All => {} // Not a real category for renaming
        }
    }

    /// Render the search bar at the top
    fn render_search_bar(&mut self, ui: &mut egui::Ui, rect: egui::Rect, shared: &SharedPaneState) {
        let search_rect =
            egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), SEARCH_BAR_HEIGHT));

        // Background
        let bg_style = shared.theme.style(".panel-header", ui.ctx());
        let bg_color = bg_style
            .background_color
            .unwrap_or(egui::Color32::from_rgb(30, 30, 30));
        ui.painter().rect_filled(search_rect, 0.0, bg_color);

        // Label position
        let label_pos = search_rect.min + egui::vec2(8.0, (SEARCH_BAR_HEIGHT - 14.0) / 2.0);
        ui.painter().text(
            label_pos,
            egui::Align2::LEFT_TOP,
            "Search:",
            egui::FontId::proportional(14.0),
            egui::Color32::from_gray(180),
        );

        // Text field using IME-safe widget
        let text_edit_rect = egui::Rect::from_min_size(
            search_rect.min + egui::vec2(65.0, 4.0),
            egui::vec2(search_rect.width() - 75.0, SEARCH_BAR_HEIGHT - 8.0),
        );

        let mut child_ui = ui.new_child(egui::UiBuilder::new().max_rect(text_edit_rect));
        ImeTextField::new(&mut self.search_filter)
            .placeholder("Filter assets...")
            .desired_width(text_edit_rect.width())
            .show(&mut child_ui);
    }

    /// Render category tabs
    fn render_category_tabs(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        shared: &SharedPaneState,
    ) {
        let tabs_rect =
            egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), CATEGORY_TAB_HEIGHT));

        // Background
        let bg_style = shared.theme.style(".panel-content", ui.ctx());
        let bg_color = bg_style
            .background_color
            .unwrap_or(egui::Color32::from_rgb(40, 40, 40));
        ui.painter().rect_filled(tabs_rect, 0.0, bg_color);

        // Tab buttons
        let tab_width = tabs_rect.width() / AssetCategory::all().len() as f32;

        for (i, category) in AssetCategory::all().iter().enumerate() {
            let tab_rect = egui::Rect::from_min_size(
                tabs_rect.min + egui::vec2(i as f32 * tab_width, 0.0),
                egui::vec2(tab_width, CATEGORY_TAB_HEIGHT),
            );

            let is_selected = self.selected_category == *category;

            // Tab background
            let tab_bg = if is_selected {
                egui::Color32::from_rgb(60, 60, 60)
            } else {
                egui::Color32::TRANSPARENT
            };
            ui.painter().rect_filled(tab_rect, 0.0, tab_bg);

            // Handle click
            let response = ui.allocate_rect(tab_rect, egui::Sense::click());
            if response.clicked() {
                self.selected_category = *category;
            }

            // Category color indicator
            let indicator_color = category.color();

            let text_color = if is_selected {
                indicator_color
            } else {
                egui::Color32::from_gray(150)
            };

            ui.painter().text(
                tab_rect.center(),
                egui::Align2::CENTER_CENTER,
                category.display_name(),
                egui::FontId::proportional(12.0),
                text_color,
            );

            // Underline for selected tab
            if is_selected {
                ui.painter().line_segment(
                    [
                        egui::pos2(tab_rect.min.x + 4.0, tab_rect.max.y - 2.0),
                        egui::pos2(tab_rect.max.x - 4.0, tab_rect.max.y - 2.0),
                    ],
                    egui::Stroke::new(2.0, indicator_color),
                );
            }
        }
    }

    /// Render the asset list
    fn render_asset_list(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        shared: &mut SharedPaneState,
        assets: &[&AssetEntry],
    ) {
        // Background
        let bg_style = shared.theme.style(".panel-content", ui.ctx());
        let bg_color = bg_style
            .background_color
            .unwrap_or(egui::Color32::from_rgb(25, 25, 25));
        ui.painter().rect_filled(rect, 0.0, bg_color);

        // Text colors
        let text_style = shared.theme.style(".text-primary", ui.ctx());
        let text_color = text_style
            .text_color
            .unwrap_or(egui::Color32::from_gray(200));
        let secondary_text_color = egui::Color32::from_gray(120);

        // Show empty state message if no assets
        if assets.is_empty() {
            let message = if !self.search_filter.is_empty() {
                "No assets match your search"
            } else {
                "No assets in this category"
            };

            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                message,
                egui::FontId::proportional(14.0),
                secondary_text_color,
            );
            return;
        }

        // Use egui's built-in ScrollArea for scrolling
        let scroll_area_rect = rect;
        ui.allocate_ui_at_rect(scroll_area_rect, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_min_width(scroll_area_rect.width() - 16.0); // Account for scrollbar

                    for asset in assets {
                        let (item_rect, response) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), ITEM_HEIGHT),
                            egui::Sense::click_and_drag(),
                        );

                        let is_selected = self.selected_asset == Some(asset.id);
                        let is_being_dragged = shared
                            .dragging_asset
                            .as_ref()
                            .map(|d| d.clip_id == asset.id)
                            .unwrap_or(false);

                        // Item background
                        let item_bg = if is_being_dragged {
                            egui::Color32::from_rgb(80, 100, 120) // Highlight when dragging
                        } else if is_selected {
                            egui::Color32::from_rgb(60, 80, 100)
                        } else if response.hovered() {
                            egui::Color32::from_rgb(45, 45, 45)
                        } else {
                            egui::Color32::from_rgb(35, 35, 35)
                        };
                        ui.painter().rect_filled(item_rect, 3.0, item_bg);

                        // Category color indicator bar
                        let indicator_color = asset.category.color();
                        let indicator_rect = egui::Rect::from_min_size(
                            item_rect.min,
                            egui::vec2(4.0, ITEM_HEIGHT),
                        );
                        ui.painter().rect_filled(indicator_rect, 0.0, indicator_color);

                        // Asset name (or inline edit field)
                        let is_renaming = self.rename_state.as_ref().map(|s| s.asset_id == asset.id).unwrap_or(false);

                        if is_renaming {
                            // Inline rename text field using IME-safe widget
                            let name_rect = egui::Rect::from_min_size(
                                item_rect.min + egui::vec2(10.0, 4.0),
                                egui::vec2(item_rect.width() - 20.0, 18.0),
                            );

                            if let Some(ref mut state) = self.rename_state {
                                let mut child_ui = ui.new_child(egui::UiBuilder::new().max_rect(name_rect));
                                ImeTextField::new(&mut state.edit_text)
                                    .font_size(13.0)
                                    .desired_width(name_rect.width())
                                    .request_focus()
                                    .show(&mut child_ui);
                            }
                        } else {
                            // Normal asset name display
                            ui.painter().text(
                                item_rect.min + egui::vec2(12.0, 8.0),
                                egui::Align2::LEFT_TOP,
                                &asset.name,
                                egui::FontId::proportional(13.0),
                                text_color,
                            );
                        }

                        // Metadata line (images don't have duration)
                        let metadata = if asset.category == AssetCategory::Images {
                            // For images, just show dimensions
                            asset.extra_info.clone()
                        } else if let Some((w, h)) = asset.dimensions {
                            format!(
                                "{:.1}s | {}x{} | {}",
                                asset.duration, w as u32, h as u32, asset.extra_info
                            )
                        } else {
                            format!("{:.1}s | {}", asset.duration, asset.extra_info)
                        };

                        ui.painter().text(
                            item_rect.min + egui::vec2(12.0, 24.0),
                            egui::Align2::LEFT_TOP,
                            &metadata,
                            egui::FontId::proportional(10.0),
                            secondary_text_color,
                        );

                        // Handle click (selection)
                        if response.clicked() {
                            self.selected_asset = Some(asset.id);
                        }

                        // Handle right-click (context menu)
                        if response.secondary_clicked() {
                            if let Some(pos) = ui.ctx().pointer_interact_pos() {
                                self.context_menu = Some(ContextMenuState {
                                    asset_id: asset.id,
                                    position: pos,
                                });
                            }
                        }

                        // Handle double-click (start rename)
                        if response.double_clicked() {
                            self.rename_state = Some(RenameState {
                                asset_id: asset.id,
                                category: asset.category,
                                edit_text: asset.name.clone(),
                            });
                        }

                        // Handle drag start
                        if response.drag_started() {
                            *shared.dragging_asset = Some(DraggingAsset {
                                clip_id: asset.id,
                                clip_type: asset.drag_clip_type,
                                name: asset.name.clone(),
                                duration: asset.duration,
                                dimensions: asset.dimensions,
                            });
                        }

                        // Add small spacing between items
                        ui.add_space(ITEM_PADDING);
                    }
                });
        });

        // Draw drag preview at cursor when dragging
        if let Some(dragging) = shared.dragging_asset.as_ref() {
            if let Some(pos) = ui.ctx().pointer_interact_pos() {
                // Draw a semi-transparent preview
                let preview_rect = egui::Rect::from_min_size(
                    pos + egui::vec2(10.0, 10.0), // Offset from cursor
                    egui::vec2(150.0, 30.0),
                );

                // Use top layer for drag preview
                let painter = ui.ctx().layer_painter(egui::LayerId::new(
                    egui::Order::Tooltip,
                    egui::Id::new("drag_preview"),
                ));

                painter.rect_filled(
                    preview_rect,
                    4.0,
                    egui::Color32::from_rgba_unmultiplied(60, 60, 60, 220),
                );

                painter.text(
                    preview_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    &dragging.name,
                    egui::FontId::proportional(12.0),
                    egui::Color32::WHITE,
                );
            }
        }

        // Clear drag state when mouse is released (if not dropped on valid target)
        // Note: Valid drop targets (Timeline, Stage) will clear this themselves
        if ui.input(|i| i.pointer.any_released()) {
            // Only clear if we're still within this pane (dropped back on library)
            if let Some(pos) = ui.ctx().pointer_interact_pos() {
                if rect.contains(pos) {
                    *shared.dragging_asset = None;
                }
            }
        }
    }
}

impl PaneRenderer for AssetLibraryPane {
    fn render_content(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        _path: &NodePath,
        shared: &mut SharedPaneState,
    ) {
        let document = shared.action_executor.document();

        // Collect and filter assets
        let all_assets = self.collect_assets(document);
        let filtered_assets = self.filter_assets(&all_assets);

        // Layout: Search bar -> Category tabs -> Asset list
        let search_rect =
            egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), SEARCH_BAR_HEIGHT));

        let tabs_rect = egui::Rect::from_min_size(
            rect.min + egui::vec2(0.0, SEARCH_BAR_HEIGHT),
            egui::vec2(rect.width(), CATEGORY_TAB_HEIGHT),
        );

        let list_rect = egui::Rect::from_min_max(
            rect.min + egui::vec2(0.0, SEARCH_BAR_HEIGHT + CATEGORY_TAB_HEIGHT),
            rect.max,
        );

        // Render components
        self.render_search_bar(ui, search_rect, shared);
        self.render_category_tabs(ui, tabs_rect, shared);
        self.render_asset_list(ui, list_rect, shared, &filtered_assets);

        // Context menu handling
        if let Some(ref context_state) = self.context_menu.clone() {
            let context_asset_id = context_state.asset_id;
            let menu_pos = context_state.position;

            // Find the asset info
            if let Some(asset) = all_assets.iter().find(|a| a.id == context_asset_id) {
                let asset_name = asset.name.clone();
                let asset_category = asset.category;
                let in_use = Self::is_asset_in_use(
                    shared.action_executor.document(),
                    context_asset_id,
                    asset_category,
                );

                // Show context menu popup at the stored position
                let menu_id = egui::Id::new("asset_context_menu");
                let menu_response = egui::Area::new(menu_id)
                    .order(egui::Order::Foreground)
                    .fixed_pos(menu_pos)
                    .show(ui.ctx(), |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            ui.set_min_width(120.0);

                            if ui.button("Rename").clicked() {
                                // Start inline rename
                                self.rename_state = Some(RenameState {
                                    asset_id: context_asset_id,
                                    category: asset_category,
                                    edit_text: asset_name.clone(),
                                });
                                self.context_menu = None;
                            }

                            if ui.button("Delete").clicked() {
                                // Set up pending delete confirmation
                                self.pending_delete = Some(PendingDelete {
                                    asset_id: context_asset_id,
                                    asset_name: asset_name.clone(),
                                    category: asset_category,
                                    in_use,
                                });
                                self.context_menu = None;
                            }
                        });
                    });

                // Close menu on click outside (using primary button release)
                let menu_rect = menu_response.response.rect;
                if ui.input(|i| i.pointer.primary_released()) {
                    if let Some(pos) = ui.ctx().pointer_interact_pos() {
                        if !menu_rect.contains(pos) {
                            self.context_menu = None;
                        }
                    }
                }

                // Also close on Escape
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    self.context_menu = None;
                }
            } else {
                self.context_menu = None;
            }
        }

        // Delete confirmation dialog
        if let Some(ref pending) = self.pending_delete.clone() {
            let window_id = egui::Id::new("delete_confirm_dialog");
            let mut should_close = false;
            let mut should_delete = false;

            egui::Window::new("Confirm Delete")
                .id(window_id)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ui.ctx(), |ui| {
                    ui.set_min_width(300.0);

                    if pending.in_use {
                        ui.label(egui::RichText::new("Warning: This asset is currently in use!")
                            .color(egui::Color32::from_rgb(255, 180, 100)));
                        ui.add_space(4.0);
                        ui.label("Deleting it will remove all clip instances that reference it.");
                        ui.add_space(8.0);
                    }

                    ui.label(format!("Are you sure you want to delete \"{}\"?", pending.asset_name));
                    ui.add_space(12.0);

                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            should_close = true;
                        }

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let delete_text = if pending.in_use { "Delete Anyway" } else { "Delete" };
                            if ui.button(delete_text).clicked() {
                                should_delete = true;
                                should_close = true;
                            }
                        });
                    });
                });

            if should_delete {
                // Perform the delete
                Self::delete_asset(
                    shared.action_executor.document_mut(),
                    pending.asset_id,
                    pending.category,
                );
            }

            if should_close {
                self.pending_delete = None;
            }
        }

        // Handle rename state (Enter to confirm, Escape to cancel, click outside to confirm)
        if let Some(ref state) = self.rename_state.clone() {
            let mut should_confirm = false;
            let mut should_cancel = false;

            // Check for Enter or Escape
            ui.input(|i| {
                if i.key_pressed(egui::Key::Enter) {
                    should_confirm = true;
                } else if i.key_pressed(egui::Key::Escape) {
                    should_cancel = true;
                }
            });

            if should_confirm {
                let new_name = state.edit_text.trim();
                if !new_name.is_empty() {
                    Self::rename_asset(
                        shared.action_executor.document_mut(),
                        state.asset_id,
                        state.category,
                        new_name,
                    );
                }
                self.rename_state = None;
            } else if should_cancel {
                self.rename_state = None;
            }
        }
    }

    fn name(&self) -> &str {
        "Asset Library"
    }
}

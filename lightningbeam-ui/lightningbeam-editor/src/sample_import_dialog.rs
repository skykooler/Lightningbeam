//! Import dialog for MultiSampler folder import.
//!
//! Shows a preview of parsed samples with editable note mappings, velocity ranges,
//! and loop mode before committing the import.

use eframe::egui;
use egui_node_graph2::NodeId;
use std::path::PathBuf;

use crate::sample_import::{
    FolderScanResult, midi_to_note_name, recalc_key_ranges,
};
use daw_backend::audio::node_graph::nodes::LoopMode;

pub struct SampleImportDialog {
    pub folder_path: PathBuf,
    pub scan_result: FolderScanResult,
    pub loop_mode: LoopMode,
    pub auto_key_ranges: bool,
    pub confirmed: bool,
    pub should_close: bool,
    pub track_id: u32,
    pub backend_node_id: u32,
    pub node_id: NodeId,
}

impl SampleImportDialog {
    pub fn new(
        folder_path: PathBuf,
        scan_result: FolderScanResult,
        track_id: u32,
        backend_node_id: u32,
        node_id: NodeId,
    ) -> Self {
        let loop_mode = scan_result.loop_mode;
        Self {
            folder_path,
            scan_result,
            loop_mode,
            auto_key_ranges: true,
            confirmed: false,
            should_close: false,
            track_id,
            backend_node_id,
            node_id,
        }
    }

    /// Returns true while the dialog is still open.
    pub fn show(&mut self, ctx: &egui::Context) -> bool {
        let mut open = true;
        let mut should_import = false;
        let mut should_cancel = false;
        let mut recalc = false;

        egui::Window::new("Import Samples")
            .open(&mut open)
            .resizable(true)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .default_width(700.0)
            .default_height(500.0)
            .show(ctx, |ui| {
                // Folder info
                ui.label(format!("Folder: {}", self.folder_path.display()));

                let enabled_count = self.scan_result.layers.iter().filter(|l| l.enabled).count();
                let unique_notes: std::collections::HashSet<u8> = self.scan_result.layers.iter()
                    .filter(|l| l.enabled)
                    .map(|l| l.root_key)
                    .collect();
                let vel_count = self.scan_result.velocity_markers.len();
                ui.label(format!(
                    "Found: {} samples, {} notes, {} velocity layer{}",
                    enabled_count,
                    unique_notes.len(),
                    vel_count,
                    if vel_count != 1 { "s" } else { "" },
                ));
                ui.add_space(4.0);

                // Global controls
                ui.horizontal(|ui| {
                    ui.label("Loop mode:");
                    egui::ComboBox::from_id_salt("loop_mode")
                        .selected_text(match self.loop_mode {
                            LoopMode::OneShot => "One Shot",
                            LoopMode::Continuous => "Continuous",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.loop_mode, LoopMode::OneShot, "One Shot");
                            ui.selectable_value(&mut self.loop_mode, LoopMode::Continuous, "Continuous");
                        });

                    ui.add_space(16.0);
                    if ui.checkbox(&mut self.auto_key_ranges, "Auto key ranges").changed() {
                        if self.auto_key_ranges {
                            recalc = true;
                        }
                    }
                });
                ui.add_space(4.0);

                // Velocity mapping table
                if !self.scan_result.velocity_ranges.is_empty() {
                    ui.collapsing("Velocity Mapping", |ui| {
                        egui::Grid::new("vel_grid").striped(true).show(ui, |ui| {
                            ui.label(egui::RichText::new("Marker").strong());
                            ui.label(egui::RichText::new("Min").strong());
                            ui.label(egui::RichText::new("Max").strong());
                            ui.end_row();

                            for (marker, min, max) in &mut self.scan_result.velocity_ranges {
                                ui.label(&*marker);
                                ui.add(egui::DragValue::new(min).range(0..=127).speed(1));
                                ui.add(egui::DragValue::new(max).range(0..=127).speed(1));
                                ui.end_row();
                            }
                        });
                    });
                    ui.add_space(4.0);
                }

                // Layers table
                ui.separator();
                ui.label(egui::RichText::new("Layers").strong());
                let available_height = ui.available_height() - 40.0; // reserve space for buttons
                egui::ScrollArea::vertical()
                    .max_height(available_height.max(100.0))
                    .show(ui, |ui| {
                        egui::Grid::new("layers_grid")
                            .striped(true)
                            .min_col_width(20.0)
                            .show(ui, |ui| {
                                // Header
                                ui.label(""); // checkbox column
                                ui.label(egui::RichText::new("File").strong());
                                ui.label(egui::RichText::new("Root").strong());
                                ui.label(egui::RichText::new("Key Range").strong());
                                ui.label(egui::RichText::new("Vel Range").strong());
                                ui.end_row();

                                for i in 0..self.scan_result.layers.len() {
                                    let layer = &mut self.scan_result.layers[i];
                                    if ui.checkbox(&mut layer.enabled, "").changed() && self.auto_key_ranges {
                                        recalc = true;
                                    }

                                    // Filename (truncated)
                                    let name = if layer.filename.len() > 40 {
                                        format!("...{}", &layer.filename[layer.filename.len()-37..])
                                    } else {
                                        layer.filename.clone()
                                    };
                                    ui.label(&name).on_hover_text(&layer.filename);

                                    // Root note
                                    let mut root = layer.root_key as i32;
                                    if ui.add(egui::DragValue::new(&mut root)
                                        .range(0..=127)
                                        .speed(1)
                                        .custom_formatter(|v, _| midi_to_note_name(v as u8))
                                    ).changed() {
                                        layer.root_key = root as u8;
                                        if self.auto_key_ranges {
                                            recalc = true;
                                        }
                                    }

                                    // Key range
                                    if self.auto_key_ranges {
                                        ui.label(format!("{}-{}", midi_to_note_name(layer.key_min), midi_to_note_name(layer.key_max)));
                                    } else {
                                        let mut kmin = layer.key_min as i32;
                                        let mut kmax = layer.key_max as i32;
                                        ui.horizontal(|ui| {
                                            if ui.add(egui::DragValue::new(&mut kmin).range(0..=127).speed(1)
                                                .custom_formatter(|v, _| midi_to_note_name(v as u8))
                                            ).changed() {
                                                layer.key_min = kmin as u8;
                                            }
                                            ui.label("-");
                                            if ui.add(egui::DragValue::new(&mut kmax).range(0..=127).speed(1)
                                                .custom_formatter(|v, _| midi_to_note_name(v as u8))
                                            ).changed() {
                                                layer.key_max = kmax as u8;
                                            }
                                        });
                                    }

                                    // Velocity range
                                    ui.label(format!("{}-{}", layer.velocity_min, layer.velocity_max));
                                    ui.end_row();
                                }
                            });

                        // Unmapped section
                        if !self.scan_result.unmapped.is_empty() {
                            ui.add_space(8.0);
                            ui.label(egui::RichText::new(format!("Unmapped ({})", self.scan_result.unmapped.len())).strong());
                            for sample in &self.scan_result.unmapped {
                                ui.label(format!("  {}", sample.filename));
                            }
                        }
                    });

                // Buttons
                ui.add_space(4.0);
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        should_cancel = true;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let import_text = format!("Import {} layers", enabled_count);
                        if ui.add_enabled(enabled_count > 0, egui::Button::new(&import_text)).clicked() {
                            should_import = true;
                        }
                    });
                });
            });

        if recalc {
            recalc_key_ranges(&mut self.scan_result.layers);
        }

        if should_import {
            self.confirmed = true;
            self.should_close = true;
        }
        if should_cancel || !open {
            self.should_close = true;
        }

        !self.should_close
    }
}

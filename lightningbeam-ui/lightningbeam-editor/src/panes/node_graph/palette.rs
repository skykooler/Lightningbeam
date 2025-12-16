//! Node Palette UI
//!
//! Left sidebar showing available node types organized by category

use super::node_types::{NodeCategory, NodeTypeRegistry};
use eframe::egui;

/// Node palette state
pub struct NodePalette {
    /// Node type registry
    registry: NodeTypeRegistry,

    /// Category collapse states
    collapsed_categories: std::collections::HashSet<NodeCategory>,

    /// Search filter text
    search_filter: String,
}

impl NodePalette {
    pub fn new() -> Self {
        Self {
            registry: NodeTypeRegistry::new(),
            collapsed_categories: std::collections::HashSet::new(),
            search_filter: String::new(),
        }
    }

    /// Render the palette UI
    ///
    /// The `on_node_clicked` callback is called when the user clicks a node type to add it
    pub fn render<F>(&mut self, ui: &mut egui::Ui, rect: egui::Rect, mut on_node_clicked: F)
    where
        F: FnMut(&str),
    {
        // Draw background
        ui.painter()
            .rect_filled(rect, 0.0, egui::Color32::from_rgb(30, 30, 30));

        // Create UI within the palette rect
        ui.allocate_ui_at_rect(rect, |ui| {
            ui.vertical(|ui| {
                ui.add_space(8.0);

                // Title
                ui.heading("Node Palette");
                ui.add_space(4.0);

                // Search box
                ui.horizontal(|ui| {
                    ui.label("Search:");
                    ui.text_edit_singleline(&mut self.search_filter);
                });

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);

                // Scrollable node list
                egui::ScrollArea::vertical()
                    .id_salt("node_palette_scroll")
                    .show(ui, |ui| {
                        self.render_categories(ui, &mut on_node_clicked);
                    });
            });
        });
    }

    fn render_categories<F>(&mut self, ui: &mut egui::Ui, on_node_clicked: &mut F)
    where
        F: FnMut(&str),
    {
        let search_lower = self.search_filter.to_lowercase();

        for category in self.registry.all_categories() {
            // Get nodes in this category
            let mut nodes = self.registry.get_by_category(category);

            // Filter by search text (node names only)
            if !search_lower.is_empty() {
                nodes.retain(|node| {
                    node.display_name.to_lowercase().contains(&search_lower)
                });
            }

            // Skip empty categories
            if nodes.is_empty() {
                continue;
            }

            // Sort nodes by name
            nodes.sort_by(|a, b| a.display_name.cmp(&b.display_name));

            // Render category header
            let is_collapsed = self.collapsed_categories.contains(&category);
            let arrow = if is_collapsed { ">" } else { "v" };
            let label = format!("{} {} ({})", arrow, category.display_name(), nodes.len());

            let header_response = ui.selectable_label(false, label);

            // Toggle collapse on click
            if header_response.clicked() {
                if is_collapsed {
                    self.collapsed_categories.remove(&category);
                } else {
                    self.collapsed_categories.insert(category);
                }
            }

            // Render nodes if not collapsed
            if !is_collapsed {
                ui.indent(category.display_name(), |ui| {
                    for node in nodes {
                        self.render_node_button(ui, node.id.as_str(), &node.display_name, on_node_clicked);
                    }
                });
            }

            ui.add_space(4.0);
        }
    }

    fn render_node_button<F>(
        &self,
        ui: &mut egui::Ui,
        node_id: &str,
        display_name: &str,
        on_node_clicked: &mut F,
    ) where
        F: FnMut(&str),
    {
        // Use drag source to enable dragging
        let drag_id = egui::Id::new(format!("node_palette_{}", node_id));
        let response = ui.dnd_drag_source(
            drag_id,
            node_id.to_string(),
            |ui| {
                let button = egui::Button::new(display_name)
                    .min_size(egui::vec2(ui.available_width() - 8.0, 24.0))
                    .fill(egui::Color32::from_rgb(50, 50, 50));
                ui.add(button)
            },
        );

        // Handle click: detect clicks by checking if drag stopped with minimal movement
        // dnd_drag_source always sets is_being_dragged=true on press, so we can't use that
        if response.response.drag_stopped() {
            // Check if this was actually a drag or just a click (minimal movement)
            if let Some(start_pos) = response.response.interact_pointer_pos() {
                if let Some(current_pos) = ui.input(|i| i.pointer.interact_pos()) {
                    let drag_distance = (current_pos - start_pos).length();
                    if drag_distance < 5.0 {
                        // This was a click, not a drag
                        on_node_clicked(node_id);
                    }
                }
            }
        }

        // Show tooltip with description
        if let Some(node_info) = self.registry.get(node_id) {
            response.response.on_hover_text(&node_info.description);
        }
    }
}

impl Default for NodePalette {
    fn default() -> Self {
        Self::new()
    }
}

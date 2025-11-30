/// Info Panel pane - displays and edits properties of selected objects
///
/// Shows context-sensitive property editors based on current selection:
/// - Tool options (when a tool is active)
/// - Transform properties (when shapes are selected)
/// - Shape properties (fill/stroke for selected shapes)
/// - Document settings (when nothing is selected)

use eframe::egui::{self, DragValue, Sense, Ui};
use lightningbeam_core::actions::{
    InstancePropertyChange, SetDocumentPropertiesAction, SetInstancePropertiesAction,
    SetShapePropertiesAction,
};
use lightningbeam_core::layer::AnyLayer;
use lightningbeam_core::shape::ShapeColor;
use lightningbeam_core::tool::{SimplifyMode, Tool};
use super::{NodePath, PaneRenderer, SharedPaneState};
use uuid::Uuid;

/// Info panel pane state
pub struct InfopanelPane {
    /// Whether the tool options section is expanded
    tool_section_open: bool,
    /// Whether the transform section is expanded
    transform_section_open: bool,
    /// Whether the shape properties section is expanded
    shape_section_open: bool,
}

impl InfopanelPane {
    pub fn new() -> Self {
        Self {
            tool_section_open: true,
            transform_section_open: true,
            shape_section_open: true,
        }
    }
}

/// Aggregated info about the current selection
struct SelectionInfo {
    /// True if nothing is selected
    is_empty: bool,
    /// Number of selected shape instances
    shape_count: usize,
    /// Layer ID of selected shapes (assumes single layer selection for now)
    layer_id: Option<Uuid>,
    /// Selected shape instance IDs
    instance_ids: Vec<Uuid>,
    /// Shape IDs referenced by selected instances
    shape_ids: Vec<Uuid>,

    // Transform values (None = mixed values across selection)
    x: Option<f64>,
    y: Option<f64>,
    rotation: Option<f64>,
    scale_x: Option<f64>,
    scale_y: Option<f64>,
    skew_x: Option<f64>,
    skew_y: Option<f64>,
    opacity: Option<f64>,

    // Shape property values (None = mixed)
    fill_color: Option<Option<ShapeColor>>,
    stroke_color: Option<Option<ShapeColor>>,
    stroke_width: Option<f64>,
}

impl Default for SelectionInfo {
    fn default() -> Self {
        Self {
            is_empty: true,
            shape_count: 0,
            layer_id: None,
            instance_ids: Vec::new(),
            shape_ids: Vec::new(),
            x: None,
            y: None,
            rotation: None,
            scale_x: None,
            scale_y: None,
            skew_x: None,
            skew_y: None,
            opacity: None,
            fill_color: None,
            stroke_color: None,
            stroke_width: None,
        }
    }
}

impl InfopanelPane {
    /// Gather info about the current selection
    fn gather_selection_info(&self, shared: &SharedPaneState) -> SelectionInfo {
        let mut info = SelectionInfo::default();

        let selected_instances = shared.selection.shape_instances();
        info.shape_count = selected_instances.len();
        info.is_empty = info.shape_count == 0;

        if info.is_empty {
            return info;
        }

        info.instance_ids = selected_instances.to_vec();

        // Find the layer containing the selected instances
        let document = shared.action_executor.document();
        let active_layer_id = *shared.active_layer_id;

        if let Some(layer_id) = active_layer_id {
            info.layer_id = Some(layer_id);

            if let Some(layer) = document.get_layer(&layer_id) {
                if let AnyLayer::Vector(vector_layer) = layer {
                    // Gather values from all selected instances
                    let mut first = true;

                    for instance_id in &info.instance_ids {
                        if let Some(instance) = vector_layer.get_object(instance_id) {
                            info.shape_ids.push(instance.shape_id);

                            if first {
                                // First instance - set initial values
                                info.x = Some(instance.transform.x);
                                info.y = Some(instance.transform.y);
                                info.rotation = Some(instance.transform.rotation);
                                info.scale_x = Some(instance.transform.scale_x);
                                info.scale_y = Some(instance.transform.scale_y);
                                info.skew_x = Some(instance.transform.skew_x);
                                info.skew_y = Some(instance.transform.skew_y);
                                info.opacity = Some(instance.opacity);

                                // Get shape properties
                                if let Some(shape) = vector_layer.shapes.get(&instance.shape_id) {
                                    info.fill_color = Some(shape.fill_color);
                                    info.stroke_color = Some(shape.stroke_color);
                                    info.stroke_width = shape
                                        .stroke_style
                                        .as_ref()
                                        .map(|s| Some(s.width))
                                        .unwrap_or(Some(1.0));
                                }

                                first = false;
                            } else {
                                // Check if values differ (set to None if mixed)
                                if info.x != Some(instance.transform.x) {
                                    info.x = None;
                                }
                                if info.y != Some(instance.transform.y) {
                                    info.y = None;
                                }
                                if info.rotation != Some(instance.transform.rotation) {
                                    info.rotation = None;
                                }
                                if info.scale_x != Some(instance.transform.scale_x) {
                                    info.scale_x = None;
                                }
                                if info.scale_y != Some(instance.transform.scale_y) {
                                    info.scale_y = None;
                                }
                                if info.skew_x != Some(instance.transform.skew_x) {
                                    info.skew_x = None;
                                }
                                if info.skew_y != Some(instance.transform.skew_y) {
                                    info.skew_y = None;
                                }
                                if info.opacity != Some(instance.opacity) {
                                    info.opacity = None;
                                }

                                // Check shape properties
                                if let Some(shape) = vector_layer.shapes.get(&instance.shape_id) {
                                    // Compare fill colors - set to None if mixed
                                    if let Some(current_fill) = &info.fill_color {
                                        if *current_fill != shape.fill_color {
                                            info.fill_color = None;
                                        }
                                    }
                                    // Compare stroke colors - set to None if mixed
                                    if let Some(current_stroke) = &info.stroke_color {
                                        if *current_stroke != shape.stroke_color {
                                            info.stroke_color = None;
                                        }
                                    }
                                    let stroke_w = shape
                                        .stroke_style
                                        .as_ref()
                                        .map(|s| s.width)
                                        .unwrap_or(1.0);
                                    if info.stroke_width != Some(stroke_w) {
                                        info.stroke_width = None;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        info
    }

    /// Render tool-specific options section
    fn render_tool_section(&mut self, ui: &mut Ui, shared: &mut SharedPaneState) {
        let tool = *shared.selected_tool;

        // Only show tool options for tools that have options
        let has_options = matches!(
            tool,
            Tool::Draw | Tool::Rectangle | Tool::Ellipse | Tool::PaintBucket | Tool::Polygon | Tool::Line
        );

        if !has_options {
            return;
        }

        egui::CollapsingHeader::new("Tool Options")
            .default_open(self.tool_section_open)
            .show(ui, |ui| {
                self.tool_section_open = true;
                ui.add_space(4.0);

                match tool {
                    Tool::Draw => {
                        // Stroke width
                        ui.horizontal(|ui| {
                            ui.label("Stroke Width:");
                            ui.add(DragValue::new(shared.stroke_width).speed(0.1).range(0.1..=100.0));
                        });

                        // Simplify mode
                        ui.horizontal(|ui| {
                            ui.label("Simplify:");
                            egui::ComboBox::from_id_salt("draw_simplify")
                                .selected_text(match shared.draw_simplify_mode {
                                    SimplifyMode::Corners => "Corners",
                                    SimplifyMode::Smooth => "Smooth",
                                    SimplifyMode::Verbatim => "Verbatim",
                                })
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        shared.draw_simplify_mode,
                                        SimplifyMode::Corners,
                                        "Corners",
                                    );
                                    ui.selectable_value(
                                        shared.draw_simplify_mode,
                                        SimplifyMode::Smooth,
                                        "Smooth",
                                    );
                                    ui.selectable_value(
                                        shared.draw_simplify_mode,
                                        SimplifyMode::Verbatim,
                                        "Verbatim",
                                    );
                                });
                        });

                        // Fill shape toggle
                        ui.checkbox(shared.fill_enabled, "Fill Shape");
                    }

                    Tool::Rectangle | Tool::Ellipse => {
                        // Stroke width
                        ui.horizontal(|ui| {
                            ui.label("Stroke Width:");
                            ui.add(DragValue::new(shared.stroke_width).speed(0.1).range(0.1..=100.0));
                        });

                        // Fill shape toggle
                        ui.checkbox(shared.fill_enabled, "Fill Shape");
                    }

                    Tool::PaintBucket => {
                        // Gap tolerance
                        ui.horizontal(|ui| {
                            ui.label("Gap Tolerance:");
                            ui.add(
                                DragValue::new(shared.paint_bucket_gap_tolerance)
                                    .speed(0.1)
                                    .range(0.0..=50.0),
                            );
                        });
                    }

                    Tool::Polygon => {
                        // Number of sides
                        ui.horizontal(|ui| {
                            ui.label("Sides:");
                            let mut sides = *shared.polygon_sides as i32;
                            if ui.add(DragValue::new(&mut sides).range(3..=20)).changed() {
                                *shared.polygon_sides = sides.max(3) as u32;
                            }
                        });

                        // Stroke width
                        ui.horizontal(|ui| {
                            ui.label("Stroke Width:");
                            ui.add(DragValue::new(shared.stroke_width).speed(0.1).range(0.1..=100.0));
                        });

                        // Fill shape toggle
                        ui.checkbox(shared.fill_enabled, "Fill Shape");
                    }

                    Tool::Line => {
                        // Stroke width
                        ui.horizontal(|ui| {
                            ui.label("Stroke Width:");
                            ui.add(DragValue::new(shared.stroke_width).speed(0.1).range(0.1..=100.0));
                        });
                    }

                    _ => {}
                }

                ui.add_space(4.0);
            });
    }

    /// Render transform properties section
    fn render_transform_section(
        &mut self,
        ui: &mut Ui,
        shared: &mut SharedPaneState,
        info: &SelectionInfo,
    ) {
        egui::CollapsingHeader::new("Transform")
            .default_open(self.transform_section_open)
            .show(ui, |ui| {
                self.transform_section_open = true;
                ui.add_space(4.0);

                let layer_id = match info.layer_id {
                    Some(id) => id,
                    None => return,
                };

                // Position X
                self.render_transform_field(
                    ui,
                    "X:",
                    info.x,
                    1.0,
                    f64::NEG_INFINITY..=f64::INFINITY,
                    |value| InstancePropertyChange::X(value),
                    layer_id,
                    &info.instance_ids,
                    shared,
                );

                // Position Y
                self.render_transform_field(
                    ui,
                    "Y:",
                    info.y,
                    1.0,
                    f64::NEG_INFINITY..=f64::INFINITY,
                    |value| InstancePropertyChange::Y(value),
                    layer_id,
                    &info.instance_ids,
                    shared,
                );

                ui.add_space(4.0);

                // Rotation
                self.render_transform_field(
                    ui,
                    "Rotation:",
                    info.rotation,
                    1.0,
                    -360.0..=360.0,
                    |value| InstancePropertyChange::Rotation(value),
                    layer_id,
                    &info.instance_ids,
                    shared,
                );

                ui.add_space(4.0);

                // Scale X
                self.render_transform_field(
                    ui,
                    "Scale X:",
                    info.scale_x,
                    0.01,
                    0.01..=100.0,
                    |value| InstancePropertyChange::ScaleX(value),
                    layer_id,
                    &info.instance_ids,
                    shared,
                );

                // Scale Y
                self.render_transform_field(
                    ui,
                    "Scale Y:",
                    info.scale_y,
                    0.01,
                    0.01..=100.0,
                    |value| InstancePropertyChange::ScaleY(value),
                    layer_id,
                    &info.instance_ids,
                    shared,
                );

                ui.add_space(4.0);

                // Skew X
                self.render_transform_field(
                    ui,
                    "Skew X:",
                    info.skew_x,
                    1.0,
                    -89.0..=89.0,
                    |value| InstancePropertyChange::SkewX(value),
                    layer_id,
                    &info.instance_ids,
                    shared,
                );

                // Skew Y
                self.render_transform_field(
                    ui,
                    "Skew Y:",
                    info.skew_y,
                    1.0,
                    -89.0..=89.0,
                    |value| InstancePropertyChange::SkewY(value),
                    layer_id,
                    &info.instance_ids,
                    shared,
                );

                ui.add_space(4.0);

                // Opacity
                self.render_transform_field(
                    ui,
                    "Opacity:",
                    info.opacity,
                    0.01,
                    0.0..=1.0,
                    |value| InstancePropertyChange::Opacity(value),
                    layer_id,
                    &info.instance_ids,
                    shared,
                );

                ui.add_space(4.0);
            });
    }

    /// Render a single transform property field with drag-to-adjust
    fn render_transform_field<F>(
        &self,
        ui: &mut Ui,
        label: &str,
        value: Option<f64>,
        speed: f64,
        range: std::ops::RangeInclusive<f64>,
        make_change: F,
        layer_id: Uuid,
        instance_ids: &[Uuid],
        shared: &mut SharedPaneState,
    ) where
        F: Fn(f64) -> InstancePropertyChange,
    {
        ui.horizontal(|ui| {
            // Label with drag sense for drag-to-adjust
            let label_response = ui.add(egui::Label::new(label).sense(Sense::drag()));

            match value {
                Some(mut v) => {
                    // Handle drag on label
                    if label_response.dragged() {
                        let delta = label_response.drag_delta().x as f64 * speed;
                        v = (v + delta).clamp(*range.start(), *range.end());

                        // Create action for each selected instance
                        for instance_id in instance_ids {
                            let action = SetInstancePropertiesAction::new(
                                layer_id,
                                *instance_id,
                                make_change(v),
                            );
                            shared.pending_actions.push(Box::new(action));
                        }
                    }

                    // DragValue widget
                    let response = ui.add(
                        DragValue::new(&mut v)
                            .speed(speed)
                            .range(range.clone()),
                    );

                    if response.changed() {
                        // Create action for each selected instance
                        for instance_id in instance_ids {
                            let action = SetInstancePropertiesAction::new(
                                layer_id,
                                *instance_id,
                                make_change(v),
                            );
                            shared.pending_actions.push(Box::new(action));
                        }
                    }
                }
                None => {
                    // Mixed values - show placeholder
                    ui.label("--");
                }
            }
        });
    }

    /// Render shape properties section (fill/stroke)
    fn render_shape_section(
        &mut self,
        ui: &mut Ui,
        shared: &mut SharedPaneState,
        info: &SelectionInfo,
    ) {
        egui::CollapsingHeader::new("Shape")
            .default_open(self.shape_section_open)
            .show(ui, |ui| {
                self.shape_section_open = true;
                ui.add_space(4.0);

                let layer_id = match info.layer_id {
                    Some(id) => id,
                    None => return,
                };

                // Fill color
                ui.horizontal(|ui| {
                    ui.label("Fill:");
                    match info.fill_color {
                        Some(Some(color)) => {
                            let mut egui_color = egui::Color32::from_rgba_unmultiplied(
                                color.r, color.g, color.b, color.a,
                            );

                            if ui.color_edit_button_srgba(&mut egui_color).changed() {
                                let new_color = Some(ShapeColor::new(
                                    egui_color.r(),
                                    egui_color.g(),
                                    egui_color.b(),
                                    egui_color.a(),
                                ));

                                // Create action for each selected shape
                                for shape_id in &info.shape_ids {
                                    let action = SetShapePropertiesAction::set_fill_color(
                                        layer_id,
                                        *shape_id,
                                        new_color,
                                    );
                                    shared.pending_actions.push(Box::new(action));
                                }
                            }
                        }
                        Some(None) => {
                            if ui.button("Add Fill").clicked() {
                                // Add default black fill
                                let default_fill = Some(ShapeColor::rgb(0, 0, 0));
                                for shape_id in &info.shape_ids {
                                    let action = SetShapePropertiesAction::set_fill_color(
                                        layer_id,
                                        *shape_id,
                                        default_fill,
                                    );
                                    shared.pending_actions.push(Box::new(action));
                                }
                            }
                        }
                        None => {
                            ui.label("--");
                        }
                    }
                });

                // Stroke color
                ui.horizontal(|ui| {
                    ui.label("Stroke:");
                    match info.stroke_color {
                        Some(Some(color)) => {
                            let mut egui_color = egui::Color32::from_rgba_unmultiplied(
                                color.r, color.g, color.b, color.a,
                            );

                            if ui.color_edit_button_srgba(&mut egui_color).changed() {
                                let new_color = Some(ShapeColor::new(
                                    egui_color.r(),
                                    egui_color.g(),
                                    egui_color.b(),
                                    egui_color.a(),
                                ));

                                // Create action for each selected shape
                                for shape_id in &info.shape_ids {
                                    let action = SetShapePropertiesAction::set_stroke_color(
                                        layer_id,
                                        *shape_id,
                                        new_color,
                                    );
                                    shared.pending_actions.push(Box::new(action));
                                }
                            }
                        }
                        Some(None) => {
                            if ui.button("Add Stroke").clicked() {
                                // Add default black stroke
                                let default_stroke = Some(ShapeColor::rgb(0, 0, 0));
                                for shape_id in &info.shape_ids {
                                    let action = SetShapePropertiesAction::set_stroke_color(
                                        layer_id,
                                        *shape_id,
                                        default_stroke,
                                    );
                                    shared.pending_actions.push(Box::new(action));
                                }
                            }
                        }
                        None => {
                            ui.label("--");
                        }
                    }
                });

                // Stroke width
                ui.horizontal(|ui| {
                    ui.label("Stroke Width:");
                    match info.stroke_width {
                        Some(mut width) => {
                            let response = ui.add(
                                DragValue::new(&mut width)
                                    .speed(0.1)
                                    .range(0.1..=100.0),
                            );

                            if response.changed() {
                                for shape_id in &info.shape_ids {
                                    let action = SetShapePropertiesAction::set_stroke_width(
                                        layer_id,
                                        *shape_id,
                                        width,
                                    );
                                    shared.pending_actions.push(Box::new(action));
                                }
                            }
                        }
                        None => {
                            ui.label("--");
                        }
                    }
                });

                ui.add_space(4.0);
            });
    }

    /// Render document settings section (shown when nothing is selected)
    fn render_document_section(&self, ui: &mut Ui, shared: &mut SharedPaneState) {
        egui::CollapsingHeader::new("Document")
            .default_open(true)
            .show(ui, |ui| {
                ui.add_space(4.0);

                let document = shared.action_executor.document();

                // Get current values for editing
                let mut width = document.width;
                let mut height = document.height;
                let mut duration = document.duration;
                let mut framerate = document.framerate;
                let layer_count = document.root.children.len();

                // Canvas width
                ui.horizontal(|ui| {
                    ui.label("Width:");
                    if ui
                        .add(DragValue::new(&mut width).speed(1.0).range(1.0..=10000.0))
                        .changed()
                    {
                        let action = SetDocumentPropertiesAction::set_width(width);
                        shared.pending_actions.push(Box::new(action));
                    }
                });

                // Canvas height
                ui.horizontal(|ui| {
                    ui.label("Height:");
                    if ui
                        .add(DragValue::new(&mut height).speed(1.0).range(1.0..=10000.0))
                        .changed()
                    {
                        let action = SetDocumentPropertiesAction::set_height(height);
                        shared.pending_actions.push(Box::new(action));
                    }
                });

                // Duration
                ui.horizontal(|ui| {
                    ui.label("Duration:");
                    if ui
                        .add(
                            DragValue::new(&mut duration)
                                .speed(0.1)
                                .range(0.1..=3600.0)
                                .suffix("s"),
                        )
                        .changed()
                    {
                        let action = SetDocumentPropertiesAction::set_duration(duration);
                        shared.pending_actions.push(Box::new(action));
                    }
                });

                // Framerate
                ui.horizontal(|ui| {
                    ui.label("Framerate:");
                    if ui
                        .add(
                            DragValue::new(&mut framerate)
                                .speed(1.0)
                                .range(1.0..=120.0)
                                .suffix(" fps"),
                        )
                        .changed()
                    {
                        let action = SetDocumentPropertiesAction::set_framerate(framerate);
                        shared.pending_actions.push(Box::new(action));
                    }
                });

                // Layer count (read-only)
                ui.horizontal(|ui| {
                    ui.label("Layers:");
                    ui.label(format!("{}", layer_count));
                });

                ui.add_space(4.0);
            });
    }
}

impl PaneRenderer for InfopanelPane {
    fn render_content(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        _path: &NodePath,
        shared: &mut SharedPaneState,
    ) {
        // Background
        ui.painter().rect_filled(
            rect,
            0.0,
            egui::Color32::from_rgb(30, 35, 40),
        );

        // Create scrollable area for content
        let content_rect = rect.shrink(8.0);
        let mut content_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(content_rect)
                .layout(egui::Layout::top_down(egui::Align::LEFT)),
        );

        egui::ScrollArea::vertical()
            .id_salt("infopanel_scroll")
            .show(&mut content_ui, |ui| {
                ui.set_min_width(content_rect.width() - 16.0);

                // 1. Tool options section (always shown if tool has options)
                self.render_tool_section(ui, shared);

                // 2. Gather selection info
                let info = self.gather_selection_info(shared);

                // 3. Transform section (if shapes selected)
                if info.shape_count > 0 {
                    self.render_transform_section(ui, shared, &info);
                }

                // 4. Shape properties section (if shapes selected)
                if info.shape_count > 0 {
                    self.render_shape_section(ui, shared, &info);
                }

                // 5. Document settings (if nothing selected)
                if info.is_empty {
                    self.render_document_section(ui, shared);
                }

                // Show selection count at bottom
                if info.shape_count > 0 {
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(4.0);
                    ui.label(format!(
                        "{} object{} selected",
                        info.shape_count,
                        if info.shape_count == 1 { "" } else { "s" }
                    ));
                }
            });
    }

    fn name(&self) -> &str {
        "Info Panel"
    }
}

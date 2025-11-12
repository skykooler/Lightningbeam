use eframe::egui;
use lightningbeam_core::layout::{LayoutDefinition, LayoutNode, PaneType};

fn main() -> eframe::Result {
    println!("🚀 Starting Lightningbeam Editor...");

    // Load layouts from JSON
    let layouts = load_layouts();
    println!("✅ Loaded {} layouts", layouts.len());
    for layout in &layouts {
        println!("   - {}: {}", layout.name, layout.description);
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1920.0, 1080.0])
            .with_title("Lightningbeam Editor"),
        ..Default::default()
    };

    eframe::run_native(
        "Lightningbeam Editor",
        options,
        Box::new(move |_cc| Ok(Box::new(EditorApp::new(layouts)))),
    )
}

fn load_layouts() -> Vec<LayoutDefinition> {
    let json = include_str!("../assets/layouts.json");
    serde_json::from_str(json).expect("Failed to parse layouts.json")
}

struct EditorApp {
    layouts: Vec<LayoutDefinition>,
    current_layout_index: usize,
}

impl EditorApp {
    fn new(layouts: Vec<LayoutDefinition>) -> Self {
        Self {
            layouts,
            current_layout_index: 0,
        }
    }

    fn current_layout(&self) -> &LayoutDefinition {
        &self.layouts[self.current_layout_index]
    }
}

impl eframe::App for EditorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Top menu bar
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("Layout", |ui| {
                    for (i, layout) in self.layouts.iter().enumerate() {
                        if ui
                            .selectable_label(i == self.current_layout_index, &layout.name)
                            .clicked()
                        {
                            self.current_layout_index = i;
                            ui.close_menu();
                        }
                    }
                });

                ui.separator();
                ui.label(format!("Current: {}", self.current_layout().name));
            });
        });

        // Main pane area
        egui::CentralPanel::default().show(ctx, |ui| {
            let available_rect = ui.available_rect_before_wrap();
            render_layout_node(ui, &self.current_layout().layout, available_rect);
        });
    }
}

/// Recursively render a layout node
fn render_layout_node(ui: &mut egui::Ui, node: &LayoutNode, rect: egui::Rect) {
    match node {
        LayoutNode::Pane { name } => {
            render_pane(ui, name, rect);
        }
        LayoutNode::HorizontalGrid { percent, children } => {
            // Split horizontally (left | right)
            let split_x = rect.left() + (rect.width() * percent / 100.0);

            let left_rect = egui::Rect::from_min_max(
                rect.min,
                egui::pos2(split_x, rect.max.y),
            );

            let right_rect = egui::Rect::from_min_max(
                egui::pos2(split_x, rect.min.y),
                rect.max,
            );

            // Render children
            render_layout_node(ui, &children[0], left_rect);
            render_layout_node(ui, &children[1], right_rect);

            // Draw divider
            ui.painter().vline(
                split_x,
                rect.y_range(),
                egui::Stroke::new(2.0, egui::Color32::from_gray(60)),
            );
        }
        LayoutNode::VerticalGrid { percent, children } => {
            // Split vertically (top / bottom)
            let split_y = rect.top() + (rect.height() * percent / 100.0);

            let top_rect = egui::Rect::from_min_max(
                rect.min,
                egui::pos2(rect.max.x, split_y),
            );

            let bottom_rect = egui::Rect::from_min_max(
                egui::pos2(rect.min.x, split_y),
                rect.max,
            );

            // Render children
            render_layout_node(ui, &children[0], top_rect);
            render_layout_node(ui, &children[1], bottom_rect);

            // Draw divider
            ui.painter().hline(
                rect.x_range(),
                split_y,
                egui::Stroke::new(2.0, egui::Color32::from_gray(60)),
            );
        }
    }
}

/// Render a single pane with its content
fn render_pane(ui: &mut egui::Ui, pane_name: &str, rect: egui::Rect) {
    let pane_type = PaneType::from_name(pane_name);

    // Get color for pane type
    let bg_color = if let Some(pane_type) = pane_type {
        pane_color(pane_type)
    } else {
        egui::Color32::from_rgb(40, 40, 40)
    };

    // Draw background
    ui.painter().rect_filled(rect, 0.0, bg_color);

    // Draw border
    ui.painter().rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(1.0, egui::Color32::from_gray(80)),
    );

    // Draw pane label
    let text = if let Some(pane_type) = pane_type {
        pane_type.display_name()
    } else {
        pane_name
    };

    let text_pos = rect.center() - egui::vec2(40.0, 10.0);
    ui.painter().text(
        text_pos,
        egui::Align2::LEFT_CENTER,
        text,
        egui::FontId::proportional(16.0),
        egui::Color32::WHITE,
    );

    // Draw pane name in corner
    let corner_pos = rect.min + egui::vec2(8.0, 8.0);
    ui.painter().text(
        corner_pos,
        egui::Align2::LEFT_TOP,
        format!("[{}]", pane_name),
        egui::FontId::monospace(10.0),
        egui::Color32::from_gray(150),
    );
}

/// Get a color for each pane type for visualization
fn pane_color(pane_type: PaneType) -> egui::Color32 {
    match pane_type {
        PaneType::Stage => egui::Color32::from_rgb(30, 40, 50),
        PaneType::Timeline => egui::Color32::from_rgb(40, 30, 50),
        PaneType::TimelineV2 => egui::Color32::from_rgb(45, 35, 55),
        PaneType::Toolbar => egui::Color32::from_rgb(50, 40, 30),
        PaneType::Infopanel => egui::Color32::from_rgb(30, 50, 40),
        PaneType::Outliner => egui::Color32::from_rgb(40, 50, 30),
        PaneType::Piano => egui::Color32::from_rgb(50, 30, 40),
        PaneType::PianoRoll => egui::Color32::from_rgb(55, 35, 45),
        PaneType::NodeEditor => egui::Color32::from_rgb(30, 45, 50),
        PaneType::PresetBrowser => egui::Color32::from_rgb(50, 45, 30),
    }
}

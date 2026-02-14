/// Shader Editor pane - WGSL shader code editor with syntax highlighting
///
/// Provides a code editor for creating and editing custom effect shaders.
/// Features:
/// - Syntax highlighting for WGSL
/// - Line numbers
/// - Basic validation feedback
/// - Template shader insertion

use eframe::egui::{self, Ui};
use egui_code_editor::{CodeEditor, ColorTheme, Syntax};
use lightningbeam_core::effect::{EffectCategory, EffectDefinition};
use lightningbeam_core::effect_registry::EffectRegistry;
use uuid::Uuid;
use super::{NodePath, PaneRenderer, SharedPaneState};

/// Result from the unsaved changes dialog
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnsavedDialogResult {
    Cancel,
    Discard,
    SaveAndContinue,
}

/// Custom syntax definition for WGSL (WebGPU Shading Language)
fn wgsl_syntax() -> Syntax {
    Syntax {
        language: "WGSL",
        case_sensitive: true,
        comment: "//",
        comment_multiline: ["/*", "*/"],
        hyperlinks: std::collections::BTreeSet::new(),
        keywords: std::collections::BTreeSet::from([
            // Control flow
            "if", "else", "for", "while", "loop", "break", "continue", "return",
            "switch", "case", "default", "discard",
            // Declarations
            "fn", "let", "var", "const", "struct", "alias", "type",
            // Storage classes and access modes
            "function", "private", "workgroup", "uniform", "storage",
            "read", "write", "read_write",
            // Shader stages
            "vertex", "fragment", "compute",
            // Attributes
            "location", "builtin", "group", "binding",
            // Built-in values
            "position", "vertex_index", "instance_index", "front_facing",
            "frag_depth", "local_invocation_id", "local_invocation_index",
            "global_invocation_id", "workgroup_id", "num_workgroups",
            "sample_index", "sample_mask",
        ]),
        types: std::collections::BTreeSet::from([
            // Scalar types
            "bool", "i32", "u32", "f32", "f16",
            // Vector types
            "vec2", "vec3", "vec4",
            "vec2i", "vec3i", "vec4i",
            "vec2u", "vec3u", "vec4u",
            "vec2f", "vec3f", "vec4f",
            "vec2h", "vec3h", "vec4h",
            // Matrix types
            "mat2x2", "mat2x3", "mat2x4",
            "mat3x2", "mat3x3", "mat3x4",
            "mat4x2", "mat4x3", "mat4x4",
            "mat2x2f", "mat3x3f", "mat4x4f",
            // Texture types
            "texture_1d", "texture_2d", "texture_2d_array", "texture_3d",
            "texture_cube", "texture_cube_array", "texture_multisampled_2d",
            "texture_storage_1d", "texture_storage_2d", "texture_storage_2d_array",
            "texture_storage_3d", "texture_depth_2d", "texture_depth_2d_array",
            "texture_depth_cube", "texture_depth_cube_array", "texture_depth_multisampled_2d",
            // Sampler types
            "sampler", "sampler_comparison",
            // Array and pointer
            "array", "ptr",
        ]),
        special: std::collections::BTreeSet::from([
            // Built-in functions (subset)
            "abs", "acos", "all", "any", "asin", "atan", "atan2",
            "ceil", "clamp", "cos", "cosh", "cross",
            "degrees", "determinant", "distance", "dot",
            "exp", "exp2", "faceForward", "floor", "fma", "fract",
            "length", "log", "log2",
            "max", "min", "mix", "modf", "normalize",
            "pow", "radians", "reflect", "refract", "round",
            "saturate", "sign", "sin", "sinh", "smoothstep", "sqrt", "step",
            "tan", "tanh", "transpose", "trunc",
            // Texture functions
            "textureSample", "textureSampleLevel", "textureSampleBias",
            "textureSampleGrad", "textureSampleCompare", "textureLoad",
            "textureStore", "textureDimensions", "textureNumLayers",
            "textureNumLevels", "textureNumSamples",
            // Atomic functions
            "atomicLoad", "atomicStore", "atomicAdd", "atomicSub",
            "atomicMax", "atomicMin", "atomicAnd", "atomicOr", "atomicXor",
            "atomicExchange", "atomicCompareExchangeWeak",
            // Data packing
            "pack4x8snorm", "pack4x8unorm", "pack2x16snorm", "pack2x16unorm",
            "unpack4x8snorm", "unpack4x8unorm", "unpack2x16snorm", "unpack2x16unorm",
            // Synchronization
            "storageBarrier", "workgroupBarrier", "workgroupUniformLoad",
            // Type constructors
            "select", "bitcast",
        ]),
    }
}

/// Default WGSL shader template for custom effects
const DEFAULT_SHADER_TEMPLATE: &str = r#"// Custom Effect Shader
// Input: source_tex (the layer content)
// Output: vec4<f32> color at each pixel

@group(0) @binding(0) var source_tex: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// Fullscreen triangle strip
@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    let x = f32((vertex_index & 1u) << 1u);
    let y = f32(vertex_index & 2u);
    out.position = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    out.uv = vec2<f32>(x, y);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Sample the source texture
    let color = textureSample(source_tex, source_sampler, in.uv);

    // Your effect code here - modify 'color' as desired
    // Example: Return the color unchanged (passthrough)
    return color;
}
"#;

/// Grayscale effect shader template
const GRAYSCALE_TEMPLATE: &str = r#"// Grayscale Effect
// Converts the image to grayscale using luminance weights

@group(0) @binding(0) var source_tex: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    let x = f32((vertex_index & 1u) << 1u);
    let y = f32(vertex_index & 2u);
    out.position = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    out.uv = vec2<f32>(x, y);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(source_tex, source_sampler, in.uv);

    // ITU-R BT.709 luminance coefficients
    let luminance = dot(color.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));

    return vec4<f32>(luminance, luminance, luminance, color.a);
}
"#;

/// Vignette effect shader template
const VIGNETTE_TEMPLATE: &str = r#"// Vignette Effect
// Darkens the edges of the image

@group(0) @binding(0) var source_tex: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    let x = f32((vertex_index & 1u) << 1u);
    let y = f32(vertex_index & 2u);
    out.position = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    out.uv = vec2<f32>(x, y);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(source_tex, source_sampler, in.uv);

    // Calculate distance from center (0.5, 0.5)
    let center = vec2<f32>(0.5, 0.5);
    let dist = distance(in.uv, center);

    // Vignette parameters
    let radius = 0.7;  // Inner radius (no darkening)
    let softness = 0.4; // Transition softness

    // Calculate vignette factor
    let vignette = smoothstep(radius + softness, radius, dist);

    return vec4<f32>(color.rgb * vignette, color.a);
}
"#;

/// Shader Editor pane state
pub struct ShaderEditorPane {
    /// The shader source code being edited
    shader_code: String,
    /// Whether to show the template selector
    #[allow(dead_code)]
    show_templates: bool,
    /// Error message from last compilation attempt (if any)
    compile_error: Option<String>,
    /// Name for the shader/effect
    shader_name: String,
    /// ID of effect being edited (None = new effect)
    editing_effect_id: Option<Uuid>,
    /// Original code when effect was loaded (for dirty checking)
    original_code: Option<String>,
    /// Original name when effect was loaded (for dirty checking)
    original_name: Option<String>,
    /// Effect awaiting confirmation to load (when there are unsaved changes)
    pending_load_effect: Option<EffectDefinition>,
    /// Whether to show the unsaved changes confirmation dialog
    show_unsaved_dialog: bool,
}

impl ShaderEditorPane {
    pub fn new() -> Self {
        Self {
            shader_code: DEFAULT_SHADER_TEMPLATE.to_string(),
            show_templates: false,
            compile_error: None,
            shader_name: "Custom Effect".to_string(),
            editing_effect_id: None,
            original_code: None,
            original_name: None,
            pending_load_effect: None,
            show_unsaved_dialog: false,
        }
    }

    /// Check if there are unsaved changes
    pub fn has_unsaved_changes(&self) -> bool {
        match (&self.original_code, &self.original_name) {
            (Some(orig_code), Some(orig_name)) => {
                self.shader_code != *orig_code || self.shader_name != *orig_name
            }
            // If no original, check if we've modified from default
            (None, None) => {
                self.shader_code != DEFAULT_SHADER_TEMPLATE || self.shader_name != "Custom Effect"
            }
            _ => true, // Inconsistent state, assume dirty
        }
    }

    /// Load an effect into the editor
    pub fn load_effect(&mut self, effect: &EffectDefinition) {
        self.shader_name = effect.name.clone();
        self.shader_code = effect.shader_code.clone();
        // For built-in effects, don't set editing_effect_id (editing creates a copy)
        if effect.category == EffectCategory::Custom {
            self.editing_effect_id = Some(effect.id);
        } else {
            self.editing_effect_id = None;
        }
        self.original_code = Some(effect.shader_code.clone());
        self.original_name = Some(effect.name.clone());
        self.compile_error = None;
    }

    /// Reset to a new blank effect
    pub fn new_effect(&mut self) {
        self.shader_name = "Custom Effect".to_string();
        self.shader_code = DEFAULT_SHADER_TEMPLATE.to_string();
        self.editing_effect_id = None;
        self.original_code = None;
        self.original_name = None;
        self.compile_error = None;
    }

    /// Mark the current state as saved
    pub fn mark_saved(&mut self, effect_id: Uuid) {
        self.editing_effect_id = Some(effect_id);
        self.original_code = Some(self.shader_code.clone());
        self.original_name = Some(self.shader_name.clone());
    }

    /// Look up an effect by ID (checks document first, then built-in registry)
    fn lookup_effect(
        &self,
        effect_id: Uuid,
        document: &lightningbeam_core::document::Document,
    ) -> Option<EffectDefinition> {
        // First check custom effects in document
        if let Some(def) = document.effect_definitions.get(&effect_id) {
            return Some(def.clone());
        }
        // Then check built-in effects
        EffectRegistry::get_by_id(&effect_id)
    }

    /// Render the unsaved changes confirmation dialog
    fn render_unsaved_dialog(&mut self, ui: &mut egui::Ui) -> Option<UnsavedDialogResult> {
        let mut result = None;

        if self.show_unsaved_dialog {
            let window_id = egui::Id::new("shader_unsaved_dialog");

            egui::Window::new("Unsaved Changes")
                .id(window_id)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ui.ctx(), |ui| {
                    ui.set_min_width(300.0);

                    ui.label("You have unsaved changes to this shader.");
                    ui.label("What would you like to do?");
                    ui.add_space(12.0);

                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            result = Some(UnsavedDialogResult::Cancel);
                        }
                        if ui.button("Discard Changes").clicked() {
                            result = Some(UnsavedDialogResult::Discard);
                        }
                        if ui.button("Save & Continue").clicked() {
                            result = Some(UnsavedDialogResult::SaveAndContinue);
                        }
                    });
                });
        }

        result
    }

    /// Render the toolbar with template selection and actions
    /// Returns true if Save was clicked
    fn render_toolbar(&mut self, ui: &mut Ui, _path: &NodePath) -> bool {
        let mut save_clicked = false;
        ui.horizontal(|ui| {
            // New button
            if ui.button("New").clicked() {
                // TODO: Check for unsaved changes first
                self.new_effect();
            }

            ui.separator();

            // Shader name input
            ui.label("Name:");
            ui.add(egui::TextEdit::singleline(&mut self.shader_name).desired_width(150.0));

            ui.separator();

            // Template dropdown
            egui::ComboBox::from_label("Template")
                .selected_text("Insert Template")
                .show_ui(ui, |ui| {
                    if ui.selectable_label(false, "Basic (Passthrough)").clicked() {
                        self.shader_code = DEFAULT_SHADER_TEMPLATE.to_string();
                    }
                    if ui.selectable_label(false, "Grayscale").clicked() {
                        self.shader_code = GRAYSCALE_TEMPLATE.to_string();
                    }
                    if ui.selectable_label(false, "Vignette").clicked() {
                        self.shader_code = VIGNETTE_TEMPLATE.to_string();
                    }
                });

            ui.separator();

            // Compile button (placeholder for now)
            if ui.button("Validate").clicked() {
                // TODO: Integrate with wgpu shader validation
                // For now, just clear any previous error
                self.compile_error = None;
            }

            // Save button
            if ui.button("Save").clicked() {
                save_clicked = true;
            }

            // Show dirty indicator
            if self.has_unsaved_changes() {
                ui.label(egui::RichText::new("*").color(egui::Color32::YELLOW));
            }

            // Show editing mode
            if let Some(_) = self.editing_effect_id {
                ui.label(egui::RichText::new("(Editing)").weak());
            } else {
                ui.label(egui::RichText::new("(New)").weak());
            }
        });
        save_clicked
    }

    /// Render the error panel if there's a compile error
    fn render_error_panel(&self, ui: &mut Ui) {
        if let Some(error) = &self.compile_error {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Error:").color(egui::Color32::RED));
                ui.label(error);
            });
            ui.separator();
        }
    }
}

impl PaneRenderer for ShaderEditorPane {
    fn render_content(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        path: &NodePath,
        shared: &mut SharedPaneState,
    ) {
        // Handle effect loading request from asset library
        if let Some(effect_id) = shared.effect_to_load.take() {
            // Look up the effect
            if let Some(effect) = self.lookup_effect(effect_id, shared.action_executor.document()) {
                if self.has_unsaved_changes() {
                    // Store effect to load and show dialog
                    self.pending_load_effect = Some(effect);
                    self.show_unsaved_dialog = true;
                } else {
                    // No unsaved changes, load immediately
                    self.load_effect(&effect);
                }
            }
        }

        // Handle unsaved changes dialog
        if let Some(result) = self.render_unsaved_dialog(ui) {
            match result {
                UnsavedDialogResult::Cancel => {
                    // Cancel the load, keep current state
                    self.pending_load_effect = None;
                    self.show_unsaved_dialog = false;
                }
                UnsavedDialogResult::Discard => {
                    // Discard changes and load the new effect
                    if let Some(effect) = self.pending_load_effect.take() {
                        self.load_effect(&effect);
                    }
                    self.show_unsaved_dialog = false;
                }
                UnsavedDialogResult::SaveAndContinue => {
                    // Save current work first
                    if !self.shader_name.trim().is_empty() {
                        let effect = if let Some(existing_id) = self.editing_effect_id {
                            EffectDefinition::with_id(
                                existing_id,
                                self.shader_name.clone(),
                                EffectCategory::Custom,
                                self.shader_code.clone(),
                                vec![],
                            )
                        } else {
                            EffectDefinition::new(
                                self.shader_name.clone(),
                                EffectCategory::Custom,
                                self.shader_code.clone(),
                                vec![],
                            )
                        };
                        let effect_id = effect.id;
                        shared.action_executor.document_mut().add_effect_definition(effect);
                        self.mark_saved(effect_id);
                        // Invalidate thumbnail so it regenerates with new shader
                        shared.effect_thumbnails_to_invalidate.push(effect_id);
                    }
                    // Then load the new effect
                    if let Some(effect) = self.pending_load_effect.take() {
                        self.load_effect(&effect);
                    }
                    self.show_unsaved_dialog = false;
                }
            }
        }

        // Background
        ui.painter().rect_filled(
            rect,
            0.0,
            egui::Color32::from_rgb(25, 25, 30),
        );

        // Create content area
        let content_rect = rect.shrink(8.0);
        let mut content_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(content_rect)
                .layout(egui::Layout::top_down(egui::Align::LEFT)),
        );

        content_ui.set_min_width(content_rect.width() - 16.0);

        // Toolbar
        let save_clicked = self.render_toolbar(&mut content_ui, path);
        content_ui.add_space(4.0);
        content_ui.separator();
        content_ui.add_space(4.0);

        // Handle save action
        if save_clicked {
            if self.shader_name.trim().is_empty() {
                self.compile_error = Some("Name cannot be empty".to_string());
            } else {
                // Create or update EffectDefinition
                let effect = if let Some(existing_id) = self.editing_effect_id {
                    // Update existing custom effect
                    EffectDefinition::with_id(
                        existing_id,
                        self.shader_name.clone(),
                        EffectCategory::Custom,
                        self.shader_code.clone(),
                        vec![], // No parameters for now
                    )
                } else {
                    // Create new custom effect
                    EffectDefinition::new(
                        self.shader_name.clone(),
                        EffectCategory::Custom,
                        self.shader_code.clone(),
                        vec![], // No parameters for now
                    )
                };

                let effect_id = effect.id;
                shared.action_executor.document_mut().add_effect_definition(effect);
                self.mark_saved(effect_id);
                // Invalidate thumbnail so it regenerates with new shader
                shared.effect_thumbnails_to_invalidate.push(effect_id);
                self.compile_error = None;
            }
        }

        // Error panel
        self.render_error_panel(&mut content_ui);

        // Calculate remaining height for the code editor
        let remaining_rect = content_ui.available_rect_before_wrap();

        // Code editor
        egui::ScrollArea::both()
            .id_salt(("shader_editor_scroll", path))
            .auto_shrink([false, false])
            .show(&mut content_ui, |ui| {
                ui.set_min_size(remaining_rect.size());

                CodeEditor::default()
                    .id_source("shader_code_editor")
                    .with_rows(50)
                    .with_fontsize(13.0)
                    .with_theme(ColorTheme::GRUVBOX_DARK)
                    .with_syntax(wgsl_syntax())
                    .with_numlines(true)
                    .show(ui, &mut self.shader_code);
            });
    }

    fn name(&self) -> &str {
        "Shader Editor"
    }
}

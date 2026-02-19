/// Script Editor pane - unified code editor for WGSL shaders and BeamDSP scripts
///
/// Supports multiple editor modes:
/// - Shader: WGSL shader code for custom visual effects
/// - BeamDSP: Audio DSP scripts for scriptable audio nodes
///
/// Both modes use the same save/load workflow through the asset library.

use eframe::egui::{self, Ui};
use egui_code_editor::{CodeEditor, ColorTheme, Syntax};
use lightningbeam_core::effect::{EffectCategory, EffectDefinition};
use lightningbeam_core::script::ScriptDefinition;
use uuid::Uuid;
use super::{NodePath, PaneRenderer, SharedPaneState};

/// Editor mode determines syntax, templates, and compile/save behavior
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorMode {
    /// WGSL shader for visual effects
    Shader,
    /// BeamDSP script for audio processing nodes
    BeamDSP,
}

/// Result from the unsaved changes dialog
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnsavedDialogResult {
    Cancel,
    Discard,
    SaveAndContinue,
}

// ── Syntax definitions ──────────────────────────────────────────────

fn wgsl_syntax() -> Syntax {
    Syntax {
        language: "WGSL",
        case_sensitive: true,
        comment: "//",
        comment_multiline: ["/*", "*/"],
        hyperlinks: std::collections::BTreeSet::new(),
        keywords: std::collections::BTreeSet::from([
            "if", "else", "for", "while", "loop", "break", "continue", "return",
            "switch", "case", "default", "discard",
            "fn", "let", "var", "const", "struct", "alias", "type",
            "function", "private", "workgroup", "uniform", "storage",
            "read", "write", "read_write",
            "vertex", "fragment", "compute",
            "location", "builtin", "group", "binding",
            "position", "vertex_index", "instance_index", "front_facing",
            "frag_depth", "local_invocation_id", "local_invocation_index",
            "global_invocation_id", "workgroup_id", "num_workgroups",
            "sample_index", "sample_mask",
        ]),
        types: std::collections::BTreeSet::from([
            "bool", "i32", "u32", "f32", "f16",
            "vec2", "vec3", "vec4",
            "vec2i", "vec3i", "vec4i", "vec2u", "vec3u", "vec4u",
            "vec2f", "vec3f", "vec4f", "vec2h", "vec3h", "vec4h",
            "mat2x2", "mat2x3", "mat2x4", "mat3x2", "mat3x3", "mat3x4",
            "mat4x2", "mat4x3", "mat4x4", "mat2x2f", "mat3x3f", "mat4x4f",
            "texture_1d", "texture_2d", "texture_2d_array", "texture_3d",
            "texture_cube", "texture_cube_array", "texture_multisampled_2d",
            "texture_storage_1d", "texture_storage_2d", "texture_storage_2d_array",
            "texture_storage_3d", "texture_depth_2d", "texture_depth_2d_array",
            "texture_depth_cube", "texture_depth_cube_array",
            "texture_depth_multisampled_2d",
            "sampler", "sampler_comparison",
            "array", "ptr",
        ]),
        special: std::collections::BTreeSet::from([
            "abs", "acos", "all", "any", "asin", "atan", "atan2",
            "ceil", "clamp", "cos", "cosh", "cross",
            "degrees", "determinant", "distance", "dot",
            "exp", "exp2", "faceForward", "floor", "fma", "fract",
            "length", "log", "log2",
            "max", "min", "mix", "modf", "normalize",
            "pow", "radians", "reflect", "refract", "round",
            "saturate", "sign", "sin", "sinh", "smoothstep", "sqrt", "step",
            "tan", "tanh", "transpose", "trunc",
            "textureSample", "textureSampleLevel", "textureSampleBias",
            "textureSampleGrad", "textureSampleCompare", "textureLoad",
            "textureStore", "textureDimensions", "textureNumLayers",
            "textureNumLevels", "textureNumSamples",
            "atomicLoad", "atomicStore", "atomicAdd", "atomicSub",
            "atomicMax", "atomicMin", "atomicAnd", "atomicOr", "atomicXor",
            "atomicExchange", "atomicCompareExchangeWeak",
            "pack4x8snorm", "pack4x8unorm", "pack2x16snorm", "pack2x16unorm",
            "unpack4x8snorm", "unpack4x8unorm", "unpack2x16snorm", "unpack2x16unorm",
            "storageBarrier", "workgroupBarrier", "workgroupUniformLoad",
            "select", "bitcast",
        ]),
    }
}

fn beamdsp_syntax() -> Syntax {
    Syntax {
        language: "BeamDSP",
        case_sensitive: true,
        comment: "//",
        comment_multiline: ["/*", "*/"],
        hyperlinks: std::collections::BTreeSet::new(),
        keywords: std::collections::BTreeSet::from([
            "name", "category", "inputs", "outputs", "params", "state", "ui", "process",
            "if", "else", "for", "in", "let", "mut",
            "generator", "effect", "utility",
            "audio", "cv", "midi",
            "param", "sample", "group", "canvas", "spacer",
        ]),
        types: std::collections::BTreeSet::from([
            "f32", "int", "bool",
        ]),
        special: std::collections::BTreeSet::from([
            "sin", "cos", "tan", "asin", "acos", "atan", "atan2",
            "exp", "log", "log2", "pow", "sqrt",
            "floor", "ceil", "round", "trunc", "fract",
            "abs", "clamp", "min", "max", "sign",
            "mix", "smoothstep",
            "len", "cv_or", "float",
            "sample_len", "sample_read", "sample_rate_of",
            "sample_rate", "buffer_size",
        ]),
    }
}

// ── Templates ───────────────────────────────────────────────────────

const DEFAULT_SHADER_TEMPLATE: &str = r#"// Custom Effect Shader
// Input: source_tex (the layer content)
// Output: vec4<f32> color at each pixel

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
    return color;
}
"#;

const GRAYSCALE_TEMPLATE: &str = r#"// Grayscale Effect
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
    let luminance = dot(color.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    return vec4<f32>(luminance, luminance, luminance, color.a);
}
"#;

const VIGNETTE_TEMPLATE: &str = r#"// Vignette Effect
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
    let center = vec2<f32>(0.5, 0.5);
    let dist = distance(in.uv, center);
    let radius = 0.7;
    let softness = 0.4;
    let vignette = smoothstep(radius + softness, radius, dist);
    return vec4<f32>(color.rgb * vignette, color.a);
}
"#;

const BEAMDSP_PASSTHROUGH: &str = r#"name "Passthrough"
category effect

inputs {
    audio_in: audio
}

outputs {
    audio_out: audio
}

process {
    for i in 0..buffer_size {
        audio_out[i * 2] = audio_in[i * 2];
        audio_out[i * 2 + 1] = audio_in[i * 2 + 1];
    }
}
"#;

const BEAMDSP_GAIN: &str = r#"name "Simple Gain"
category effect

inputs {
    audio_in: audio
}

outputs {
    audio_out: audio
}

params {
    gain: 1.0 [0.0, 2.0] ""
}

ui {
    param gain
}

process {
    for i in 0..buffer_size {
        audio_out[i * 2] = audio_in[i * 2] * gain;
        audio_out[i * 2 + 1] = audio_in[i * 2 + 1] * gain;
    }
}
"#;

const BEAMDSP_STEREO_DELAY: &str = r#"name "Stereo Delay"
category effect

inputs {
    audio_in: audio
}

outputs {
    audio_out: audio
}

params {
    delay_time: 0.5 [0.01, 2.0] "s"
    feedback:   0.3 [0.0, 0.95] ""
    mix:        0.5 [0.0, 1.0]  ""
}

state {
    buffer: [88200]f32
    write_pos: int
}

ui {
    param delay_time
    param feedback
    param mix
}

process {
    let delay_samples = int(delay_time * float(sample_rate)) * 2;
    for i in 0..buffer_size {
        let l = audio_in[i * 2];
        let r = audio_in[i * 2 + 1];
        let read_pos = (write_pos - delay_samples + len(buffer)) % len(buffer);
        let dl = buffer[read_pos];
        let dr = buffer[read_pos + 1];
        buffer[write_pos] = l + dl * feedback;
        buffer[write_pos + 1] = r + dr * feedback;
        write_pos = (write_pos + 2) % len(buffer);
        audio_out[i * 2]     = l * (1.0 - mix) + dl * mix;
        audio_out[i * 2 + 1] = r * (1.0 - mix) + dr * mix;
    }
}
"#;

const BEAMDSP_LFO: &str = r#"name "Custom LFO"
category generator

outputs {
    cv_out: cv
}

params {
    rate: 1.0 [0.01, 20.0] "Hz"
    depth: 1.0 [0.0, 1.0] ""
}

state {
    phase: f32
}

ui {
    param rate
    param depth
}

process {
    let inc = rate / float(sample_rate);
    for i in 0..buffer_size {
        cv_out[i] = sin(phase * 6.2831853) * depth;
        phase = phase + inc;
        if phase >= 1.0 {
            phase = phase - 1.0;
        }
    }
}
"#;

// ── Pane state ──────────────────────────────────────────────────────

/// Script Editor pane state — unified editor for shaders and DSP scripts
pub struct ShaderEditorPane {
    /// Current editor mode
    mode: EditorMode,
    /// The source code being edited
    code: String,
    /// Display name for the asset being edited
    asset_name: String,
    /// Error message from last compilation attempt
    compile_error: Option<String>,

    // ── Shader mode state ───────────────────────────────────
    /// ID of effect being edited (None = new effect)
    editing_effect_id: Option<Uuid>,

    // ── BeamDSP mode state ──────────────────────────────────
    /// ID of script being edited (None = new script)
    editing_script_id: Option<Uuid>,

    // ── Shared state ────────────────────────────────────────
    /// Original code when asset was loaded (for dirty checking)
    original_code: Option<String>,
    /// Original name when asset was loaded (for dirty checking)
    original_name: Option<String>,
    /// Effect awaiting confirmation to load (when there are unsaved changes)
    pending_load_effect: Option<EffectDefinition>,
    /// Script awaiting confirmation to load (when there are unsaved changes)
    pending_load_script: Option<ScriptDefinition>,
    /// Whether to show the unsaved changes confirmation dialog
    show_unsaved_dialog: bool,
}

impl ShaderEditorPane {
    pub fn new() -> Self {
        Self {
            mode: EditorMode::Shader,
            code: DEFAULT_SHADER_TEMPLATE.to_string(),
            asset_name: "Custom Effect".to_string(),
            compile_error: None,
            editing_effect_id: None,
            editing_script_id: None,
            original_code: None,
            original_name: None,
            pending_load_effect: None,
            pending_load_script: None,
            show_unsaved_dialog: false,
        }
    }

    fn default_code(&self) -> &str {
        match self.mode {
            EditorMode::Shader => DEFAULT_SHADER_TEMPLATE,
            EditorMode::BeamDSP => BEAMDSP_PASSTHROUGH,
        }
    }

    fn default_name(&self) -> &str {
        match self.mode {
            EditorMode::Shader => "Custom Effect",
            EditorMode::BeamDSP => "New Script",
        }
    }

    fn has_unsaved_changes(&self) -> bool {
        match (&self.original_code, &self.original_name) {
            (Some(orig_code), Some(orig_name)) => {
                self.code != *orig_code || self.asset_name != *orig_name
            }
            (None, None) => {
                self.code != self.default_code() || self.asset_name != self.default_name()
            }
            _ => true,
        }
    }

    fn is_editing_existing(&self) -> bool {
        match self.mode {
            EditorMode::Shader => self.editing_effect_id.is_some(),
            EditorMode::BeamDSP => self.editing_script_id.is_some(),
        }
    }

    fn mark_saved_state(&mut self) {
        self.original_code = Some(self.code.clone());
        self.original_name = Some(self.asset_name.clone());
    }

    fn new_asset(&mut self) {
        self.asset_name = self.default_name().to_string();
        self.code = self.default_code().to_string();
        match self.mode {
            EditorMode::Shader => self.editing_effect_id = None,
            EditorMode::BeamDSP => self.editing_script_id = None,
        }
        self.original_code = None;
        self.original_name = None;
        self.compile_error = None;
    }

    // ── Shader-specific ─────────────────────────────────────

    fn load_effect(&mut self, effect: &EffectDefinition) {
        self.mode = EditorMode::Shader;
        self.asset_name = effect.name.clone();
        self.code = effect.shader_code.clone();
        if effect.category == EffectCategory::Custom {
            self.editing_effect_id = Some(effect.id);
        } else {
            self.editing_effect_id = None;
        }
        self.original_code = Some(effect.shader_code.clone());
        self.original_name = Some(effect.name.clone());
        self.compile_error = None;
    }

    fn lookup_effect(
        &self,
        effect_id: Uuid,
        document: &lightningbeam_core::document::Document,
    ) -> Option<EffectDefinition> {
        use lightningbeam_core::effect_registry::EffectRegistry;
        if let Some(def) = document.effect_definitions.get(&effect_id) {
            return Some(def.clone());
        }
        EffectRegistry::get_by_id(&effect_id)
    }

    fn save_effect(&mut self, shared: &mut SharedPaneState) -> bool {
        if self.asset_name.trim().is_empty() {
            self.compile_error = Some("Name cannot be empty".to_string());
            return false;
        }
        let effect = if let Some(existing_id) = self.editing_effect_id {
            EffectDefinition::with_id(
                existing_id, self.asset_name.clone(),
                EffectCategory::Custom, self.code.clone(), vec![],
            )
        } else {
            EffectDefinition::new(
                self.asset_name.clone(), EffectCategory::Custom,
                self.code.clone(), vec![],
            )
        };
        let effect_id = effect.id;
        shared.action_executor.document_mut().add_effect_definition(effect);
        self.editing_effect_id = Some(effect_id);
        self.mark_saved_state();
        shared.effect_thumbnails_to_invalidate.push(effect_id);
        self.compile_error = None;
        true
    }

    // ── BeamDSP-specific ────────────────────────────────────

    fn load_script(&mut self, script: &ScriptDefinition) {
        self.mode = EditorMode::BeamDSP;
        self.asset_name = script.name.clone();
        self.code = script.source.clone();
        self.editing_script_id = Some(script.id);
        self.original_code = Some(script.source.clone());
        self.original_name = Some(script.name.clone());
        self.compile_error = None;
    }

    fn save_script(&mut self, shared: &mut SharedPaneState) -> bool {
        if self.asset_name.trim().is_empty() {
            self.compile_error = Some("Name cannot be empty".to_string());
            return false;
        }

        // Compile first — reject if invalid
        if let Err(err) = beamdsp::compile(&self.code) {
            self.compile_error = Some(format!("{}", err));
            return false;
        }

        let script = if let Some(existing_id) = self.editing_script_id {
            ScriptDefinition::with_id(existing_id, self.asset_name.clone(), self.code.clone())
        } else {
            ScriptDefinition::new(self.asset_name.clone(), self.code.clone())
        };
        let script_id = script.id;
        shared.action_executor.document_mut().add_script_definition(script);
        self.editing_script_id = Some(script_id);
        self.mark_saved_state();
        self.compile_error = None;

        // Auto-recompile: notify all Script nodes referencing this script
        *shared.script_saved = Some(script_id);
        true
    }

    // ── Dialog rendering ────────────────────────────────────

    fn render_unsaved_dialog(&mut self, ui: &mut egui::Ui) -> Option<UnsavedDialogResult> {
        let mut result = None;
        if self.show_unsaved_dialog {
            egui::Window::new("Unsaved Changes")
                .id(egui::Id::new("script_unsaved_dialog"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ui.ctx(), |ui| {
                    ui.set_min_width(300.0);
                    let label = match self.mode {
                        EditorMode::Shader => "You have unsaved changes to this shader.",
                        EditorMode::BeamDSP => "You have unsaved changes to this script.",
                    };
                    ui.label(label);
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

    // ── Toolbar rendering ───────────────────────────────────

    fn render_toolbar(
        &mut self,
        ui: &mut Ui,
        available_scripts: &[(Uuid, String)],
    ) -> (bool, bool, Option<Uuid>) {
        let mut save_clicked = false;
        let mut export_clicked = false;
        let mut open_script_id = None;
        ui.horizontal(|ui| {
            if ui.button("New").clicked() {
                self.new_asset();
            }

            // Open dropdown for existing scripts (BeamDSP mode)
            if self.mode == EditorMode::BeamDSP && !available_scripts.is_empty() {
                let open_btn = ui.button("Open");
                let popup_id = egui::Id::new("script_editor_open_popup");
                if open_btn.clicked() {
                    ui.memory_mut(|m| m.toggle_popup(popup_id));
                }
                egui::popup_below_widget(ui, popup_id, &open_btn, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
                    ui.set_min_width(160.0);
                    for (id, name) in available_scripts {
                        let is_current = self.editing_script_id == Some(*id);
                        if ui.selectable_label(is_current, name).clicked() {
                            open_script_id = Some(*id);
                            ui.memory_mut(|m| m.close_popup(popup_id));
                        }
                    }
                });
            }

            ui.separator();

            ui.label("Name:");
            ui.add(egui::TextEdit::singleline(&mut self.asset_name).desired_width(150.0));
            ui.separator();

            // Mode-specific templates
            match self.mode {
                EditorMode::Shader => {
                    egui::ComboBox::from_label("Template")
                        .selected_text("Insert Template")
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(false, "Basic (Passthrough)").clicked() {
                                self.code = DEFAULT_SHADER_TEMPLATE.to_string();
                            }
                            if ui.selectable_label(false, "Grayscale").clicked() {
                                self.code = GRAYSCALE_TEMPLATE.to_string();
                            }
                            if ui.selectable_label(false, "Vignette").clicked() {
                                self.code = VIGNETTE_TEMPLATE.to_string();
                            }
                        });
                }
                EditorMode::BeamDSP => {
                    egui::ComboBox::from_label("Template")
                        .selected_text("Insert Template")
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(false, "Passthrough").clicked() {
                                self.code = BEAMDSP_PASSTHROUGH.to_string();
                            }
                            if ui.selectable_label(false, "Simple Gain").clicked() {
                                self.code = BEAMDSP_GAIN.to_string();
                            }
                            if ui.selectable_label(false, "Stereo Delay").clicked() {
                                self.code = BEAMDSP_STEREO_DELAY.to_string();
                            }
                            if ui.selectable_label(false, "Custom LFO").clicked() {
                                self.code = BEAMDSP_LFO.to_string();
                            }
                        });
                }
            }
            ui.separator();

            if ui.button("Save").clicked() {
                save_clicked = true;
            }

            if self.mode == EditorMode::BeamDSP {
                if ui.button("Export").clicked() {
                    export_clicked = true;
                }
            }

            if self.has_unsaved_changes() {
                ui.label(egui::RichText::new("*").color(egui::Color32::YELLOW));
            }
            if self.is_editing_existing() {
                ui.label(egui::RichText::new("(Editing)").weak());
            } else {
                ui.label(egui::RichText::new("(New)").weak());
            }
        });
        (save_clicked, export_clicked, open_script_id)
    }

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
            if let Some(effect) = self.lookup_effect(effect_id, shared.action_executor.document()) {
                if self.has_unsaved_changes() {
                    self.pending_load_effect = Some(effect);
                    self.show_unsaved_dialog = true;
                } else {
                    self.load_effect(&effect);
                }
            }
        }

        // Handle script loading request from node graph
        if let Some(script_id) = shared.script_to_edit.take() {
            if let Some(script) = shared.action_executor.document().get_script_definition(&script_id).cloned() {
                if self.has_unsaved_changes() {
                    self.pending_load_script = Some(script);
                    self.show_unsaved_dialog = true;
                } else {
                    self.load_script(&script);
                }
            }
        }

        // Handle unsaved changes dialog
        if let Some(result) = self.render_unsaved_dialog(ui) {
            match result {
                UnsavedDialogResult::Cancel => {
                    self.pending_load_effect = None;
                    self.pending_load_script = None;
                    self.show_unsaved_dialog = false;
                }
                UnsavedDialogResult::Discard => {
                    if let Some(effect) = self.pending_load_effect.take() {
                        self.load_effect(&effect);
                    }
                    if let Some(script) = self.pending_load_script.take() {
                        self.load_script(&script);
                    }
                    self.show_unsaved_dialog = false;
                }
                UnsavedDialogResult::SaveAndContinue => {
                    match self.mode {
                        EditorMode::Shader => { self.save_effect(shared); }
                        EditorMode::BeamDSP => { self.save_script(shared); }
                    }
                    if let Some(effect) = self.pending_load_effect.take() {
                        self.load_effect(&effect);
                    }
                    if let Some(script) = self.pending_load_script.take() {
                        self.load_script(&script);
                    }
                    self.show_unsaved_dialog = false;
                }
            }
        }

        // Background
        ui.painter().rect_filled(rect, 0.0, egui::Color32::from_rgb(25, 25, 30));

        let content_rect = rect.shrink(8.0);
        let mut content_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(content_rect)
                .layout(egui::Layout::top_down(egui::Align::LEFT)),
        );
        content_ui.set_min_width(content_rect.width() - 16.0);

        // Mode selector
        content_ui.horizontal(|ui| {
            if ui.selectable_value(&mut self.mode, EditorMode::Shader, "Shader").changed() {
                // Switching modes - reset to defaults for the new mode
                self.new_asset();
            }
            if ui.selectable_value(&mut self.mode, EditorMode::BeamDSP, "BeamDSP").changed() {
                self.new_asset();
            }
        });
        content_ui.add_space(2.0);

        // Collect available scripts for the Open dropdown
        let available_scripts: Vec<(Uuid, String)> = shared.action_executor.document()
            .script_definitions()
            .map(|s| (s.id, s.name.clone()))
            .collect();

        // Toolbar
        let (save_clicked, export_clicked, open_script_id) = self.render_toolbar(&mut content_ui, &available_scripts);
        content_ui.add_space(4.0);
        content_ui.separator();
        content_ui.add_space(4.0);

        // Handle open script
        if let Some(script_id) = open_script_id {
            if let Some(script) = shared.action_executor.document().get_script_definition(&script_id).cloned() {
                if self.has_unsaved_changes() {
                    self.pending_load_script = Some(script);
                    self.show_unsaved_dialog = true;
                } else {
                    self.load_script(&script);
                }
            }
        }

        // Handle save
        if save_clicked {
            match self.mode {
                EditorMode::Shader => { self.save_effect(shared); }
                EditorMode::BeamDSP => { self.save_script(shared); }
            }
        }

        // Handle export (.bdsp)
        if export_clicked {
            let default_name = format!("{}.bdsp", self.asset_name.trim());
            if let Some(path) = rfd::FileDialog::new()
                .set_title("Export BeamDSP Script")
                .set_file_name(&default_name)
                .add_filter("BeamDSP Script", &["bdsp"])
                .save_file()
            {
                if let Err(e) = std::fs::write(&path, &self.code) {
                    self.compile_error = Some(format!("Export failed: {}", e));
                }
            }
        }

        // Error panel
        self.render_error_panel(&mut content_ui);

        // Code editor
        let remaining_rect = content_ui.available_rect_before_wrap();
        let syntax = match self.mode {
            EditorMode::Shader => wgsl_syntax(),
            EditorMode::BeamDSP => beamdsp_syntax(),
        };

        egui::ScrollArea::both()
            .id_salt(("script_editor_scroll", path))
            .auto_shrink([false, false])
            .show(&mut content_ui, |ui| {
                ui.set_min_size(remaining_rect.size());
                CodeEditor::default()
                    .id_source("script_code_editor")
                    .with_rows(50)
                    .with_fontsize(13.0)
                    .with_theme(ColorTheme::GRUVBOX_DARK)
                    .with_syntax(syntax)
                    .with_numlines(true)
                    .show(ui, &mut self.code);
            });
    }

    fn name(&self) -> &str {
        match self.mode {
            EditorMode::Shader => "Shader Editor",
            EditorMode::BeamDSP => "Script Editor",
        }
    }
}

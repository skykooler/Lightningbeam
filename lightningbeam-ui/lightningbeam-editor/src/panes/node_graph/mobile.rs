//! Mobile (touch) node editor — Focus & Patch views (wireframe Plate 07).
//!
//! Swapped in for the desktop `draw_graph_editor` canvas when `shared.is_mobile`. Focus shows one
//! module's parameters as big touch controls plus navigation; Patch does tap-to-cable wiring. All
//! edits reuse the existing dispatch: mutating a param's `ValueType` in place is picked up by
//! `check_parameter_changes`, and add/connect/disconnect go through `NodeGraphAction`.

use super::graph_data::{DataType, NodeTemplate, ValueType};
use super::{actions, NodeGraphPane};
use crate::mobile::icons;
use eframe::egui;
use egui_node_graph2::{InputId, InputParamKind, NodeDataTrait, NodeId, NodeTemplateTrait, OutputId};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NodeViewMode {
    Focus,
    Patch,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PortDir {
    In,
    Out,
}

/// An armed port awaiting a compatible endpoint to cable to.
#[derive(Clone, Copy)]
pub struct PatchPick {
    node: NodeId,
    port: usize,
    dir: PortDir,
    typ: DataType,
}

pub struct MobileNodeState {
    pub mode: NodeViewMode,
    /// The module currently shown in Focus (and centred in Patch).
    pub focus_node: Option<NodeId>,
    /// Armed cable source in Patch: (node, output-port index).
    pub patch_source: Option<(NodeId, usize)>,
    /// Whether the add-node picker overlay is open.
    pub show_add: bool,
    /// Search filter in the add-node picker.
    pub add_search: String,
    /// Armed port in Patch awaiting a compatible endpoint.
    patch_pick: Option<PatchPick>,
}

impl Default for MobileNodeState {
    fn default() -> Self {
        Self {
            mode: NodeViewMode::Focus,
            focus_node: None,
            patch_source: None,
            show_add: false,
            add_search: String::new(),
            patch_pick: None,
        }
    }
}

impl NodeGraphPane {
    pub(super) fn render_mobile(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        shared: &mut crate::panes::SharedPaneState,
    ) {
        let bg = shared.theme.bg_color(&["#node-editor", ".pane-content"], ui.ctx(), egui::Color32::from_gray(28));
        ui.painter().rect_filled(rect, 0.0, bg);

        // Resolve the focus node: keep the current one if it still exists, else the selected node,
        // else the first node in the graph.
        let focus_valid = self
            .mobile
            .focus_node
            .map_or(false, |id| self.state.graph.nodes.get(id).is_some());
        if !focus_valid {
            self.mobile.focus_node = self
                .state
                .selected_nodes
                .iter()
                .next()
                .copied()
                .or_else(|| self.state.graph.iter_nodes().next());
        }

        // Header: Focus/Patch toggle + focused node name.
        let header_h = 44.0;
        let header = egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), rect.top() + header_h));
        let body = egui::Rect::from_min_max(egui::pos2(rect.left(), header.bottom()), rect.max);

        let name = self
            .mobile
            .focus_node
            .and_then(|id| self.state.graph.nodes.get(id))
            .map(|n| n.label.clone())
            .unwrap_or_else(|| "—".to_string());
        ui.painter().line_segment(
            [egui::pos2(header.left(), header.bottom()), egui::pos2(header.right(), header.bottom())],
            egui::Stroke::new(1.0, egui::Color32::from_gray(60)),
        );
        let mut mode = self.mobile.mode;
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(header.shrink(8.0)).layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                if ui.selectable_label(mode == NodeViewMode::Focus, "Focus").clicked() {
                    mode = NodeViewMode::Focus;
                }
                if ui.selectable_label(mode == NodeViewMode::Patch, "Patch").clicked() {
                    mode = NodeViewMode::Patch;
                }
                ui.add_space(8.0);
                ui.label(egui::RichText::new(name).strong());
            },
        );
        self.mobile.mode = mode;

        // Keep backend-id map current so embedded `bottom_ui` (sampler/script/etc.) targets the
        // right backend node (mirrors the desktop pre-draw sync).
        self.user_state.node_backend_ids = self
            .node_id_map
            .iter()
            .map(|(&nid, bid)| (nid, bid.index()))
            .collect();

        match self.mobile.mode {
            NodeViewMode::Focus => self.render_mobile_focus(ui, body, shared),
            NodeViewMode::Patch => self.render_mobile_patch(ui, body, shared),
        }

        // Same dispatch tail as the desktop path: apply param edits and run any queued action.
        self.check_parameter_changes(shared);
        self.execute_pending_action(shared);

        // Drain the custom-UI loads that have self-contained handlers (sampler + script sample).
        // Other bespoke interactions (sequencer grid, NAM model, script canvas) are a case-by-case
        // follow-up; clear their queues so they don't accumulate.
        if let Some(load) = self.user_state.pending_sampler_load.take() {
            self.handle_pending_sampler_load(load, shared);
        }
        if let Some(load) = self.user_state.pending_script_sample_load.take() {
            self.handle_pending_script_sample_load(load, shared);
        }
        self.user_state.pending_sequencer_changes.clear();
        self.user_state.pending_draw_param_changes.clear();
        self.user_state.pending_root_note_changes.clear();
    }

    fn render_mobile_focus(
        &mut self,
        ui: &mut egui::Ui,
        body: egui::Rect,
        shared: &mut crate::panes::SharedPaneState,
    ) {
        let Some(focus) = self.mobile.focus_node else {
            return;
        };

        // Full-width minimap strip across the top; everything else below it.
        let mm_h = 96.0;
        let mm = egui::Rect::from_min_max(body.min, egui::pos2(body.right(), body.top() + mm_h));
        // Reserve an add-node button strip at the bottom.
        let add_h = 46.0;
        let add_rect = egui::Rect::from_min_max(egui::pos2(body.left(), body.bottom() - add_h), body.max);
        let content = egui::Rect::from_min_max(egui::pos2(body.left(), mm.bottom()), egui::pos2(body.right(), add_rect.top()));

        // Minimap first (tap a node to focus it).
        let mut jump: Option<NodeId> = self.render_minimap(ui, mm, focus);

        // Connection travel chips (owned) + params.
        let (in_chips, out_chips) = self.focus_chips(focus);
        let inputs: Vec<(String, InputId)> = self
            .state
            .graph
            .nodes
            .get(focus)
            .map(|n| n.inputs.clone())
            .unwrap_or_default();

        ui.scope_builder(
            egui::UiBuilder::new().max_rect(content.shrink(10.0)).layout(egui::Layout::top_down(egui::Align::Min)),
            |ui| {
                if !in_chips.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(egui::RichText::new("in").weak());
                        for (label, node) in &in_chips {
                            if ui.button(label.as_str()).clicked() {
                                jump = Some(*node);
                            }
                        }
                    });
                }
                if !out_chips.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(egui::RichText::new("out").weak());
                        for (label, node) in &out_chips {
                            if ui.button(label.as_str()).clicked() {
                                jump = Some(*node);
                            }
                        }
                    });
                }
                if !in_chips.is_empty() || !out_chips.is_empty() {
                    ui.separator();
                }
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let mut any = false;
                    for (name, input_id) in &inputs {
                        let Some(param) = self.state.graph.inputs.get_mut(*input_id) else {
                            continue;
                        };
                        if matches!(param.kind, InputParamKind::ConnectionOnly) {
                            continue;
                        }
                        any = true;
                        render_param_row(ui, name, &mut param.value);
                        ui.add_space(6.0);
                    }
                    if !any {
                        ui.label(egui::RichText::new("No editable parameters").weak());
                    }
                    // Embedded desktop custom UI (sampler picker, sequencer grid, script UI, …).
                    // Standard nodes render nothing here.
                    if let Some(node) = self.state.graph.nodes.get(focus) {
                        let _ = node.user_data.bottom_ui(ui, focus, &self.state.graph, &mut self.user_state);
                    }
                });
            },
        );

        // Add-node button.
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(add_rect.shrink(8.0)).layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                if ui.button(egui::RichText::new("＋ Add node").size(15.0)).clicked() {
                    self.mobile.show_add = true;
                }
            },
        );

        if let Some(nid) = jump {
            self.mobile.focus_node = Some(nid);
        }

        if self.mobile.show_add {
            self.render_add_picker(ui, body, shared);
        }
    }

    /// Connection chips for the focus node: (label, remote node) for upstream inputs and downstream
    /// outputs. Label names the remote endpoint (`node ▸ port`).
    fn focus_chips(&self, focus: NodeId) -> (Vec<(String, NodeId)>, Vec<(String, NodeId)>) {
        let mut ins = Vec::new();
        let mut outs = Vec::new();
        for (input_id, outputs) in self.state.graph.iter_connection_groups() {
            let in_node = self.state.graph.inputs.get(input_id).map(|p| p.node);
            for output_id in outputs {
                let out_node = self.state.graph.outputs.get(output_id).map(|p| p.node);
                if in_node == Some(focus) {
                    if let Some(src) = out_node {
                        ins.push((format!("{} · {}", self.node_label(src), self.output_port_name(src, output_id)), src));
                    }
                }
                if out_node == Some(focus) {
                    if let Some(dst) = in_node {
                        outs.push((format!("{} · {}", self.node_label(dst), self.input_port_name(dst, input_id)), dst));
                    }
                }
            }
        }
        (ins, outs)
    }

    fn node_label(&self, n: NodeId) -> String {
        self.state.graph.nodes.get(n).map(|x| x.label.clone()).unwrap_or_default()
    }
    fn output_port_name(&self, n: NodeId, oid: OutputId) -> String {
        self.state
            .graph
            .nodes
            .get(n)
            .and_then(|node| node.outputs.iter().find(|(_, id)| *id == oid).map(|(nm, _)| nm.clone()))
            .unwrap_or_default()
    }
    fn input_port_name(&self, n: NodeId, iid: InputId) -> String {
        self.state
            .graph
            .nodes
            .get(n)
            .and_then(|node| node.inputs.iter().find(|(_, id)| *id == iid).map(|(nm, _)| nm.clone()))
            .unwrap_or_default()
    }

    /// Draw a minimap of the graph; returns a node id if the user tapped one.
    fn render_minimap(&self, ui: &mut egui::Ui, rect: egui::Rect, focus: NodeId) -> Option<NodeId> {
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, egui::Color32::from_rgba_unmultiplied(0, 0, 0, 120));
        painter.rect_stroke(rect, 4.0, egui::Stroke::new(1.0, egui::Color32::from_gray(70)), egui::StrokeKind::Inside);

        let nodes: Vec<(NodeId, egui::Pos2)> = self
            .state
            .graph
            .iter_nodes()
            .filter_map(|n| self.state.node_positions.get(n).map(|p| (n, *p)))
            .collect();
        if nodes.is_empty() {
            return None;
        }
        let (mut mnx, mut mny, mut mxx, mut mxy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for (_, p) in &nodes {
            mnx = mnx.min(p.x);
            mny = mny.min(p.y);
            mxx = mxx.max(p.x);
            mxy = mxy.max(p.y);
        }
        let pad = 10.0;
        let inner = rect.shrink(pad);
        let span = egui::vec2((mxx - mnx).max(1.0), (mxy - mny).max(1.0));
        let map = |p: egui::Pos2| {
            egui::pos2(
                inner.left() + (p.x - mnx) / span.x * inner.width(),
                inner.top() + (p.y - mny) / span.y * inner.height(),
            )
        };

        // Connection lines.
        for (input_id, outputs) in self.state.graph.iter_connection_groups() {
            let in_pos = self.state.graph.inputs.get(input_id).and_then(|p| self.state.node_positions.get(p.node)).copied();
            for output_id in outputs {
                let out_pos = self.state.graph.outputs.get(output_id).and_then(|p| self.state.node_positions.get(p.node)).copied();
                if let (Some(a), Some(b)) = (out_pos, in_pos) {
                    painter.line_segment([map(a), map(b)], egui::Stroke::new(1.0, egui::Color32::from_gray(90)));
                }
            }
        }

        let resp = ui.interact(rect, ui.id().with("node_minimap"), egui::Sense::click());
        let click = if resp.clicked() { resp.interact_pointer_pos() } else { None };
        let mut hit = None;
        let mut best_d = f32::MAX;
        for (nid, p) in &nodes {
            let c = map(*p);
            let focused = *nid == focus;
            let color = if focused { egui::Color32::from_rgb(0x4a, 0xa3, 0xff) } else { egui::Color32::from_gray(200) };
            painter.circle_filled(c, if focused { 7.0 } else { 5.0 }, color);
            if focused {
                painter.circle_stroke(c, 9.0, egui::Stroke::new(1.5, color));
            }
            // Tap selects the nearest node (dots are small, so don't require a precise hit).
            if let Some(cp) = click {
                let d = (cp - c).length();
                if d < best_d {
                    best_d = d;
                    hit = Some(*nid);
                }
            }
        }
        hit
    }

    /// The add-node picker overlay: a searchable list of templates.
    fn render_add_picker(&mut self, ui: &mut egui::Ui, body: egui::Rect, shared: &mut crate::panes::SharedPaneState) {
        let panel = egui::Rect::from_center_size(body.center(), egui::vec2(body.width().min(320.0), body.height().min(420.0)));
        let mut chosen: Option<NodeTemplate> = None;
        let mut close = false;
        ui.scope_builder(egui::UiBuilder::new().max_rect(panel), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Add node").strong());
                    if ui.button("✕").clicked() {
                        close = true;
                    }
                });
                ui.add(egui::TextEdit::singleline(&mut self.mobile.add_search).hint_text("Search…"));
                let q = self.mobile.add_search.to_lowercase();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for template in NodeTemplate::all_finder_kinds() {
                        let label = template.node_finder_label(&mut self.user_state).to_string();
                        if !q.is_empty() && !label.to_lowercase().contains(&q) {
                            continue;
                        }
                        if ui.button(&label).clicked() {
                            chosen = Some(template);
                        }
                    }
                });
            });
        });
        if let Some(t) = chosen {
            self.mobile_add_node(t, shared);
            self.mobile.show_add = false;
            self.mobile.add_search.clear();
        }
        if close {
            self.mobile.show_add = false;
        }
    }

    /// Create a node (frontend + backend action) and focus it, mirroring the desktop CreatedNode path.
    fn mobile_add_node(&mut self, template: NodeTemplate, _shared: &mut crate::panes::SharedPaneState) {
        let Some(track_id) = self.track_id else {
            return;
        };
        // Place near the current focus node, else the origin.
        let base = self
            .mobile
            .focus_node
            .and_then(|id| self.state.node_positions.get(id).copied())
            .unwrap_or(egui::Pos2::ZERO);
        let pos = base + egui::vec2(160.0, 0.0);

        let label = template.node_graph_label(&mut self.user_state);
        let user_data = template.user_data(&mut self.user_state);
        let new_id = self
            .state
            .graph
            .add_node(label, user_data, |graph, id| template.build_node(graph, &mut self.user_state, id));
        self.state.node_positions.insert(new_id, pos);

        let node_type = template.backend_type_name().to_string();
        self.pending_action = Some(Box::new(actions::NodeGraphAction::AddNode(
            actions::AddNodeAction::new(track_id, node_type.clone(), (pos.x, pos.y)),
        )));
        self.pending_node_addition = Some((new_id, node_type, (pos.x, pos.y)));
        self.mobile.focus_node = Some(new_id);
    }

    fn render_mobile_patch(
        &mut self,
        ui: &mut egui::Ui,
        body: egui::Rect,
        _shared: &mut crate::panes::SharedPaneState,
    ) {
        let Some(focus) = self.mobile.focus_node else {
            return;
        };

        // Build owned row data (immutable reads) so rendering can freely queue ops.
        let inputs: Vec<(String, InputId)> = self.state.graph.nodes.get(focus).map(|n| n.inputs.clone()).unwrap_or_default();
        let outputs: Vec<(String, OutputId)> = self.state.graph.nodes.get(focus).map(|n| n.outputs.clone()).unwrap_or_default();

        // Rows: (idx, name, type, cables[(remote_node, remote_port, other_node, other_port)]).
        let mut in_rows: Vec<(usize, String, DataType, Vec<(String, String, NodeId, usize)>)> = Vec::new();
        for (idx, (name, input_id)) in inputs.iter().enumerate() {
            let Some(typ) = self.state.graph.inputs.get(*input_id).map(|p| p.typ) else { continue };
            let mut cables = Vec::new();
            if let Some(outs) = self.state.graph.connections.get(*input_id) {
                for oid in outs {
                    if let Some(src) = self.state.graph.outputs.get(*oid).map(|o| o.node) {
                        let sp = self.output_port_index(src, *oid);
                        cables.push((self.node_label(src), self.output_port_name(src, *oid), src, sp));
                    }
                }
            }
            in_rows.push((idx, name.clone(), typ, cables));
        }
        let mut out_rows: Vec<(usize, String, DataType, Vec<(String, String, NodeId, usize)>)> = Vec::new();
        for (idx, (name, output_id)) in outputs.iter().enumerate() {
            let Some(typ) = self.state.graph.outputs.get(*output_id).map(|o| o.typ) else { continue };
            let mut cables = Vec::new();
            for (input_id, outs) in self.state.graph.iter_connection_groups() {
                if outs.iter().any(|o| o == output_id) {
                    if let Some(dst) = self.state.graph.inputs.get(input_id).map(|i| i.node) {
                        let dp = self.input_port_index(dst, input_id);
                        cables.push((self.node_label(dst), self.input_port_name(dst, input_id), dst, dp));
                    }
                }
            }
            out_rows.push((idx, name.clone(), typ, cables));
        }

        // Queue ops during render, apply after (avoids borrowing self mid-render).
        enum Op {
            Disconnect(NodeId, usize, NodeId, usize), // (out_node, out_port, in_node, in_port)
            Connect(NodeId, usize, NodeId, usize),
            Pick(PatchPick),
            ClearPick,
        }
        let mut ops: Vec<Op> = Vec::new();
        let pick = self.mobile.patch_pick;

        ui.scope_builder(
            egui::UiBuilder::new().max_rect(body.shrink(10.0)).layout(egui::Layout::top_down(egui::Align::Min)),
            |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    // A Grid aligns each section's port arrows into a column. Tapping a port arrow
                    // arms a cable from it (also for unconnected ports); tapping a cable chip removes
                    // it. Inputs read `[cables] ⥓ name`; outputs read `name ↦ [cables]`.
                    ui.label(egui::RichText::new("Inputs").weak());
                    egui::Grid::new("patch_inputs").num_columns(3).spacing([8.0, 6.0]).show(ui, |ui| {
                        for (idx, name, typ, cables) in &in_rows {
                            ui.horizontal(|ui| {
                                for (rnode, rport, sn, sp) in cables {
                                    if cable_chip(ui, rnode, rport, *typ).clicked() {
                                        ops.push(Op::Disconnect(*sn, *sp, focus, *idx));
                                    }
                                }
                            });
                            if arrow_button(ui, icons::ARROW_RIGHT_TO_LINE, *typ).clicked() {
                                ops.push(Op::Pick(PatchPick { node: focus, port: *idx, dir: PortDir::In, typ: *typ }));
                            }
                            ui.label(name.as_str());
                            ui.end_row();
                        }
                    });
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new("Outputs").weak());
                    egui::Grid::new("patch_outputs").num_columns(3).spacing([8.0, 6.0]).show(ui, |ui| {
                        for (idx, name, typ, cables) in &out_rows {
                            ui.label(name.as_str());
                            if arrow_button(ui, icons::ARROW_RIGHT_FROM_LINE, *typ).clicked() {
                                ops.push(Op::Pick(PatchPick { node: focus, port: *idx, dir: PortDir::Out, typ: *typ }));
                            }
                            ui.horizontal(|ui| {
                                for (rnode, rport, dn, dp) in cables {
                                    if cable_chip(ui, rnode, rport, *typ).clicked() {
                                        ops.push(Op::Disconnect(focus, *idx, *dn, *dp));
                                    }
                                }
                            });
                            ui.end_row();
                        }
                    });
                });
            },
        );

        // Compatible-endpoint picker overlay when a port is armed.
        if let Some(p) = pick {
            let candidates = self.patch_candidates(p);
            let panel = egui::Rect::from_center_size(body.center(), egui::vec2(body.width().min(320.0), body.height().min(360.0)));
            ui.scope_builder(egui::UiBuilder::new().max_rect(panel), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        port_marker(ui, p.typ, p.dir);
                        ui.label(egui::RichText::new("Connect to…").strong());
                        if ui.button("✕").clicked() {
                            ops.push(Op::ClearPick);
                        }
                    });
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        if candidates.is_empty() {
                            ui.label(egui::RichText::new("No compatible ports").weak());
                        }
                        for (label, other, other_port) in &candidates {
                            if ui.button(egui::RichText::new(label.as_str()).color(type_color(p.typ))).clicked() {
                                // Orient the cable: armed In needs an Out source; armed Out needs an In target.
                                match p.dir {
                                    PortDir::In => ops.push(Op::Connect(*other, *other_port, p.node, p.port)),
                                    PortDir::Out => ops.push(Op::Connect(p.node, p.port, *other, *other_port)),
                                }
                                ops.push(Op::ClearPick);
                            }
                        }
                    });
                });
            });
        }

        for op in ops {
            match op {
                Op::Disconnect(on, op_, inn, ip) => self.mobile_disconnect(on, op_, inn, ip),
                Op::Connect(on, op_, inn, ip) => self.mobile_connect(on, op_, inn, ip),
                Op::Pick(p) => self.mobile.patch_pick = Some(p),
                Op::ClearPick => self.mobile.patch_pick = None,
            }
        }
    }

    /// Compatible endpoints on OTHER nodes for an armed port (opposite direction, same type).
    fn patch_candidates(&self, p: PatchPick) -> Vec<(String, NodeId, usize)> {
        let mut out = Vec::new();
        for node in self.state.graph.iter_nodes() {
            if node == p.node {
                continue;
            }
            let Some(n) = self.state.graph.nodes.get(node) else { continue };
            match p.dir {
                // Armed input → list outputs of matching type.
                PortDir::In => {
                    for (i, (name, oid)) in n.outputs.iter().enumerate() {
                        if self.state.graph.outputs.get(*oid).map(|o| o.typ) == Some(p.typ) {
                            out.push((format!("{} · {}", self.node_label(node), name), node, i));
                        }
                    }
                }
                // Armed output → list inputs of matching type.
                PortDir::Out => {
                    for (i, (name, iid)) in n.inputs.iter().enumerate() {
                        if self.state.graph.inputs.get(*iid).map(|x| x.typ) == Some(p.typ) {
                            out.push((format!("{} · {}", self.node_label(node), name), node, i));
                        }
                    }
                }
            }
        }
        out
    }

    fn output_port_index(&self, n: NodeId, oid: OutputId) -> usize {
        self.state.graph.nodes.get(n).and_then(|node| node.outputs.iter().position(|(_, id)| *id == oid)).unwrap_or(0)
    }
    fn input_port_index(&self, n: NodeId, iid: InputId) -> usize {
        self.state.graph.nodes.get(n).and_then(|node| node.inputs.iter().position(|(_, id)| *id == iid)).unwrap_or(0)
    }

    /// Cable an output port → input port: update the frontend graph + dispatch `Connect`.
    fn mobile_connect(&mut self, out_node: NodeId, out_port: usize, in_node: NodeId, in_port: usize) {
        let Some(track_id) = self.track_id else { return };
        let output_id = self.state.graph.nodes.get(out_node).and_then(|n| n.outputs.get(out_port)).map(|(_, id)| *id);
        let input_id = self.state.graph.nodes.get(in_node).and_then(|n| n.inputs.get(in_port)).map(|(_, id)| *id);
        let (Some(output_id), Some(input_id)) = (output_id, input_id) else { return };
        if let Some(conns) = self.state.graph.connections.get_mut(input_id) {
            if !conns.contains(&output_id) {
                conns.push(output_id);
            }
        } else {
            self.state.graph.connections.insert(input_id, vec![output_id]);
        }
        if let (Some(&from_id), Some(&to_id)) = (self.node_id_map.get(&out_node), self.node_id_map.get(&in_node)) {
            self.pending_action = Some(Box::new(actions::NodeGraphAction::Connect(
                actions::ConnectAction::new(track_id, from_id, out_port, to_id, in_port),
            )));
        }
    }

    /// Remove a cable: update the frontend graph + dispatch `Disconnect`.
    fn mobile_disconnect(&mut self, out_node: NodeId, out_port: usize, in_node: NodeId, in_port: usize) {
        let Some(track_id) = self.track_id else { return };
        let output_id = self.state.graph.nodes.get(out_node).and_then(|n| n.outputs.get(out_port)).map(|(_, id)| *id);
        let input_id = self.state.graph.nodes.get(in_node).and_then(|n| n.inputs.get(in_port)).map(|(_, id)| *id);
        let (Some(output_id), Some(input_id)) = (output_id, input_id) else { return };
        if let Some(conns) = self.state.graph.connections.get_mut(input_id) {
            conns.retain(|o| *o != output_id);
        }
        if let (Some(&from_id), Some(&to_id)) = (self.node_id_map.get(&out_node), self.node_id_map.get(&in_node)) {
            self.pending_action = Some(Box::new(actions::NodeGraphAction::Disconnect(
                actions::DisconnectAction::new(track_id, from_id, out_port, to_id, in_port),
            )));
        }
    }
}

/// Desktop port color per signal type (matches `DataType::data_type_color`).
fn type_color(t: DataType) -> egui::Color32 {
    match t {
        DataType::Audio => egui::Color32::from_rgb(100, 150, 255), // blue
        DataType::Midi => egui::Color32::from_rgb(100, 255, 100),  // green
        DataType::CV => egui::Color32::from_rgb(255, 150, 100),    // orange
    }
}

/// A single Lucide direction arrow glyph, tinted by signal type.
fn arrow(ui: &mut egui::Ui, glyph: &str, t: DataType) {
    ui.label(
        egui::RichText::new(glyph)
            .color(type_color(t))
            .family(egui::FontFamily::Name(icons::FAMILY.into()))
            .size(15.0),
    );
}

/// A port marker: a Lucide direction arrow (out = from-line, in = to-line) tinted by signal type.
fn port_marker(ui: &mut egui::Ui, t: DataType, dir: PortDir) {
    let glyph = match dir {
        PortDir::In => icons::ARROW_RIGHT_TO_LINE,
        PortDir::Out => icons::ARROW_RIGHT_FROM_LINE,
    };
    arrow(ui, glyph, t);
}

/// A clickable, aligned port arrow (out = from-line, in = to-line) tinted by signal type. Tapping it
/// arms a cable from the port. Frameless so it reads like the desktop port dot.
fn arrow_button(ui: &mut egui::Ui, glyph: &str, t: DataType) -> egui::Response {
    ui.add(
        egui::Button::new(
            egui::RichText::new(glyph)
                .color(type_color(t))
                .family(egui::FontFamily::Name(icons::FAMILY.into()))
                .size(16.0),
        )
        .frame(false),
    )
}

/// A cable badge naming the remote endpoint (`remote-node · remote-port`), framed and tinted by
/// signal type. Returns a click response (used to disconnect the cable).
fn cable_chip(ui: &mut egui::Ui, remote_node: &str, remote_port: &str, t: DataType) -> egui::Response {
    egui::Frame::group(ui.style())
        .inner_margin(egui::Margin::symmetric(6, 1))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(format!("{remote_node} · {remote_port}")).color(type_color(t)));
        })
        .response
        .interact(egui::Sense::click())
}

/// One parameter row: a touch control matching the desktop widget rule (enum → dropdown, ranged →
/// slider, plain → stepper, string → field). Mutates the value in place; dispatch happens later via
/// `check_parameter_changes`.
fn render_param_row(ui: &mut egui::Ui, name: &str, value: &mut ValueType) {
    match value {
        ValueType::Float { value, min, max, unit, enum_labels, .. } => {
            ui.label(name);
            if let Some(labels) = enum_labels {
                let mut sel = (*value as usize).min(labels.len().saturating_sub(1));
                egui::ComboBox::from_id_salt(name)
                    .width(ui.available_width().min(240.0))
                    .selected_text(labels.get(sel).copied().unwrap_or("?"))
                    .show_ui(ui, |ui| {
                        for (i, label) in labels.iter().enumerate() {
                            ui.selectable_value(&mut sel, i, *label);
                        }
                    });
                *value = sel as f32;
            } else if *max > *min {
                // Give the rail a visible color so the full track length reads (the theme's default
                // inactive fill can be near-transparent); trailing_fill shows progress along it.
                ui.scope(|ui| {
                    let rail = egui::Color32::from_gray(90);
                    ui.visuals_mut().widgets.inactive.bg_fill = rail;
                    ui.visuals_mut().widgets.inactive.weak_bg_fill = rail;
                    ui.visuals_mut().widgets.hovered.bg_fill = rail;
                    ui.add(egui::Slider::new(value, *min..=*max).suffix(*unit).trailing_fill(true));
                });
            } else {
                ui.add(egui::DragValue::new(value).speed(0.1));
            }
        }
        ValueType::String { value } => {
            ui.label(name);
            ui.text_edit_singleline(value);
        }
    }
}

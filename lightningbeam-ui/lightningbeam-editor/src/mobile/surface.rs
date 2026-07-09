//! Full-bleed hero surface rendering: maps a mobile surface to an existing pane and renders
//! its content into a rect with no header/divider chrome. This is the de-chromed core of
//! `render_pane` (main.rs) — see the get-or-create + `render_content` pattern there.

use eframe::egui;
use lightningbeam_core::pane::PaneType;

use crate::panes::{NodePath, PaneInstance, PaneRenderer};
use crate::RenderContext;

/// Render `pane_type` full-bleed into `rect`, reusing (or creating) the cached pane instance
/// keyed by `path`. The pane clips its own drawing to `rect`, so no extra clipping is needed.
pub fn render_surface_fullbleed(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    path: &NodePath,
    pane_type: PaneType,
    rc: &mut RenderContext,
) {
    // Get-or-create the instance for this slot (recreate if the type changed).
    let needs_new = rc
        .pane_instances
        .get(path)
        .map(|inst| inst.pane_type() != pane_type)
        .unwrap_or(true);
    if needs_new {
        rc.pane_instances.insert(path.clone(), PaneInstance::new(pane_type));
    }

    if let Some(inst) = rc.pane_instances.get_mut(path) {
        inst.render_content(ui, rect, path, &mut rc.shared);
    }
}

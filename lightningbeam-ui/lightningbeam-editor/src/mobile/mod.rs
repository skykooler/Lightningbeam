//! Mobile / phone UI shell.
//!
//! Developed on desktop behind the `LB_MOBILE_UI` env var (see [`is_mobile_env`]).
//! The shell reuses the existing egui panes (`PaneInstance` / `render_content`) for its
//! "hero" surfaces, composing them as a single surface at a time with a top tab bar, a
//! resizable timeline ribbon, and a fixed transport "floor" at the bottom — per the
//! `phone-ui-sketches.html` wireframe spec.
//!
//! Not every pane becomes a surface: per the spec the Toolbar becomes the omnibutton and
//! the Infopanel becomes the selection inspector (later phases). The tab list here mirrors
//! the *mobile-relevant* subset of `PaneType`.

use eframe::egui;
use lightningbeam_core::pane::PaneType;

use crate::panes::NodePath;
use crate::RenderContext;

mod ribbon;
mod surface;
mod topbar;
mod transport;

/// Reserved sentinel namespace for mobile pane-instance paths. Desktop layout paths are
/// built from small child indices, so prefixing with `usize::MAX` guarantees mobile slots
/// never alias a real layout path in the shared `pane_instances` map.
pub const MOBILE_NS: usize = usize::MAX;

const TOPBAR_H: f32 = 50.0;
const TRANSPORT_H: f32 = 60.0;
const GRABBER_H: f32 = 16.0;

/// Returns true if the mobile UI is requested via the `LB_MOBILE_UI` env var.
/// Any non-empty value other than "0" enables it.
pub fn is_mobile_env() -> bool {
    std::env::var("LB_MOBILE_UI")
        .map(|v| !v.is_empty() && v != "0")
        .unwrap_or(false)
}

/// The hero surfaces selectable from the top tab bar. These mirror the pane list minus the
/// panes that become other mobile affordances (Toolbar → omnibutton, Infopanel → inspector).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobileSurface {
    Stage,
    Time,
    Nodes,
    Mixer,
    Tree,
}

impl MobileSurface {
    /// All surfaces, in tab order.
    pub const TABS: [MobileSurface; 5] = [
        MobileSurface::Stage,
        MobileSurface::Time,
        MobileSurface::Nodes,
        MobileSurface::Mixer,
        MobileSurface::Tree,
    ];

    /// The existing pane that backs this surface. Phase 1 reuses panes directly; the Nodes
    /// surface is a placeholder for the focus/patch rework (it currently shows the desktop
    /// node editor) and Mixer maps to the closest existing audio pane.
    pub fn pane_type(self) -> PaneType {
        match self {
            MobileSurface::Stage => PaneType::Stage,
            MobileSurface::Time => PaneType::Timeline,
            MobileSurface::Nodes => PaneType::NodeEditor,
            MobileSurface::Mixer => PaneType::VirtualPiano,
            MobileSurface::Tree => PaneType::Outliner,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            MobileSurface::Stage => "Stage",
            MobileSurface::Time => "Time",
            MobileSurface::Nodes => "Nodes",
            MobileSurface::Mixer => "Mixer",
            MobileSurface::Tree => "Tree",
        }
    }

    fn index(self) -> usize {
        match self {
            MobileSurface::Stage => 0,
            MobileSurface::Time => 1,
            MobileSurface::Nodes => 2,
            MobileSurface::Mixer => 3,
            MobileSurface::Tree => 4,
        }
    }

    /// Stable `pane_instances` key for this surface's cached pane.
    fn path(self) -> NodePath {
        vec![MOBILE_NS, self.index()]
    }
}

/// How far the timeline ribbon is expanded. The transport floor is always present, so the
/// ribbon never collapses to zero — `Peek` is the floor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RibbonTier {
    Peek,
    Half,
    Full,
}

impl RibbonTier {
    /// Ribbon body height (excluding the grabber) for a given available middle-region
    /// height (the space between the top bar and the transport floor).
    fn height(self, region_h: f32) -> f32 {
        match self {
            RibbonTier::Peek => 96.0_f32.min(region_h * 0.45),
            RibbonTier::Half => region_h * 0.45,
            RibbonTier::Full => region_h * 0.72,
        }
    }

    fn snap_from(region_h: f32, target_h: f32) -> RibbonTier {
        // Snap to whichever tier height is closest to the dragged target.
        let candidates = [RibbonTier::Peek, RibbonTier::Half, RibbonTier::Full];
        *candidates
            .iter()
            .min_by(|a, b| {
                let da = (a.height(region_h) - target_h).abs();
                let db = (b.height(region_h) - target_h).abs();
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or(&RibbonTier::Peek)
    }
}

/// Persistent mobile-shell UI state, cached on `EditorApp`.
pub struct MobileState {
    pub active_surface: MobileSurface,
    pub ribbon_tier: RibbonTier,
    /// Transient live-drag offset (px) applied to the ribbon height while the grabber is
    /// held; folded into `ribbon_tier` on release.
    pub ribbon_drag: f32,
}

impl Default for MobileState {
    fn default() -> Self {
        Self {
            active_surface: MobileSurface::Stage,
            ribbon_tier: RibbonTier::Peek,
            ribbon_drag: 0.0,
        }
    }
}

/// Render the whole mobile shell into `available_rect`. Reuses `rc` (the shared
/// `RenderContext`) for all pane content and `state` for the persistent shell state.
pub fn render_mobile_shell(
    ui: &mut egui::Ui,
    available_rect: egui::Rect,
    rc: &mut RenderContext,
    state: &mut MobileState,
) {
    // Background (wireframe device color; the bands below paint over most of it).
    let bg = egui::Color32::from_rgb(0x14, 0x16, 0x1b);
    ui.painter().rect_filled(available_rect, 0.0, bg);

    let top = available_rect.top();
    let bottom = available_rect.bottom();
    let left = available_rect.left();
    let right = available_rect.right();

    // Fixed bands.
    let topbar_rect = egui::Rect::from_min_max(
        egui::pos2(left, top),
        egui::pos2(right, top + TOPBAR_H),
    );
    let transport_rect = egui::Rect::from_min_max(
        egui::pos2(left, bottom - TRANSPORT_H),
        egui::pos2(right, bottom),
    );

    // Middle region between the top bar and the transport floor.
    let region_top = topbar_rect.bottom();
    let region_bottom = transport_rect.top();
    let region_h = (region_bottom - region_top).max(0.0);

    // The ribbon is hidden when the Time surface is the hero (it already *is* the timeline).
    let show_ribbon = state.active_surface != MobileSurface::Time && region_h > GRABBER_H + 40.0;

    let (hero_rect, ribbon_rects) = if show_ribbon {
        let base_h = state.ribbon_tier.height(region_h);
        let ribbon_h = (base_h + state.ribbon_drag)
            .clamp(40.0, region_h - GRABBER_H - 80.0);
        let grabber_top = region_bottom - ribbon_h - GRABBER_H;
        let grabber_rect = egui::Rect::from_min_max(
            egui::pos2(left, grabber_top),
            egui::pos2(right, grabber_top + GRABBER_H),
        );
        let ribbon_body = egui::Rect::from_min_max(
            egui::pos2(left, grabber_rect.bottom()),
            egui::pos2(right, region_bottom),
        );
        let hero = egui::Rect::from_min_max(
            egui::pos2(left, region_top),
            egui::pos2(right, grabber_top),
        );
        (hero, Some((grabber_rect, ribbon_body)))
    } else {
        let hero = egui::Rect::from_min_max(
            egui::pos2(left, region_top),
            egui::pos2(right, region_bottom),
        );
        (hero, None)
    };

    // Hero surface (reuses an existing pane full-bleed).
    let surface = state.active_surface;
    surface::render_surface_fullbleed(ui, hero_rect, &surface.path(), surface.pane_type(), rc);

    // Resizable timeline ribbon.
    if let Some((grabber_rect, ribbon_body)) = ribbon_rects {
        ribbon::render(ui, grabber_rect, ribbon_body, region_h, state, rc);
    }

    // Top tab bar (drawn after the hero so its hit area wins along the top edge).
    topbar::render(ui, topbar_rect, state, rc);

    // Transport floor (drawn last = always on top, the persistent spine).
    transport::render(ui, transport_rect, &mut rc.shared);
}

/// Cross-platform graphics tablet input support.
///
/// Architecture:
/// - Wayland: secondary event queue + background thread using `zwp_tablet_manager_v2`
/// - X11: x11rb XInput2 raw events for pressure/tilt (cursor already works via OS mouse emulation)
/// - Windows: pressure read from egui `Event::Touch` (winit already converts WM_POINTER)
/// - macOS: cursor/clicks work via mouse emulation; pressure/tilt is future work
///
/// Pressure and tilt are stored in AtomicU32 globals so `make_stroke_point` can read them
/// without needing a context parameter or trait changes.

use std::sync::atomic::{AtomicU32, Ordering};
use eframe::egui;
use lightningbeam_core::tool::Tool;

// ---------------------------------------------------------------------------
// Global tablet state — read by make_stroke_point() on the UI thread
// ---------------------------------------------------------------------------

static TABLET_PRESSURE_BITS: AtomicU32 = AtomicU32::new(0x3f800000); // 1.0f32
static TABLET_TILT_X_BITS: AtomicU32 = AtomicU32::new(0); // 0.0f32
static TABLET_TILT_Y_BITS: AtomicU32 = AtomicU32::new(0); // 0.0f32

/// Current pen pressure (0.0–1.0). Falls back to 1.0 when no tablet is active.
pub fn current_pressure() -> f32 {
    f32::from_bits(TABLET_PRESSURE_BITS.load(Ordering::Relaxed))
}

/// Current pen tilt in radians (tilt_x, tilt_y).
pub fn current_tilt() -> (f32, f32) {
    let x = f32::from_bits(TABLET_TILT_X_BITS.load(Ordering::Relaxed));
    let y = f32::from_bits(TABLET_TILT_Y_BITS.load(Ordering::Relaxed));
    (x, y)
}

fn set_pressure(p: f32) {
    TABLET_PRESSURE_BITS.store(p.to_bits(), Ordering::Relaxed);
}

fn set_tilt(x: f32, y: f32) {
    TABLET_TILT_X_BITS.store(x.to_bits(), Ordering::Relaxed);
    TABLET_TILT_Y_BITS.store(y.to_bits(), Ordering::Relaxed);
}

// ---------------------------------------------------------------------------
// Shared event types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum RawTabletEvent {
    ProximityIn { tool_type: TabletToolType },
    ProximityOut,
    /// Physical pixel coords relative to window surface (Wayland).
    /// On X11 this is unused (cursor comes via OS).
    Motion { x: f64, y: f64 },
    Pressure(f32),
    Tilt { x: f32, y: f32 },
    TipDown,
    TipUp,
    /// End of Wayland tablet event group; commit accumulated state.
    Frame,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum TabletToolType {
    #[default]
    Pen,
    Eraser,
    Other,
}

// ---------------------------------------------------------------------------
// TabletInput — owned by EditorApp
// ---------------------------------------------------------------------------

pub struct TabletInput {
    /// Latest tool position in egui logical pixels relative to the window.
    /// Only set on Wayland (X11/Windows/macOS: cursor already follows OS pointer).
    pub position: Option<egui::Pos2>,
    /// Last known position, persists across frames so PointerButton has a valid position
    /// even when TipDown arrives in a frame without a Motion event.
    last_known_pos: egui::Pos2,
    pub pressure: f32,
    pub tilt: (f32, f32),
    /// `Some(true)` = tip went down this frame; `Some(false)` = tip went up; `None` = no change.
    pub tip_down: Option<bool>,
    pub in_proximity: bool,
    pub was_in_proximity: bool,
    pub tool_type: TabletToolType,
    /// True when the Wayland backend is active (needs PointerButton injection).
    /// On X11, clicks already come through winit via OS mouse emulation.
    inject_buttons: bool,

    /// Pending tool switch (eraser in/out). Consumed by `EditorApp::update()`.
    pub pending_tool_switch: Option<Tool>,
    tool_before_eraser: Option<Tool>,

    /// One-shot sender used to hand the egui Context to the background thread
    /// on the first poll() call so it can call request_repaint().
    repaint_sender: Option<std::sync::mpsc::Sender<egui::Context>>,

    backend: TabletBackend,
}

enum TabletBackend {
    #[cfg(target_os = "linux")]
    Wayland(std::sync::mpsc::Receiver<RawTabletEvent>),
    #[cfg(target_os = "linux")]
    X11(std::sync::mpsc::Receiver<RawTabletEvent>),
    None,
}

impl TabletInput {
    pub fn new(cc: &eframe::CreationContext) -> Self {
        let (backend, inject_buttons, repaint_sender) = Self::init_backend(cc);
        TabletInput {
            position: None,
            last_known_pos: egui::Pos2::ZERO,
            pressure: 1.0,
            tilt: (0.0, 0.0),
            tip_down: None,
            in_proximity: false,
            was_in_proximity: false,
            tool_type: TabletToolType::Pen,
            inject_buttons,
            pending_tool_switch: None,
            tool_before_eraser: None,
            repaint_sender,
            backend,
        }
    }

    #[cfg(target_os = "linux")]
    fn init_backend(
        cc: &eframe::CreationContext,
    ) -> (TabletBackend, bool, Option<std::sync::mpsc::Sender<egui::Context>>) {
        use winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
        let raw = cc.display_handle().ok().map(|h| h.as_raw());
        match raw {
            Some(RawDisplayHandle::Wayland(h)) => {
                let (repaint_tx, repaint_rx) = std::sync::mpsc::channel::<egui::Context>();
                match wayland::init(h, repaint_rx) {
                    // Wayland: winit sees no tablet events at all, inject everything.
                    Some(rx) => (TabletBackend::Wayland(rx), true, Some(repaint_tx)),
                    None     => (TabletBackend::None, false, None),
                }
            }
            Some(RawDisplayHandle::Xlib(h)) => {
                match x11::init_xlib(h) {
                    // X11: OS mouse emulation already sends clicks through winit.
                    Some(rx) => (TabletBackend::X11(rx), false, None),
                    None     => (TabletBackend::None, false, None),
                }
            }
            Some(RawDisplayHandle::Xcb(h)) => {
                match x11::init_xcb(h) {
                    Some(rx) => (TabletBackend::X11(rx), false, None),
                    None     => (TabletBackend::None, false, None),
                }
            }
            _ => (TabletBackend::None, false, None),
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn init_backend(
        _cc: &eframe::CreationContext,
    ) -> (TabletBackend, bool, Option<std::sync::mpsc::Sender<egui::Context>>) {
        (TabletBackend::None, false, None)
    }

    /// Call from `raw_input_hook`. Drains platform events and updates state.
    /// `current_tool` is needed to save the tool before switching to the eraser.
    pub fn poll(
        &mut self,
        ctx: &egui::Context,
        raw_input: &mut egui::RawInput,
        current_tool: Tool,
    ) {
        // On the first poll() we have the egui Context available. Hand it to the
        // background thread so it can call request_repaint() and wake the event loop.
        if let Some(tx) = self.repaint_sender.take() {
            // send() is non-blocking on an unbounded channel.
            let _ = tx.send(ctx.clone());
        }

        self.was_in_proximity = self.in_proximity;
        self.tip_down = None;
        self.position = None;

        // Windows: read pressure from egui Touch events (winit converts WM_POINTER).
        #[cfg(target_os = "windows")]
        self.poll_windows(ctx);

        // Linux: drain the platform event channel.
        #[cfg(target_os = "linux")]
        self.poll_linux(ctx);

        // Inject synthetic egui events on Wayland (X11/Windows cursor moves via OS).
        self.inject_events(raw_input);

        // Handle eraser tool switch.
        self.handle_tool_switch(current_tool);

        // Publish globals for make_stroke_point().
        set_pressure(self.pressure);
        let (tx, ty) = self.tilt;
        set_tilt(tx, ty);
    }

    #[cfg(target_os = "windows")]
    fn poll_windows(&mut self, ctx: &egui::Context) {
        let pressure = ctx.input(|i| {
            i.events.iter().rev().find_map(|e| {
                if let egui::Event::Touch {
                    force: Some(f),
                    ..
                } = e
                {
                    Some(*f)
                } else {
                    None
                }
            })
        });
        if let Some(p) = pressure {
            self.pressure = p;
            if !self.in_proximity {
                self.in_proximity = true;
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn poll_linux(&mut self, ctx: &egui::Context) {
        use std::sync::mpsc::TryRecvError;

        let pixels_per_point = ctx.pixels_per_point();

        let rx = match &self.backend {
            TabletBackend::Wayland(rx) => rx as *const std::sync::mpsc::Receiver<RawTabletEvent>,
            TabletBackend::X11(rx) => rx as *const std::sync::mpsc::Receiver<RawTabletEvent>,
            TabletBackend::None => return,
        };
        // SAFETY: we own self exclusively; the raw pointer is valid for this call duration.
        let rx = unsafe { &*rx };

        loop {
            match rx.try_recv() {
                Ok(event) => self.apply_event(event, pixels_per_point),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    // Background thread exited; degrade gracefully.
                    self.backend = TabletBackend::None;
                    break;
                }
            }
        }
    }

    fn apply_event(&mut self, event: RawTabletEvent, pixels_per_point: f32) {
        match event {
            RawTabletEvent::ProximityIn { tool_type } => {
                self.tool_type = tool_type;
                self.in_proximity = true;
            }
            RawTabletEvent::ProximityOut => {
                self.in_proximity = false;
                self.pressure = 0.0;
            }
            RawTabletEvent::Motion { x, y } => {
                // Wayland gives physical pixels; convert to egui logical points.
                let lx = x as f32 / pixels_per_point;
                let ly = y as f32 / pixels_per_point;
                let pos = egui::pos2(lx, ly);
                self.position = Some(pos);
                self.last_known_pos = pos;
            }
            RawTabletEvent::Pressure(p) => {
                self.pressure = p;
            }
            RawTabletEvent::Tilt { x, y } => {
                // Convert degrees to radians for StrokePoint.
                self.tilt = (x.to_radians(), y.to_radians());
            }
            RawTabletEvent::TipDown => {
                self.tip_down = Some(true);
            }
            RawTabletEvent::TipUp => {
                self.tip_down = Some(false);
            }
            RawTabletEvent::Frame => {
                // Frame is the commit signal; processing already happened in individual events.
            }
        }
    }

    fn inject_events(&self, raw_input: &mut egui::RawInput) {
        if let Some(pos) = self.position {
            raw_input.events.push(egui::Event::PointerMoved(pos));
        }
        // Only inject button events on Wayland. On X11 the OS mouse emulation already
        // sends clicks through winit; injecting here would cause duplicate events.
        if self.inject_buttons {
            if let Some(pressed) = self.tip_down {
                // Use last_known_pos so clicks work even when TipDown arrives without
                // a simultaneous Motion event (pen stationary when touching).
                raw_input.events.push(egui::Event::PointerButton {
                    pos: self.last_known_pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: raw_input.modifiers,
                });
            }
        }
        if !self.in_proximity && self.was_in_proximity {
            raw_input.events.push(egui::Event::PointerGone);
        }
    }

    fn handle_tool_switch(&mut self, current_tool: Tool) {
        let just_entered = self.in_proximity && !self.was_in_proximity;
        let just_left = !self.in_proximity && self.was_in_proximity;

        if just_entered && self.tool_type == TabletToolType::Eraser {
            // Save the current tool so we can restore it later.
            if current_tool != Tool::Erase {
                self.tool_before_eraser = Some(current_tool);
            }
            self.pending_tool_switch = Some(Tool::Erase);
        } else if just_left && self.tool_type == TabletToolType::Eraser {
            // Restore previous tool.
            if let Some(prev) = self.tool_before_eraser.take() {
                self.pending_tool_switch = Some(prev);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Wayland backend
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
mod wayland {
    use super::{RawTabletEvent, TabletToolType};
    use eframe::egui;
    use std::collections::HashMap;
    use std::sync::mpsc;
    use winit::raw_window_handle::WaylandDisplayHandle;

    use wayland_client::{
        globals::registry_queue_init,
        protocol::{wl_registry, wl_seat},
        Connection, Dispatch, QueueHandle, delegate_noop, event_created_child,
    };
    use wayland_backend::sys::client::Backend;
    use wayland_protocols::wp::tablet::zv2::client::{
        zwp_tablet_manager_v2, zwp_tablet_seat_v2, zwp_tablet_tool_v2,
        zwp_tablet_v2, zwp_tablet_pad_v2, zwp_tablet_pad_ring_v2,
        zwp_tablet_pad_strip_v2, zwp_tablet_pad_group_v2,
    };

    /// Attempt to connect to the Wayland tablet protocol.
    /// Returns `None` if the compositor doesn't support `zwp_tablet_manager_v2`.
    /// `repaint_rx` receives the egui Context from the UI thread on its first poll(),
    /// after which the background thread calls `ctx.request_repaint()` on every tablet frame
    /// so the event loop wakes up and drains the channel.
    pub fn init(
        handle: WaylandDisplayHandle,
        repaint_rx: mpsc::Receiver<egui::Context>,
    ) -> Option<mpsc::Receiver<RawTabletEvent>> {
        let (tx, rx) = mpsc::channel();

        let ptr = handle.display.as_ptr() as *mut wayland_sys::client::wl_display;
        // SAFETY: `handle.display` is a valid `wl_display *` owned by winit.
        let backend = unsafe { Backend::from_foreign_display(ptr) };
        let conn = Connection::from_backend(backend);

        let (globals, mut event_queue) = match registry_queue_init::<TabletDispatch>(&conn) {
            Ok(result) => result,
            Err(_) => return None,
        };

        let qh = event_queue.handle();

        // Bind wl_seat (required to obtain a zwp_tablet_seat_v2).
        let seat: wl_seat::WlSeat = match globals.bind(&qh, 1..=8, ()) {
            Ok(s) => s,
            Err(_) => return None,
        };

        // Bind the tablet manager.
        let mgr: zwp_tablet_manager_v2::ZwpTabletManagerV2 = match globals.bind(&qh, 1..=1, ()) {
            Ok(m) => m,
            Err(_) => return None,
        };

        // Create the tablet seat, which will start delivering tool_added events.
        let _tablet_seat = mgr.get_tablet_seat(&seat, &qh, ());

        // Initial roundtrip to discover pre-existing tools.
        let mut dispatch = TabletDispatch {
            tx: tx.clone(),
            tools: HashMap::new(),
        };

        if event_queue.roundtrip(&mut dispatch).is_err() {
            return None;
        }

        // Spawn background thread; owns the event queue.
        std::thread::Builder::new()
            .name("lightningbeam-tablet-wayland".into())
            .spawn(move || {
                // Wait for the UI thread to hand us the egui Context (happens on first poll()).
                let repaint_ctx: Option<egui::Context> = repaint_rx.recv().ok();

                loop {
                    if event_queue.blocking_dispatch(&mut dispatch).is_err() {
                        break;
                    }
                    // Wake the egui event loop so raw_input_hook runs and drains our channel.
                    if let Some(ref ctx) = repaint_ctx {
                        ctx.request_repaint();
                    }
                }
            })
            .ok()?;

        Some(rx)
    }

    // -----------------------------------------------------------------------
    // Per-tool accumulator
    // -----------------------------------------------------------------------

    #[derive(Default)]
    struct ToolAccumulator {
        tool_type: TabletToolType,
        pending_x: f64,
        pending_y: f64,
        pending_pressure: f32,
        pending_tilt: (f32, f32),
        pending_motion: bool,
        pending_tip: Option<bool>,
        pending_proximity: Option<bool>,
    }

    // -----------------------------------------------------------------------
    // Dispatch state
    // -----------------------------------------------------------------------

    struct TabletDispatch {
        tx: mpsc::Sender<RawTabletEvent>,
        tools: HashMap<zwp_tablet_tool_v2::ZwpTabletToolV2, ToolAccumulator>,
    }

    // --- wl_registry (required by registry_queue_init) ---
    impl Dispatch<wl_registry::WlRegistry, wayland_client::globals::GlobalListContents>
        for TabletDispatch
    {
        fn event(
            _state: &mut Self,
            _proxy: &wl_registry::WlRegistry,
            _event: wl_registry::Event,
            _data: &wayland_client::globals::GlobalListContents,
            _conn: &Connection,
            _qh: &QueueHandle<Self>,
        ) {
        }
    }

    delegate_noop!(TabletDispatch: ignore wl_seat::WlSeat);
    delegate_noop!(TabletDispatch: ignore zwp_tablet_manager_v2::ZwpTabletManagerV2);
    delegate_noop!(TabletDispatch: ignore zwp_tablet_v2::ZwpTabletV2);
    delegate_noop!(TabletDispatch: ignore zwp_tablet_pad_ring_v2::ZwpTabletPadRingV2);
    delegate_noop!(TabletDispatch: ignore zwp_tablet_pad_strip_v2::ZwpTabletPadStripV2);

    // --- zwp_tablet_seat_v2: receives tool_added, tablet_added, pad_added ---
    impl Dispatch<zwp_tablet_seat_v2::ZwpTabletSeatV2, ()> for TabletDispatch {
        fn event(
            state: &mut Self,
            _proxy: &zwp_tablet_seat_v2::ZwpTabletSeatV2,
            event: zwp_tablet_seat_v2::Event,
            _data: &(),
            _conn: &Connection,
            _qh: &QueueHandle<Self>,
        ) {
            match event {
                zwp_tablet_seat_v2::Event::ToolAdded { id } => {
                    state.tools.entry(id).or_default();
                }
                zwp_tablet_seat_v2::Event::TabletAdded { .. } => {}
                zwp_tablet_seat_v2::Event::PadAdded { .. } => {}
                _ => {}
            }
        }

        event_created_child!(TabletDispatch, zwp_tablet_seat_v2::ZwpTabletSeatV2, [
            zwp_tablet_seat_v2::EVT_TABLET_ADDED_OPCODE => (zwp_tablet_v2::ZwpTabletV2, ()),
            zwp_tablet_seat_v2::EVT_TOOL_ADDED_OPCODE   => (zwp_tablet_tool_v2::ZwpTabletToolV2, ()),
            zwp_tablet_seat_v2::EVT_PAD_ADDED_OPCODE    => (zwp_tablet_pad_v2::ZwpTabletPadV2, ()),
        ]);
    }

    // --- zwp_tablet_pad_v2: announces groups via new-id 'group' event ---
    impl Dispatch<zwp_tablet_pad_v2::ZwpTabletPadV2, ()> for TabletDispatch {
        fn event(
            _state: &mut Self,
            _proxy: &zwp_tablet_pad_v2::ZwpTabletPadV2,
            _event: zwp_tablet_pad_v2::Event,
            _data: &(),
            _conn: &Connection,
            _qh: &QueueHandle<Self>,
        ) {
        }

        event_created_child!(TabletDispatch, zwp_tablet_pad_v2::ZwpTabletPadV2, [
            zwp_tablet_pad_v2::EVT_GROUP_OPCODE => (zwp_tablet_pad_group_v2::ZwpTabletPadGroupV2, ()),
        ]);
    }

    // --- zwp_tablet_pad_group_v2: announces rings and strips via new-id events ---
    impl Dispatch<zwp_tablet_pad_group_v2::ZwpTabletPadGroupV2, ()> for TabletDispatch {
        fn event(
            _state: &mut Self,
            _proxy: &zwp_tablet_pad_group_v2::ZwpTabletPadGroupV2,
            _event: zwp_tablet_pad_group_v2::Event,
            _data: &(),
            _conn: &Connection,
            _qh: &QueueHandle<Self>,
        ) {
        }

        event_created_child!(TabletDispatch, zwp_tablet_pad_group_v2::ZwpTabletPadGroupV2, [
            zwp_tablet_pad_group_v2::EVT_RING_OPCODE  => (zwp_tablet_pad_ring_v2::ZwpTabletPadRingV2, ()),
            zwp_tablet_pad_group_v2::EVT_STRIP_OPCODE => (zwp_tablet_pad_strip_v2::ZwpTabletPadStripV2, ()),
        ]);
    }

    // --- zwp_tablet_tool_v2: the core tablet event stream ---
    impl Dispatch<zwp_tablet_tool_v2::ZwpTabletToolV2, ()> for TabletDispatch {
        fn event(
            state: &mut Self,
            proxy: &zwp_tablet_tool_v2::ZwpTabletToolV2,
            event: zwp_tablet_tool_v2::Event,
            _data: &(),
            _conn: &Connection,
            _qh: &QueueHandle<Self>,
        ) {
            use zwp_tablet_tool_v2::Event;

            let acc = state.tools.entry(proxy.clone()).or_default();

            match event {
                Event::Type { tool_type } => {
                    acc.tool_type = match tool_type.into_result() {
                        Ok(zwp_tablet_tool_v2::Type::Pen)    => TabletToolType::Pen,
                        Ok(zwp_tablet_tool_v2::Type::Eraser) => TabletToolType::Eraser,
                        _                                     => TabletToolType::Other,
                    };
                }
                Event::ProximityIn { .. } => {
                    acc.pending_proximity = Some(true);
                }
                Event::ProximityOut => {
                    acc.pending_proximity = Some(false);
                }
                Event::Down { .. } => {
                    acc.pending_tip = Some(true);
                }
                Event::Up => {
                    acc.pending_tip = Some(false);
                }
                Event::Motion { x, y } => {
                    // wayland-client already decodes wl_fixed_t to f64 (physical pixels).
                    acc.pending_x = x;
                    acc.pending_y = y;
                    acc.pending_motion = true;
                }
                Event::Pressure { pressure } => {
                    acc.pending_pressure = pressure as f32 / 65535.0;
                }
                Event::Tilt { tilt_x, tilt_y } => {
                    acc.pending_tilt = (tilt_x as f32, tilt_y as f32);
                }
                Event::Frame { .. } => {
                    // Flush accumulated events to the channel.
                    if let Some(prox) = acc.pending_proximity.take() {
                        if prox {
                            let _ = state.tx.send(RawTabletEvent::ProximityIn {
                                tool_type: acc.tool_type,
                            });
                        } else {
                            let _ = state.tx.send(RawTabletEvent::ProximityOut);
                        }
                    }
                    if acc.pending_motion {
                        let _ = state.tx.send(RawTabletEvent::Motion {
                            x: acc.pending_x,
                            y: acc.pending_y,
                        });
                        let _ = state.tx.send(RawTabletEvent::Pressure(acc.pending_pressure));
                        let (tx, ty) = acc.pending_tilt;
                        let _ = state.tx.send(RawTabletEvent::Tilt { x: tx, y: ty });
                        acc.pending_motion = false;
                    }
                    if let Some(tip) = acc.pending_tip.take() {
                        let _ = state.tx.send(if tip {
                            RawTabletEvent::TipDown
                        } else {
                            RawTabletEvent::TipUp
                        });
                    }
                    let _ = state.tx.send(RawTabletEvent::Frame);
                }
                Event::Removed => {
                    state.tools.remove(proxy);
                }
                _ => {}
            }
        }
    }
}

// ---------------------------------------------------------------------------
// X11 backend — XInput2 for pressure + tilt
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
mod x11 {
    use super::{RawTabletEvent, TabletToolType};
    use std::sync::mpsc;
    use winit::raw_window_handle::{XcbDisplayHandle, XlibDisplayHandle};

    /// Init from an Xlib display handle. Opens a second X11 connection for XI2 raw events.
    pub fn init_xlib(_handle: XlibDisplayHandle) -> Option<mpsc::Receiver<RawTabletEvent>> {
        // Both Xlib and XCB paths create a fresh independent connection (X11 allows N connections).
        init_inner()
    }

    /// Init from an XCB display handle.
    pub fn init_xcb(_handle: XcbDisplayHandle) -> Option<mpsc::Receiver<RawTabletEvent>> {
        init_inner()
    }

    fn init_inner() -> Option<mpsc::Receiver<RawTabletEvent>> {
        use x11rb::connection::Connection;
        use x11rb::protocol::xinput;
        use x11rb::protocol::xinput::ConnectionExt as XInputExt;
        use x11rb::rust_connection::RustConnection;

        let (tx, rx) = mpsc::channel();

        // Open a dedicated connection for XI2 event listening.
        let (conn, screen_num) = match RustConnection::connect(None) {
            Ok(c) => c,
            Err(_) => return None,
        };

        // Check XI2 availability.
        match conn.xinput_xi_query_version(2, 2) {
            Ok(reply) => {
                if reply.reply().map(|r| r.major_version < 2).unwrap_or(true) {
                    return None;
                }
            }
            Err(_) => return None,
        }

        let root = conn.setup().roots[screen_num].root;

        // Discover stylus/eraser devices and their pressure/tilt axes.
        let device_axes = match discover_devices(&conn) {
            Ok(d) => d,
            Err(_) => return None,
        };

        if device_axes.is_empty() {
            return None;
        }

        // Select XI2 raw motion + button events on the root window.
        let masks: Vec<xinput::EventMask> = device_axes
            .keys()
            .map(|&devid| xinput::EventMask {
                deviceid: devid,
                mask: vec![
                    (xinput::XIEventMask::RAW_MOTION
                        | xinput::XIEventMask::RAW_BUTTON_PRESS
                        | xinput::XIEventMask::RAW_BUTTON_RELEASE)
                        .into(),
                ],
            })
            .collect();

        if conn.xinput_xi_select_events(root, &masks).is_err() {
            return None;
        }
        let _ = conn.flush();

        std::thread::Builder::new()
            .name("lightningbeam-tablet-x11".into())
            .spawn(move || {
                event_loop(conn, device_axes, tx);
            })
            .ok()?;

        Some(rx)
    }

    // -----------------------------------------------------------------------
    // Per-device axis mapping
    // -----------------------------------------------------------------------

    #[derive(Debug, Clone)]
    struct DeviceAxes {
        pressure_axis: Option<usize>,
        tilt_x_axis: Option<usize>,
        tilt_y_axis: Option<usize>,
        tool_type: TabletToolType,
        /// Range of pressure axis (min, max) for normalisation.
        pressure_range: (f64, f64),
        tilt_range: (f64, f64),
    }

    fn discover_devices(
        conn: &x11rb::rust_connection::RustConnection,
    ) -> Result<std::collections::HashMap<u16, DeviceAxes>, x11rb::errors::ReplyError> {
        use x11rb::protocol::xinput;
        use x11rb::protocol::xinput::ConnectionExt as XInputExt;
        use x11rb::protocol::xproto::ConnectionExt as XprotoExt;
        #[allow(unused_imports)]
        use x11rb::connection::Connection;

        let mut result = std::collections::HashMap::new();

        // 0 = XIAllDevices
        let devices = conn
            .xinput_xi_query_device(0u16)?
            .reply()?;

        // Intern atoms we need.
        let atom_pressure = conn.intern_atom(false, b"Abs Pressure")?.reply()?.atom;
        let atom_tilt_x   = conn.intern_atom(false, b"Abs Tilt X")?.reply()?.atom;
        let atom_tilt_y   = conn.intern_atom(false, b"Abs Tilt Y")?.reply()?.atom;

        for dev in &devices.infos {
            let name = std::str::from_utf8(&dev.name).unwrap_or("").to_lowercase();
            let is_stylus = name.contains("stylus") || name.contains("pen");
            let is_eraser = name.contains("eraser");

            if !is_stylus && !is_eraser {
                continue;
            }

            let mut pressure_axis = None;
            let mut tilt_x_axis = None;
            let mut tilt_y_axis = None;
            let mut pressure_range = (0.0_f64, 65535.0_f64);
            let mut tilt_range = (-64.0_f64, 63.0_f64);

            for (idx, class) in dev.classes.iter().enumerate() {
                if let xinput::DeviceClassData::Valuator(v) = &class.data {
                    if v.label == atom_pressure {
                        pressure_axis = Some(idx);
                        pressure_range = (v.min.integral as f64, v.max.integral as f64);
                    } else if v.label == atom_tilt_x {
                        tilt_x_axis = Some(idx);
                        tilt_range = (v.min.integral as f64, v.max.integral as f64);
                    } else if v.label == atom_tilt_y {
                        tilt_y_axis = Some(idx);
                    }
                }
            }

            if pressure_axis.is_some() || tilt_x_axis.is_some() {
                result.insert(
                    dev.deviceid,
                    DeviceAxes {
                        pressure_axis,
                        tilt_x_axis,
                        tilt_y_axis,
                        tool_type: if is_eraser {
                            TabletToolType::Eraser
                        } else {
                            TabletToolType::Pen
                        },
                        pressure_range,
                        tilt_range,
                    },
                );
            }
        }

        Ok(result)
    }

    // -----------------------------------------------------------------------
    // Event loop (background thread)
    // -----------------------------------------------------------------------

    fn event_loop(
        conn: x11rb::rust_connection::RustConnection,
        device_axes: std::collections::HashMap<u16, DeviceAxes>,
        tx: mpsc::Sender<RawTabletEvent>,
    ) {
        use x11rb::connection::Connection;
        #[allow(unused_imports)]
        use x11rb::protocol::xinput;
        use x11rb::protocol::Event;

        // Track which devices are in proximity.
        let mut in_proximity: std::collections::HashSet<u16> = std::collections::HashSet::new();

        loop {
            let event = match conn.wait_for_event() {
                Ok(e) => e,
                Err(_) => break,
            };

            match event {
                Event::XinputRawMotion(raw) => {
                    let devid = raw.deviceid;
                    let axes = match device_axes.get(&devid) {
                        Some(a) => a,
                        None => continue,
                    };

                    // Synthesise proximity in when we first see motion from a device.
                    if !in_proximity.contains(&devid) {
                        in_proximity.insert(devid);
                        let _ = tx.send(RawTabletEvent::ProximityIn {
                            tool_type: axes.tool_type,
                        });
                    }

                    let valuators = &raw.axisvalues;

                    // Pressure
                    if let Some(idx) = axes.pressure_axis {
                        if let Some(v) = valuators.get(idx) {
                            let norm = normalize(
                                v.integral as f64 + v.frac as f64 / 65536.0,
                                axes.pressure_range.0,
                                axes.pressure_range.1,
                            );
                            let _ = tx.send(RawTabletEvent::Pressure(norm as f32));
                        }
                    }

                    // Tilt
                    let tx_deg = axes.tilt_x_axis.and_then(|i| valuators.get(i)).map(|v| {
                        map_range(
                            v.integral as f64 + v.frac as f64 / 65536.0,
                            axes.tilt_range.0,
                            axes.tilt_range.1,
                            -90.0,
                            90.0,
                        ) as f32
                    });
                    let ty_deg = axes.tilt_y_axis.and_then(|i| valuators.get(i)).map(|v| {
                        map_range(
                            v.integral as f64 + v.frac as f64 / 65536.0,
                            axes.tilt_range.0,
                            axes.tilt_range.1,
                            -90.0,
                            90.0,
                        ) as f32
                    });
                    if tx_deg.is_some() || ty_deg.is_some() {
                        let _ = tx.send(RawTabletEvent::Tilt {
                            x: tx_deg.unwrap_or(0.0),
                            y: ty_deg.unwrap_or(0.0),
                        });
                    }

                    let _ = tx.send(RawTabletEvent::Frame);
                }

                Event::XinputRawButtonPress(raw) => {
                    if device_axes.contains_key(&raw.deviceid) && raw.detail == 1 {
                        let _ = tx.send(RawTabletEvent::TipDown);
                        let _ = tx.send(RawTabletEvent::Frame);
                    }
                }

                Event::XinputRawButtonRelease(raw) => {
                    if device_axes.contains_key(&raw.deviceid) {
                        if raw.detail == 1 {
                            let _ = tx.send(RawTabletEvent::TipUp);
                            let _ = tx.send(RawTabletEvent::Frame);
                        }
                        // When all buttons released, synthesise proximity out.
                        in_proximity.remove(&raw.deviceid);
                        let _ = tx.send(RawTabletEvent::ProximityOut);
                        let _ = tx.send(RawTabletEvent::Frame);
                    }
                }

                _ => {}
            }
        }
    }

    fn normalize(val: f64, min: f64, max: f64) -> f64 {
        if (max - min).abs() < 1e-9 {
            return 1.0;
        }
        ((val - min) / (max - min)).clamp(0.0, 1.0)
    }

    fn map_range(val: f64, in_min: f64, in_max: f64, out_min: f64, out_max: f64) -> f64 {
        if (in_max - in_min).abs() < 1e-9 {
            return out_min;
        }
        (val - in_min) / (in_max - in_min) * (out_max - out_min) + out_min
    }
}

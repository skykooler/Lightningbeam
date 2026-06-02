use std::sync::Arc;
use std::thread;
use winit::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop, EventLoopBuilder, EventLoopProxy},
    window::WindowBuilder,
};

#[cfg(target_os = "linux")]
use winit::platform::x11::EventLoopBuilderExtX11;

use crate::renderer::Renderer;

/// Events that can be sent to the render window thread
#[derive(Debug, Clone)]
pub enum RenderEvent {
    UpdateGradient { top: [f32; 4], bottom: [f32; 4] },
    SetPosition { x: i32, y: i32 },
    SetSize { width: u32, height: u32 },
    RequestRedraw,
    Close,
}

/// Handle to control the render window from other threads
pub struct RenderWindowHandle {
    proxy: EventLoopProxy<RenderEvent>,
}

impl RenderWindowHandle {
    /// Update the gradient colors
    pub fn update_gradient(&self, top: [f32; 4], bottom: [f32; 4]) {
        let _ = self.proxy.send_event(RenderEvent::UpdateGradient { top, bottom });
    }

    /// Set window position
    pub fn set_position(&self, x: i32, y: i32) {
        let _ = self.proxy.send_event(RenderEvent::SetPosition { x, y });
    }

    /// Set window size
    pub fn set_size(&self, width: u32, height: u32) {
        let _ = self.proxy.send_event(RenderEvent::SetSize { width, height });
    }

    /// Request a redraw
    pub fn request_redraw(&self) {
        let _ = self.proxy.send_event(RenderEvent::RequestRedraw);
    }

    /// Close the render window
    pub fn close(&self) {
        let _ = self.proxy.send_event(RenderEvent::Close);
    }
}

/// Spawn the render window in a separate thread
pub fn spawn_render_window(
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> Result<RenderWindowHandle, String> {
    let (tx, rx) = std::sync::mpsc::channel();

    thread::spawn(move || {
        let mut event_loop_builder = EventLoopBuilder::with_user_event();

        // On Linux, allow event loop on any thread (not just main thread)
        #[cfg(target_os = "linux")]
        {
            event_loop_builder.with_any_thread(true);
        }

        let event_loop: EventLoop<RenderEvent> = event_loop_builder.build().unwrap();
        let proxy = event_loop.create_proxy();

        // Send the proxy back to the main thread
        tx.send(proxy.clone()).unwrap();

        let window = WindowBuilder::new()
            .with_title("Lightningbeam Renderer")
            .with_inner_size(winit::dpi::PhysicalSize::new(width, height))
            .with_position(winit::dpi::PhysicalPosition::new(x, y))
            .with_decorations(false) // No title bar
            .with_transparent(false) // Opaque background
            .with_resizable(false)
            .build(&event_loop)
            .unwrap();

        let window = Arc::new(window);

        // Initialize renderer (async operation)
        let mut renderer = pollster::block_on(Renderer::new(window.clone()));

        event_loop.run(move |event, elwt| {
            elwt.set_control_flow(ControlFlow::Wait);

            match event {
                Event::UserEvent(render_event) => match render_event {
                    RenderEvent::UpdateGradient { top, bottom } => {
                        eprintln!("[RenderWindow] Updating gradient: {:?} -> {:?}", top, bottom);
                        renderer.update_gradient(top, bottom);
                        window.request_redraw();
                    }
                    RenderEvent::SetPosition { x, y } => {
                        eprintln!("[RenderWindow] Setting position: ({}, {})", x, y);
                        let _ = window.set_outer_position(winit::dpi::PhysicalPosition::new(x, y));
                    }
                    RenderEvent::SetSize { width, height } => {
                        eprintln!("[RenderWindow] Setting size: {}x{}", width, height);
                        let _ = window.request_inner_size(winit::dpi::PhysicalSize::new(width, height));
                    }
                    RenderEvent::RequestRedraw => {
                        window.request_redraw();
                    }
                    RenderEvent::Close => {
                        eprintln!("[RenderWindow] Closing render window");
                        elwt.exit();
                    }
                },

                Event::WindowEvent {
                    event: WindowEvent::CloseRequested,
                    ..
                } => {
                    elwt.exit();
                }

                Event::WindowEvent {
                    event: WindowEvent::Resized(physical_size),
                    ..
                } => {
                    renderer.resize(physical_size);
                    window.request_redraw();
                }

                Event::WindowEvent {
                    event: WindowEvent::RedrawRequested,
                    ..
                } => {
                    match renderer.render() {
                        Ok(_) => {}
                        Err(wgpu::SurfaceError::Lost) => renderer.resize(window.inner_size()),
                        Err(wgpu::SurfaceError::OutOfMemory) => {
                            eprintln!("Out of memory!");
                            elwt.exit();
                        }
                        Err(e) => eprintln!("Render error: {:?}", e),
                    }
                }

                _ => {}
            }
        }).expect("Event loop error");
    });

    // Wait for the proxy to be sent back
    let proxy = rx.recv().map_err(|e| format!("Failed to receive proxy: {}", e))?;

    Ok(RenderWindowHandle { proxy })
}

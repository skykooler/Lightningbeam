//! Zero-copy GPU video encoding.
//!
//! Converts a rendered RGBA texture to the encoder's pixel format (NV12) on the GPU
//! and feeds it to a hardware video encoder without a CPU round-trip. All the unsafe
//! GPU↔encoder interop (Vulkan external memory / DMA-BUF → VAAPI on Linux, etc.) is
//! isolated in this crate.
//!
//! Status: scaffolding. Headless GPU probe + (next) NV12 compute live here first so
//! the GPU-side conversion can be validated against a CPU reference before any unsafe
//! interop is written. See `lightningbeam-ui/ZEROCOPY_GPU_ENCODE_PLAN.md`.

pub mod nv12;

/// Fragment-shader RGBA→NV12 conversion that renders into plane textures.
pub mod render_nv12;

/// VAAPI hardware encode (Linux-only; libva).
#[cfg(target_os = "linux")]
pub mod vaapi;

/// Custom Vulkan device with DMA-BUF import extensions (Linux).
#[cfg(target_os = "linux")]
pub mod vk_device;

/// Import a VAAPI NV12 DMA-BUF as wgpu textures (Linux).
#[cfg(target_os = "linux")]
pub mod dmabuf;

/// End-to-end zero-copy `h264_vaapi` encoder (Linux).
#[cfg(target_os = "linux")]
pub mod encoder;

/// VAAPI hardware decode → wgpu textures (Linux).
#[cfg(target_os = "linux")]
pub mod decoder;

#[cfg(test)]
mod probe_tests {
    /// Confirm a headless GPU adapter is reachable (Vulkan on Linux/Intel). This gates
    /// whether the GPU-side conversion can be tested on real hardware in this env.
    #[test]
    fn headless_adapter_available() {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN | wgpu::Backends::GL,
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(
            &wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
                compatible_surface: None,
            },
        ));
        match adapter {
            Ok(a) => {
                let info = a.get_info();
                eprintln!(
                    "[gpu-probe] adapter: {} | backend={:?} | type={:?} | driver={}",
                    info.name, info.backend, info.device_type, info.driver
                );
            }
            Err(e) => panic!("no GPU adapter available headless: {e:?}"),
        }
    }
}

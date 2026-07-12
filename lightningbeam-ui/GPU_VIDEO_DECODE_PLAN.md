# GPU-resident video decode + dynamic decode resolution

## Context

Profiling the zero-copy H.264 export (single Group[Video, Audio] clip, `LB_RENDER_PROFILE=1`)
broke the per-frame CPU "render" bucket down as:

| Cost (ms/frame)         | 1080p | 4K    | What it is                                          |
|-------------------------|-------|-------|-----------------------------------------------------|
| decode                  | 3.1   | 19.0  | software ffmpeg decode (`video.rs::get_frame`)      |
| background re-render     | 3.6   | 7.5   | static background pushed through Vello *every frame* |
| video upload + blit      | 4.1   | 4.2   | per-frame transient texture alloc + `write_texture`  |
| srgb                    | 0.4   | 0.4   | linear→sRGB pass                                     |

The video correctly takes the GPU Video-instance path (not Vello-baked) — `LB_LAYER_DEBUG=1`
shows `Video (1 instance)`. So the cost is **the video frame itself**: software decode, then an
8 MB `write_texture` upload of the decoded RGBA every frame. At 4K, software decode (19 ms)
dominates everything.

### Two correctness problems found alongside the perf issue

1. **Decode resolution is frozen to document size at import.** `load_video(clip, src, doc_w, doc_h)`
   (`main.rs:4302`) sizes the decoder's swscale output to the document, capped to never upscale
   (`video.rs:149`). Export *reuses that decoder*, so exporting **above** document resolution yields
   video that was decoded to ≤document res and then GPU-**up**scaled — real source detail thrown away.
2. **It can't follow the consumer or a document resize.** Preview wants small/fast frames; export
   wants full res; changing the document size should re-target the decode. None of that works with a
   size frozen at import.

## Goal

Decouple **decode resolution** from import/document size: the renderer requests a frame *at a target
resolution*, and the decode path produces it. Hardware-decode H.264 (and later HEVC/AV1) into a GPU
surface and keep it GPU-resident through composite into the encoder — no CPU frame copy in either
direction. Software decode stays a **first-class** path (codecs/platforms without HW support), decoding
at the requested target res. This fixes the 4K decode wall, the 8 MB upload, *and* the resolution bugs.

## Design principles

- **Decode native, scale to the consumer's target.**
  - *Hardware path:* decode into a native VAAPI surface → import as a wgpu texture (reuse the
    `gpu-video-encoder` `dmabuf.rs` / `vk_device.rs` plumbing, read direction) → the GPU blit that
    already composites the Video instance scales native→target for free. Handles any target res and
    document resizes inherently; the cached frame is a native GPU texture.
  - *Software path:* decode native → `swscale` to the requested target (the reusable scaler is keyed
    on input format/size **and** output size — rebuilt when the target changes). Preview requests
    preview res (cheap); export requests export res (full quality).
- **`VideoManager::get_frame` takes a target `(w, h)`** instead of relying on a frozen output size.
  The frame cache is keyed to handle multiple live targets (preview + export) — either cache native
  frames and scale on demand, or key by `(clip, ts, target)`; decide in Stage 2 by measuring cache
  hit/scale tradeoff.
- **Software is not optional.** Hardware decode is an acceleration of the same `get_frame` contract,
  selected per source when the codec/driver supports it; everything falls back to software cleanly.

## Approach (staged; each stage compiles + is independently useful)

### Stage 0 — independent quick wins (not blocked on decode)
- **Cache the static background** (`composite_document_to_hdr`): render once, reuse via a persistent
  HDR texture (copy-in each frame) instead of a full Vello render + 2 passes/submits every frame.
  Recovers ~3.6 ms (1080p) / ~7.5 ms (4K) per frame on *every* export. (In flight.)

### Stage 1 — software: decode at the requested target res (testable; fixes the quality bug now)
- Change `VideoManager::get_frame(clip, ts)` → `get_frame(clip, ts, target_w, target_h)`; thread the
  target from the renderer (preview = current doc/preview res, export = export res). Cap at native.
- Rework `VideoDecoder` so output size is per-request, not frozen at construction; cache the swscale
  context per output size (already cached per stream — extend the key). Adjust the frame cache key.
- Result: software exports are full-quality at any export res, and document resizes re-target decode.
  No hardware needed; this is the correctness fix for the codecs HW can't handle anyway.

### Stage 2 — hardware decode primitive (DONE, commit 255e164)
`decoder::VaapiDecoder` in `gpu-video-encoder`: decode → VAAPI surface → DRM-PRIME DMA-BUF →
`dmabuf::import_raw` → wgpu textures. Round-trip test (encode gray → decode → readback Y≈128) passes.

### The device-affinity problem (drives the whole rest of the design)
wgpu textures can't cross devices, and a decoded frame is a wgpu texture imported from a DMA-BUF —
which **requires a device with the DMA-BUF-import extensions** (`VK_EXT_image_drm_format_modifier`
+ external-memory), built via wgpu-hal `device_from_raw` (the safe `DeviceDescriptor` can't add
them). So a hardware-decoded frame is only usable by a compositor running on **such** a device.
- **Export** composites on the encoder's custom device → already fine.
- **Preview** composites on eframe's *normal* device → can't import DMA-BUFs → can't use HW frames.

Since **preview must HW-decode 4K** (software 4K decode ≈19 ms/frame), the resolution is a **single
shared custom device** used by eframe + preview compositor + decoder + encoder. eframe 0.33 (local
`egui-fork`) accepts it via `WgpuSetup::Existing { instance, adapter, device, queue }` — confirmed.
The earlier "separate export device" becomes redundant once this lands.

### Stage 3a — windowed shared `DrmDevice`, injected into eframe (highest-risk; blind)
Today `vk_device::create()` is **headless**. Make a windowed variant (or extend it) that is a
**superset** device: DMA-BUF import ext **+** `VK_KHR_swapchain` (device) and the WSI surface
instance extensions, **+** everything eframe/egui/vello need — `adapter.limits()` (already; Vello
needs `max_storage_buffers_per_shader_stage` ≥ 5), `max_texture_dimension_2d` 8192, and the optional
features main.rs requests (`SHADER_F16`, `TIMESTAMP_QUERY[_INSIDE_ENCODERS]`). Pick the adapter that
is the **VAAPI GPU** (the render node must match libva's, or DMA-BUF sharing fails on multi-GPU).
- main.rs: try to build the shared device; on success pass `WgpuSetup::Existing`, else fall back to
  the current `WgpuSetupCreateNew` (software decode only). Gate on Linux + VAAPI + a config/env
  override; **must be bulletproof** — this device now renders *every* frame of *every* session for
  Linux/VAAPI users, video or not. Milestone: editor runs normally on it with no video involved.

### Stage 3b — VideoManager hardware decode on the shared device (blind)
- `VideoManager` holds a `VaapiDecoder` per HW-decodable clip (built on the shared device), plus the
  software `VideoDecoder` fallback. `get_frame` gains a GPU-returning variant: yields an imported NV12
  texture pair (native res) instead of `Arc<Vec<u8>>`. Probe HW support per source; non-VAAPI /
  unsupported codecs / non-Linux → software path (Stage 1, target-res).
- Cache native GPU textures keyed by (clip, ts); revisit the byte budget (4K NV12 ≈ 12 MB each).

### Stage 3c — compositor consumes the GPU frame (blind; user-verifies)
- The video-instance composite path takes an NV12 texture (or a small NV12→RGB GPU pass) and blits it
  to the target with the existing bilinear blit — **no `write_texture` upload**. GPU scales native→
  target (preview res or export res). Both preview and the zero-copy export become
  decode→composite(→encode) with no CPU frame. Software frames still upload as today.

## Critical files
- `lightningbeam-core/src/video.rs` — `VideoDecoder` (per-request output size, scaler cache),
  `VideoManager::get_frame` (target param, cache key).
- `lightningbeam-core/src/renderer.rs` — pass the render target res into the video-instance build.
- `lightningbeam-editor/src/export/video_exporter.rs` — background cache (Stage 0); consume a GPU
  texture instead of uploading RGBA (Stage 3).
- `gpu-video-encoder/` (→ `gpu-video-codec`) — `dmabuf.rs`/`vk_device.rs` reused for the decode import.

## Risks
- **Shared custom device is the editor's main device (BIGGEST risk)** — Stage 3a makes a hand-built
  wgpu-hal Vulkan device render every frame for Linux/VAAPI users. It must satisfy eframe + egui +
  vello + winit presentation across varied Intel/AMD/Mesa stacks, or the editor won't start. Mitigate
  with a strict try-and-fall-back-to-normal-device path + an env/config kill switch. Test broadly.
- **Multi-GPU** — the shared render device must be the *same* GPU as libva's VAAPI device, or DMA-BUF
  import fails. Adapter selection must match the render node to the VAAPI node (laptops with iGPU +
  dGPU, PRIME).
- **Codec coverage** — only some codecs are HW-decodable per GPU/driver; software must stay correct
  and well-tested. Probe support per source, don't assume.
- **Cache memory** — native-res GPU textures (esp. 4K NV12 ≈12 MB) are large; revisit the frame cache
  budget, and the two live targets (preview res + export res) shouldn't thrash.
- **Colorspace/format** — VAAPI decode surfaces are NV12/tiled; import handles NV12, but 10-bit/HDR
  (P010) needs format handling. Decoded NV12 also needs the right BT.601/709 + range on the NV12→RGB
  read (mirror the encoder's color tags, [[gpu-video-decode]] color-range work).
- **Non-Linux / no-VAAPI** — must cleanly run on the normal eframe device with software decode.

## Verification
- Stage 0/1: visual — export above document res is now full-quality (not upscaled); profile shows
  background ≈ 0 and (Stage 1) software export correct at the chosen res.
- Stage 2: headless hardware test in `gpu-video-codec` (decode → wgpu texture, ffprobe/byte checks).
- Stage 3 (user): 1080p + 4K H.264 export — decode/upload buckets collapse; software fallback for a
  non-HW codec (e.g. ProRes) still produces correct full-res output.

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

### Stage 2 — hardware decode primitive (headless-testable here, like the 8 encode tests)
- In `gpu-video-encoder` (rename → `gpu-video-codec`): `h264_vaapi`-style **decode** → VAAPI surface →
  export DMA-BUF → import as a wgpu texture. Hardware test: decode a known file, verify dims/contents.

### Stage 3 — wire hardware decode into `get_frame` (blind; user-verifies)
- When the source codec/driver is HW-decodable, `get_frame` returns a **GPU texture** (native res)
  instead of `Arc<Vec<u8>>`; the compositor uses it directly (no `write_texture`), GPU-scaling to the
  target. For the zero-copy export the frame never leaves the GPU: **decode → composite → encode** on
  one device. Software path is the fallback for everything else.

## Critical files
- `lightningbeam-core/src/video.rs` — `VideoDecoder` (per-request output size, scaler cache),
  `VideoManager::get_frame` (target param, cache key).
- `lightningbeam-core/src/renderer.rs` — pass the render target res into the video-instance build.
- `lightningbeam-editor/src/export/video_exporter.rs` — background cache (Stage 0); consume a GPU
  texture instead of uploading RGBA (Stage 3).
- `gpu-video-encoder/` (→ `gpu-video-codec`) — `dmabuf.rs`/`vk_device.rs` reused for the decode import.

## Risks
- **Codec coverage** — only some codecs are HW-decodable per GPU/driver; software must stay correct
  and well-tested. Selection must probe support per source, not assume.
- **Cache memory** — native-res GPU textures (esp. 4K) are large; the frame cache budget needs revisiting.
- **Colorspace/format** — VAAPI decode surfaces are NV12/tiled; the existing import handles NV12, but
  10-bit/HDR sources (P010) need format handling.
- **Preview vs export sharing** — two live targets (preview res + export res) from the same source; the
  cache/scaler design must serve both without thrashing.

## Verification
- Stage 0/1: visual — export above document res is now full-quality (not upscaled); profile shows
  background ≈ 0 and (Stage 1) software export correct at the chosen res.
- Stage 2: headless hardware test in `gpu-video-codec` (decode → wgpu texture, ffprobe/byte checks).
- Stage 3 (user): 1080p + 4K H.264 export — decode/upload buckets collapse; software fallback for a
  non-HW codec (e.g. ProRes) still produces correct full-res output.

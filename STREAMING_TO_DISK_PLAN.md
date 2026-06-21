# Streaming Media To/From Disk — Plan

**Goal:** Lightningbeam must handle audio and video files (and raster animation, and
image assets) of *arbitrary length/size*. Anywhere we touch media we should stream from
and to disk when the data is too large to fit comfortably in memory, rather than loading
the entire file regardless of size.

**Scope of this document:** audio, video, raster frames, image-asset paging, **and the
`.beam` container format** — these turned out to be one problem, not two. Streaming on load
is impossible while the container forces a full decode, so the container decision (below)
is now part of this plan.

## Deferred bugs (do at the end)
- [x] **Timeline thumbnail scroll (FIXED):** the strip tiled from the *clamped* visible-left of the
      clip, so when a clip was scrolled partly off the left it showed the clip's start content at the
      viewport edge. Now tiled from the clip's **true (unclamped) origin** over its full width, drawing
      only the tiles intersecting the visible rect (`draw_video_thumbnail_strip` in timeline.rs). Both
      render sites (collapsed-group + expanded-track) share the helper. *(Compiles; needs in-app check.)*
- [x] **Clip thumbnails stop updating (FIXED):** the GPU texture cache was keyed by the *requested*
      content time, so once a tile cached the first (often far-off) thumbnail it never refreshed as
      closer ones loaded. `VideoManager::get_thumbnail_at` now also returns the **actual** thumbnail
      timestamp, and the cache keys on that — so a tile picks up a new texture when a closer thumbnail
      finishes generating. Existing `retain`-by-visible-clip cleanup keeps it bounded. *(Needs in-app check.)*

## Raster-keyframe-UI bugs — **[DONE]** (built the raster keyframe timeline UI, 2026-06-20)
Both resolved by the raster-keyframe-timeline-UI work: timeline now draws a diamond per
`RasterKeyframe` (mirrors vector), `K`/New Keyframe inserts a blank cel via `AddRasterKeyframeAction`
(canvas refreshes), paint tools edit the active keyframe instead of lazily creating, diamonds are
click-to-seek (pointing-hand cursor), playback prefetches frames, and onion skinning (raster+vector,
tinted, Info-Panel settings) is in. (a) canvas-refresh-on-new-keyframe and (b) keyframes-on-timeline
are both fixed.

## Noted enhancements (later, after the phases)
- [x] **Surround → stereo downmix (DONE).** Done uniformly in `render_from_file` (`pool.rs`) so it
  covers every storage type (PCM/InMemory, compressed via symphonia, video-audio via ffmpeg — all
  flow through this mixer with the source kept multichannel in the read-ahead buffer). New
  `stereo_downmix_matrix(src_channels)` gives `[L][src]`/`[R][src]` coefficients for the conventional
  interleave order (FL FR FC LFE BL BR SL SR…) for 3/4/5/5.1/6.1/7.1: full level for the matching
  front, `1/√2` for centre + each surround, LFE dropped; each row normalized so |coef| sum ≤ 1 to
  prevent clipping (matches ffmpeg's default). Applied in both the direct-copy and sinc-resample
  paths (only when `dst==2 && src>2`; unknown layouts fall back to front L/R). Compiles clean.
  *(Needs in-app check: a 5.1 file now has centre/dialog present and isn't thin; not distorted/clipping.)*
  Native multichannel support remains a separate, larger project.
- **Export speed:** a 1:14 1080p MP4 took ~9:06 to export (~7.4x slower than realtime). The video
  export pipeline re-seeks + decodes per output frame (see `[Video Seek]`/`[Video Timing]` logs) and
  does CPU YUV conversion; likely wins from sequential decode (avoid per-frame seeks), reusing the
  decode cache, and/or GPU-side color conversion. Profile before optimizing.
- **AAC export NaN guard (done):** `convert_chunk_to_planar_f32` now sanitizes non-finite samples
  (NaN/Inf → 0, finite clamped to [-1,1]) like the integer paths, with a one-time warning — a stray
  non-finite render sample no longer fails the whole export. Upstream NaN source (effect/automation/
  decode) still worth chasing if it recurs.
- [x] **Persist video thumbnails (DONE).** Mirrors waveform persistence: each clip's thumbnails are
  PNG-encoded + packed into one opaque `LBTN` blob (editor owns the format; `encode/decode_thumbnail_blob`
  in main.rs), stored as a `MediaKind::Thumbnail` row keyed by `thumbnail_media_id(clip_id)` (clip id XOR
  a fixed sentinel). Save: a cheap Arc-clone snapshot (`VideoManager::snapshot_all_thumbnails`) rides the
  `FileCommand::Save`, PNG-encoded off the UI thread in the worker, written by `save_beam` (kept in place
  on re-save). Load: `load_beam_sqlite` reads the packs into `LoadedProject.thumbnail_blobs`; the editor
  decodes + `insert_thumbnail`s them on a background thread and **gates regeneration** (`register_loaded_videos`
  skips clips with persisted thumbnails). Bonus: thumbnails show even if the source video file is missing.
  **Partial sets are persisted and resumed** (not thrown away): the `LBTN` blob (v2) carries a `complete`
  flag (`VideoManager.thumbnails_complete`, marked when the keyframe pass finishes). On load, complete
  packs are restored + skip regeneration; *partial* packs are restored AND generation is resumed —
  `generate_keyframe_thumbnails` takes a `should_skip` predicate (`has_thumbnail_near`) so it only decodes
  the keyframes not already covered. `insert_thumbnail` is now sorted + idempotent (fixes a latent
  unsorted-`binary_search` bug and makes concurrent restore + resume race-safe). So a save 50 min into a
  2 h video keeps that work and continues from there on reload.
  Container tests still green; all crates compile. *(Needs in-app check: reload = instant thumbnails for
  complete clips; a mid-generation save resumes from where it left off on reload.)*
  **Size assessment (done):** thumbnails are 128px wide, height by aspect (72px at 16:9 →
  128×72×4 ≈ **36 KB raw** each; 4:3 ≈ 49 KB), generated **one per ~5 s** (capped `interval_secs`,
  at keyframes — so ~12/min). Raw: ~0.5 MB per 1:14 clip, ~26 MB/hour, ~52 MB/2 h. Compressed for
  on-disk: JPEG ~3–6 KB/thumb → **~6 MB/2 h**; PNG ~8–15 KB → ~14 MB/2 h. So persistence is cheap
  (≤ the waveform's ~36 MB/2 h), especially as JPEG. Plan: encode each clip's thumbnails (JPEG) +
  their timestamps into one blob, a new `MediaKind::Thumbnail` row keyed by the clip/media id (mirror
  the waveform persistence: write on save, restore via `insert_thumbnail` on load, regenerate if
  absent). The 5 s interval already bounds count; no extra budget needed.
- **Progressive waveform on first import:** generation streams the whole file before the
  waveform appears (several seconds for large files). Since `build_waveform_pyramid` already
  streams, emit partial floors as it advances (e.g. flush every N seconds of decoded audio via
  the existing `waveform_result` channel + chunked GPU upload) so the overview fills in across
  the clip left-to-right instead of appearing all at once. Persistence saves only the final
  complete pyramid.

## Guiding principle
Three subsystems already have the right streaming primitive; most of the work is wiring,
bounding caches, and adding a residency window. The recurring pattern:

> Keep tiny metadata always-resident, fault the heavy payload in on demand keyed by a
> stable ID, and evict everything outside a window around the playhead.

---

## Audit summary (where we stand today)

### Correctly streaming / bounded
- Video frame decode/seek/playback (`lightningbeam-core/src/video.rs:191` `get_frame` —
  keyframe-index seek + decode-until-target, one frame resident).
- WAV/AIFF import via mmap (`daw-backend/src/audio/engine.rs:2328`).
- Webcam capture encodes directly to disk (`lightningbeam-core/src/webcam.rs`).
- `WaveformCache` (100MB cap), decoder `LruCache` (20 frames), export render loop (≤3
  frames in flight).
- The compressed-audio disk reader `daw-backend/src/audio/disk_reader.rs`
  (`CompressedReader` + 3s `ReadAheadBuffer`) — **correct but never activated** (Phase 1a).

### Fully-loaded, unbounded by file length (the problems)
| Site | Issue |
|---|---|
| `daw-backend/src/io/audio_file.rs:344` `decode_progressive` | Decodes whole compressed file into a `Vec<f32>`; de-facto playback source. |
| `daw-backend/src/audio/pool.rs:1071` `load_file_into_pool` | Every audio file in a saved project fully decoded to `InMemory` on open. |
| `lightningbeam-core/src/video.rs:711` `extract_audio_from_video` | Whole video audio track into one `Vec<f32>`. |
| `lightningbeam-core/src/video.rs:412` `VideoManager.frame_cache` | Unbounded `HashMap` of full-res RGBA frames; grows while scrubbing. |
| `export/mod.rs:388-400` | Mux step buffers all compressed packets into `Vec`s; O(duration). |
| `lightningbeam-core/src/raster_layer.rs:115` `RasterKeyframe.raw_pixels` | ~8MB/frame at 1080p; all keyframes decoded from PNG at load (`file_io.rs:611-640`), never evicted. |
| `lightningbeam-editor/src/gpu_brush.rs:1051` `raster_layer_cache` | Unbounded GPU texture `HashMap`. |
| `lightningbeam-core/src/renderer.rs:25` `ImageCache` | Unbounded decoded image cache (asset textures). |
| `Document.image_assets` (`document.rs:206`) | Every image asset's compressed bytes resident for document life. |

---

## Container format decision: `.beam` → SQLite  *(DECIDED)*

The `.beam` container moves from a **ZIP archive** to a **SQLite database file** (same
`.beam` extension). This is the foundation the rest of the plan builds on.

### Why
ZIP can stream `Stored` entries in place (via `data_start()`), but it has **no in-place
mutation** — every save and every raster frame write-back rewrites the whole archive — and
embedded PCM is rarely mmap-aligned. The current load path is even worse: it reads each
ZIP audio entry fully, decodes FLAC → re-encodes WAV → base64 → base64-decodes → temp file
→ full Symphonia decode → resident `Vec<f32>` (`file_io.rs:513-604`, `pool.rs:1071`).

SQLite dissolves the single-file-vs-performance tension:
- **Single file** — beginner-friendly, behaves like a file on every OS (no package-folder
  confusion; we have no bundle magic on Linux/Windows).
- **Streaming reads** — `sqlite3_blob_open` / `blob_read(offset, len)` gives seekable,
  chunked reads through the pager (mmap mode for the DB). For chunked streaming the
  pager-copy is negligible vs. decode cost, so the lack of zero-copy mmap doesn't matter.
- **Cheap, crash-safe mutation** — raster frame write-back is a transactional `UPDATE`;
  save is a metadata write + dirty-blob updates. **ACID** means a force-quit / power loss /
  crash mid-save can't corrupt the project (ZIP and package-dirs both have to hand-roll
  atomicity).
- **Inspectable / scriptable** — `sqlite3` CLI; `beam_inspector.py` can read it directly.

**Net effect: there is no scratch directory anywhere in this plan.** Media stream via blob
reads (or external paths); raster frames live in blob rows and write back transactionally.

### Large-media policy: packed OR referenced
Two storage modes per media item, both supported:
- **Packed** — bytes live in the DB. To stay under SQLite's ~2GB per-blob ceiling (and to
  make reads naturally chunked), large media is split into **multiple blob-chunk rows**
  (e.g. 64 MB/chunk); streaming reads address `(chunk_index, offset)`.
- **Referenced** — the DB stores only a path; bytes stay on disk (useful for shared media
  on a network drive, or media too large/volatile to pack).

**Default-mode preference for files over the per-blob limit (~2GB):**
- A user preference `large_media_default: Pack | Reference` controls what happens to
  imports above the threshold.
- The **first time** the user imports a media file over the limit, **prompt** them
  (Pack vs Reference), apply it, and **persist the choice** as the preference for future
  large imports (changeable later in settings).
- Files under the limit are packed by default (chunked only if needed).

### Schema sketch
```
media(
  id BLOB PRIMARY KEY,        -- stable Uuid
  kind INTEGER,               -- audio | video | raster | image-asset
  codec TEXT,                 -- "flac","mp3","png",... (original, lossless-preserving)
  storage INTEGER,            -- 0 = packed, 1 = referenced
  ext_path TEXT,              -- set when storage = referenced
  total_len INTEGER,          -- bytes (packed) for chunk math
  channels INTEGER, sample_rate INTEGER, width INTEGER, height INTEGER  -- kind-specific meta
)
media_chunk(
  media_id BLOB, chunk_index INTEGER, bytes BLOB,
  PRIMARY KEY (media_id, chunk_index)
)
project_json(id INTEGER PRIMARY KEY CHECK (id = 0), data TEXT)   -- existing project.json, verbatim
meta(key TEXT PRIMARY KEY, value TEXT)                           -- version, created, modified
```
`project.json` stays the same serialized `BeamProject` for now — only its container and the
media storage change. A migration reads a legacy ZIP `.beam` and writes the SQLite form on
first open/save.

### Streaming reads from packed media
A `BlobReader` implementing `Read + Seek` over `media_chunk` rows feeds the existing
streaming consumers unchanged: `CompressedReader` (audio) decodes from it instead of a
`File`; the video decoder seeks within it; raster `UPDATE`s a chunk. Referenced media uses a
plain `File` exactly as `do_import_audio` already does for originals today.

---

## Phase 1 — Audio: activate what already exists  *(highest impact, lowest effort)*

### 1a. Turn on the compressed-audio disk reader
The `CompressedReader` + 3-second `ReadAheadBuffer` in `disk_reader.rs` is complete but
never invoked (`DiskReaderCommand::ActivateFile` / `DiskReader::create_buffer` are never
called; `AudioClip::read_ahead` at `clip.rs:63` is hard-wired to `None`).
- On compressed import (`engine.rs:2381`) and during playback setup, activate the file and
  assign `AudioClip::read_ahead`.
- Change `decode_progressive` (`io/audio_file.rs:344`) to produce only the downsampled
  waveform overview (min/max peaks) the UI needs, then drop decoded PCM. Playback comes
  from the ring buffer, not RAM.
- Verify `render_from_file` (`pool.rs:449`) reads from `read_ahead` when `data()` is empty.

**Risk:** the real-time thread must never block on disk. The ring buffer prefetches ~2s
ahead; underruns degrade to silence (live) or block-wait (export), which `disk_reader.rs`
already distinguishes.

### 1b. Stream on project load  *(depends on the SQLite container)*
Three coupled changes (none works alone):
1. Replace `load_file_into_pool`'s full decode (`pool.rs:1071`) with the same branching as
   `do_import_audio`: PCM → mmap (referenced) or in-memory for tiny packed PCM; compressed
   (incl. FLAC) → `from_compressed` placeholder backed by a `BlobReader` (packed) or `File`
   (referenced). The claxon FLAC→WAV→base64 round-trip in `file_io.rs:533-591` is deleted.
2. **Bulk read-ahead activation:** loaded clips are deserialized directly
   (`audio_backend.project`), bypassing `AddAudioClip`, so the Phase 1a wiring never fires
   for them. After the engine installs the project, walk all audio clips and
   `create_buffer` + `ActivateFile` + set `read_ahead` for every clip referencing a
   `Compressed` pool entry. (`CompressedReader::open` needs a variant that takes a
   `BlobReader` instead of a path for packed media.)
3. Pool entries carry storage mode (packed-chunks vs referenced path) from the `media`
   table instead of base64 `embedded_data`.

### 1c. Video's embedded audio track — stream from the video via ffmpeg

**Interim stopgap (shipped):** `extract_audio_from_video_to_wav` streams the decoded audio to
a temp WAV, imported via `import_audio_sync` (mmap). Fixes the RAM OOM but writes the whole
uncompressed track to `/tmp` (fills small temp partitions) and the temp path doesn't survive
save/reload. **Superseded by the design below.**

**Proper design — stream the video's audio track on demand, never materialized.**

*Enabler:* `daw-backend` already depends on `ffmpeg-next` (used for MP3/AAC encoding), so the
ffmpeg audio decoder lives beside `CompressedReader` in `daw-backend/src/audio/`. No
cross-crate work (`core → daw-backend` is one-way). `CompressedReader` already has the needed
interface.

1. **`VideoAudioReader` (ffmpeg)** — mirrors `CompressedReader`:
   `open(path)`, `decode_next(&mut Vec<f32>) -> frames` (resample → interleaved f32 at native
   rate; reuse the old extraction resampler), `seek(target_frame) -> actual`,
   `sample_rate`/`channels`/`total_frames`.
2. **Source dispatch:** `enum StreamSource { Compressed(CompressedReader), Video(VideoAudioReader) }`
   (or a small `trait AudioFrameSource`) held by the reader thread; ring buffer / prefetch /
   export-blocking unchanged. `DiskReaderCommand::ActivateFile` gains a `kind: SourceKind`.
3. **Pool model:** `AudioStorage::VideoAudio { video_path, decoded_for_waveform, decoded_frames,
   total_frames }` (near-copy of `Compressed`); `data()` empty, playback via `read_ahead`. Pool
   entry `path` = the video file.
4. **Engine API:** `EngineController::add_video_audio_sync(video_path) -> usize` — ffmpeg-probe
   the audio track (rate/channels/frames/duration, no decode), build the pool entry, return index.
5. **Clip activation:** extend the Phase 1a `AddAudioClip` wiring — if entry is `VideoAudio`,
   make the buffer + `ActivateFile{kind:VideoAudio, path:video_path}` + set `clip.read_ahead`.
   One ffmpeg context + 3 s buffer per active clip instance.
6. **Import flow:** `import_video` calls `add_video_audio_sync(video_path)` →
   `AudioClip::new_sampled`. **Remove** `extract_audio_from_video_to_wav`, the temp-WAV
   handling, and the now-dead `add_audio_file_sync`. No WAV / `/tmp` / RAM.
7. **Save/load:** the `VideoAudio` entry serializes as a path reference to the video (no media
   bytes — the video is already referenced by its `VideoClip`); reconstruct on load by
   re-probing. Fixes the stopgap's reload fragility (nothing to persist).
8. **Waveform overview:** background ffmpeg pass emitting **downsampled peaks only** (bounded
   memory) into the existing waveform path — shared with the Phase 1a `decode_progressive`
   cleanup.

**Sample accuracy (required — video audio must stay frame-synced with other clips):**
Coarse ffmpeg seeks are NOT sufficient. `VideoAudioReader::seek(target_frame)` must:
- coarse-seek to a point ≤ target, then **decode-and-discard** to land exactly on
  `target_frame`, tracking the absolute sample position from decoded-frame PTS (discard whole
  frames before target; for the frame straddling target, drop its leading samples). After
  `seek`, `decode_next` yields samples starting at exactly `target_frame`.
- This makes frame N of the video-audio pool entry correspond to the exact timeline position,
  so it mixes sample-aligned with mmap/InMemory clips. Continuous decode advances frame-exact.
- *Consistency note:* `CompressedReader` should get the same decode-discard alignment (its
  current coarse-seek-then-write-at-target can misalign by up to a GOP after a seek). Fold in
  while here, or at least flag.

*Model decision (confirmed):* the video's audio stays a **separate, editable `AudioClip`** on
an audio track, backed by the `VideoAudio` pool entry — users can move/trim/mute/detach it.

*Build order:* `VideoAudioReader` + `StreamSource` → pool `VideoAudio` variant →
`add_video_audio_sync` + activation → swap `import_video` (remove WAV path) → sample-accurate
seek (both readers) → waveform-peaks pass.

---

## Phase 2 — Video: bound the caches  *(small, isolated)*

### 2a. Bound `VideoManager.frame_cache`
`video.rs:412` — convert the unbounded `HashMap<(Uuid,i64), Arc<VideoFrame>>` to an LRU
mirroring the decoder-level cache (`video.rs:34`). Frame-count or byte budget.

### 2b. Stream the export mux
`export/mod.rs:388-400` — interleave-write packets to the output as produced (compare PTS,
write the earlier stream) instead of collecting all then writing. O(duration) → O(1).

---

## Phase 3 — Raster: disk-backed keyframe paging  *(the heavy one)*  **[locked design]**

Today `load_beam_sqlite` (`file_io.rs:564`) eagerly `decode_png`s **every** raster keyframe's
`Raster` media row into `RasterKeyframe.raw_pixels` (`raster_layer.rs:115`, `w·h·4` ≈ 8 MB @
1080p, `#[serde(skip)]`), never evicts, has an unbounded GPU texture cache, and holds full-frame
undo snapshots. `raw_pixels` is the working rep (edits write it, save reads it, render reads it),
`has_pixels()` = `!raw_pixels.is_empty()`, `keyframe_at` is a `partition_point` binary search, and
the container is opened only at load/save (no live handle).

**Design (confirmed with user):** keep `raw_pixels` as the working rep; make residency explicit
via a `RasterStore` + an editor-run fault-in/evict pass *before* the immutable render. Async
fault-in (no scrub hitch), with a **low-res image proxy** shown until the full frame lands.
Decisions: small window (±~2 keyframes); **dirty (edited-unsaved) frames stay fully resident**
(spill-to-scratch deferred); fault-in is **async**; proxy is a **per-keyframe low-res RGBA image**
(PNG/WebP, correct alpha), NOT a video (VP9-alpha was rejected as finicky for negligible disk win).

### Drive-by (Arc pixels): DROPPED
Investigated and rejected: `raw_pixels` has ~64 access sites, and most `.clone()`s genuinely need
an owned `Vec<u8>` (undo buffers, export, GPU readback) so `Arc<Vec<u8>>` would force `(*p).clone()`
and still copy. The only beneficiary, the per-frame `renderer.rs:550` Vello clone, is on the
**legacy/dead** path — the live HDR canvas renders raster as `RenderedLayerType::Raster` → GPU
upload in `stage.rs` which passes a `&[u8]` slice and uploads only on cache-miss (no per-frame
clone). Not worth 64 edits. Start at 3a.

### 3a. Lazy async fault-in + image proxy
- **[DONE 3a-1]** Lazy load: full-decode removed; `raw_pixels` empty on load, `needs_fault_in`
  armed recursively; canvas records misses → App pages in via `RasterStore.load_pixels`.
- **[DONE 3a-2]** Async: page-in runs on a background thread (deduped via `raster_loads_inflight`);
  results applied at top of `update()`. No UI block on cold scrub.
- **[DONE 3a-3]** Image proxy: `MediaKind::RasterProxy` (≤192px PNG, derived id), written
  beside each resident full PNG on save + eager-decoded on load into `RasterKeyframe::proxy`.
  Separate `proxy_layer_cache` (own LRU, budget 64); the raster render blits the proxy mapped to
  the keyframe's FULL logical dims (upscales via sampler) when the full texture isn't resident.
  *(Proxies exist only after a save+reload; eager decode → lazy/paged is a refinement for huge
  paint projects.)*

- **`RasterStore`** (core): current `.beam` path + a read-only connection; `load_pixels(kf_id,w,h)`
  reads the `Raster` row and `decode_png`s it. Set/cleared by the editor on load + save-as.
- **Save:** alongside the full PNG, write a low-res RGBA proxy per resident keyframe
  (`MediaKind::RasterProxy`, ≤~480px long edge, keyed by `kf.id`).
- **Load:** stop eager full-decode; decode **proxies** eagerly (cheap → instant scrub everywhere);
  leave full `raw_pixels` empty.
- **Fault-in pass** (editor, `&mut document` + store, each frame before render): for each raster
  layer ensure the active keyframe ±N is requested; load full PNGs on a **background thread pool**;
  on arrival, set `raw_pixels` + `texture_dirty`. Render uses full `raw_pixels` if resident, else the
  upscaled proxy. Reused by the exporter (already frame-by-frame).

### 3b. Residency window + eviction  **[DONE]**
- Added `#[serde(skip)] dirty: bool` (edited-since-persist; distinct from `texture_dirty`). Set on
  stroke/fill/paint-bucket/floating-lift commits + undo/redo; cleared on save (which re-arms the LRU).
- Implemented as a fault-in-recency **LRU** (`RASTER_RESIDENT_MAX = 12`), not a strict ±N window:
  evict the oldest **clean** frame (drop `raw_pixels`, re-arm `needs_fault_in`); the shown frame is
  always most-recent so it's protected; **dirty frames never evicted**. Save preserves evicted frames'
  rows via `media_exists` (no data loss) and walks all layers to match load.
  *(Refinement deferred: count budget → byte budget for 4K resolution-robustness.)*

### 3c. Bound the GPU cache  **[DONE for raster_layer_cache]**
`raster_layer_cache` (`gpu_brush.rs`, `HashMap<Uuid,CanvasPair>`, Rgba16Float ping-pong
≈ `w·h·16`/entry, was **unbounded**) → recency LRU (`RASTER_LAYER_CACHE_MAX = 12`) in
`ensure_layer_texture`: bump-to-most-recent + evict oldest; shown frames protected. F3 overlay
now shows tracked VRAM (raster cache MB + count). *(Refinements: count→byte budget; raise/headroom
if >12 raster layers are visible at once. Export `raster_cache` lives one export — fine. Vello
`ImageCache` is image *assets* → Phase 4.)*

### 3d. Undo memory  **[DONE]**
`RasterStrokeAction`/`RasterFillAction` stored `buffer_before`+`buffer_after` full frames.
Now store a `RasterDiff` (`actions/raster_diff.rs`) — changed bbox before/after only, computed in
`new()`, full buffers dropped. Undo/redo apply onto the keyframe's resident pixels; the editor
faults the target frame in first (`Action::raster_resident_hint` + `peek_undo/redo_raster_hint`),
correct because a clean evicted frame's container bytes == its logical state. Non-resident base ⇒
skip (no corruption). Unit-tested round-trip. *(Refinement: compress full-canvas-fill diffs, whose
bbox is the whole frame.)*

### 3e. Prefetch frames  **[DONE for playback]**
Implemented for playback: each update during playback, page in the next `PREFETCH_AHEAD=4`
upcoming keyframes per raster layer (reusing the async worker + `raster_loads_inflight` dedup), so
full frames are resident before the playhead arrives — fixes "proxy on every frame"/flicker during
playback. *(Caveat: with many simultaneous raster layers the 12-frame resident budget may evict a
prefetched frame before it's shown — raise budget or scale prefetch if that surfaces. Scrub-direction
prefetch still TODO.)*

Original note: *(future, after 3d — pure latency win, no correctness need)*
Fault-in is reactive (page in only on a render miss), so a never-visited frame still shows the
proxy for a beat before the full lands. **Prefetch the full pixels for frames about to be shown**:
on scrub/playback, dispatch background page-ins for the active keyframe ±N in the direction of
playhead motion (and during playback, the next K keyframes), reusing the 3a-2 async worker +
`raster_loads_inflight` dedup. Keep prefetched frames in the 3b LRU so they're still bounded; cap
concurrent prefetch loads so scrubbing fast doesn't thrash the disk. Optional: also prewarm the GPU
texture (3c cache) for the immediate next frame. Net effect: cold scrubbing/playback shows full-res
frames with no proxy flicker. Proxy stays as the instant fallback when prefetch can't keep up.

### Build order & tests
1. Arc drive-by — COW make_mut test. 2. 3a fault-in + store + proxy — load→empty-until-faulted,
PNG round-trip, proxy-then-swap. 3. 3b window/evict/dirty — residency ≤ window while scrubbing,
dirty never evicted. 4. 3c GPU bound. 5. 3d undo diffs reproduce pre-stroke buffer exactly.

---

## Phase 3.5 — Image textures in vector scenes  **[DONE 2026-06-21]**  *(prereq for Phase 4; fixed DCEL-broken image import)*

**Done:** 3.5a — import/drop places an image as a borderless image-filled rectangle
(`AddShapeAction::image_rect`), centered (direct import) or at the drop point (library drag);
renderer now maps the image brush onto the fill's bounding box (was anchored at world origin →
only a corner showed); `SetImageFillAction` + an **Image** fill-type tab (None|Solid|Gradient|Image)
with an asset picker in the Info Panel. 3.5b — image bytes persist as `MediaKind::ImageAsset` rows in
the `.beam` (kept-in-place; `ImageAsset.data` is `skip_serializing` + container-backed; old base64
projects migrate on re-save); eager-read on load. *(ImageCache still unbounded — Phase 4 adds the
usage-based LRU/lazy paging.)*

### (original plan below)
## Phase 3.5 — Image textures in vector scenes  *(prereq for testing Phase 4; fixes DCEL-broken image import)*

**Why:** Phase 4 pages *image assets*, but there's currently no way to get an image asset into a
vector scene — so nothing to page. This also repairs image import, half-broken since the DCEL switch.

**Current state (audited 2026-06-21):**
- *Works:* `import_image` (`main.rs`) decodes dims + creates an `ImageAsset` (raw bytes embedded in
  `Document::image_assets`, serialized as **base64 in project JSON**). The renderer's image-fill paths
  are **complete** — GPU/Vello (`renderer.rs:~1160`, `ImageBrush` via `ImageCache.get_or_decode`) and
  CPU/tiny-skia (`renderer.rs:~1486`). `Fill::image_fill` (`vector_graph/mod.rs:110`) and
  `Face::image_fill` (`dcel2/mod.rs:117`) fields exist and render when set.
- *Broken/missing (the workflow):*
  1. **Drop image → canvas is stubbed:** `stage.rs:~11782` and `main.rs:~4924` both just print
     "Image drag to stage not yet supported with DCEL backend". Nothing is added to the scene.
  2. **No way to assign an image fill:** no `SetImageFillAction` (only `SetFillPaintAction` for
     color/gradient); no Info-Panel picker. `Fill`/`Face.image_fill` are never populated.
  3. **DCEL faces never get `image_fill`** (`dcel2/import.rs:275` always `None`; topology copies from
     parent which is also `None`).
  4. **Not in the container:** `MediaKind::ImageAsset` exists but is **dead** — image bytes live only
     as base64 in project JSON. Not chunked, not pageable (so Phase 4 can't page them).

**Tasks:**
- **3.5a — Place + assign.** Replace the two drop stubs: dropping an image onto a vector layer creates
  a rectangle face sized to the image at the drop point with `image_fill = asset_id`. Add
  `SetImageFillAction` (set/clear an image fill on the selected face/shape; mirrors `SetFillPaintAction`)
  + an Info-Panel image-asset picker for the selected shape's fill. Populate `Face.image_fill` in DCEL
  (and keep it through topology ops — already copied from parent).
- **3.5b — Persist in the container.** Write image assets as `MediaKind::ImageAsset` rows in the `.beam`
  SQLite (like raster/audio: write on save kept-in-place on re-save; read on load), keyed by asset id;
  drop the base64-in-JSON embedding (or keep a tiny ref). This is the storage Phase 4 pages from.
- **3.5c — Lazy decode hook.** Image bytes load from the container into `ImageCache` on first render
  (decode → `ImageBrush`/`Pixmap`). Leave `ImageCache` **unbounded for now**; Phase 4 adds the
  usage-based LRU/eviction (this phase just makes there *be* real, container-backed image assets to page).
- **Tests:** import→drop→render round-trip; save/reload preserves the image fill + reads bytes from the
  container (not JSON); CPU and GPU render paths both show the image.

---

## Phase 4 — Asset paging by usage + LRU  *(vector's real cost is assets, not geometry)*

Vector geometry is compact flat POD (tens of KB/frame, no cached tessellation/DCEL) — leave
it resident. The heavy, evictable thing is the **image assets** referenced by fills.

**Data model.**
- `ImageAsset` (`clip.rs:250`): `path: PathBuf` + `data: Option<Vec<u8>>` (whole compressed
  file bytes) + dims. Imported fully into `data` at `main.rs:3936`.
- All assets resident in `Document.image_assets: HashMap<Uuid, ImageAsset>` (`document.rs:206`).
- Decoded form in `ImageCache` (`renderer.rs:25`): `HashMap<Uuid, Arc<ImageBrush>>` + CPU
  `Pixmap` map, keyed by asset id, **unbounded**.
- A `Fill` references an asset by `image_fill: Option<Uuid>` (`vector_graph/mod.rs:110`).
  Same UUID may appear in many fills/keyframes/layers and recursively through clip instances.
  **No asset→frame or frame→asset index exists today.**

**Two evictable tiers:** Tier 1 = compressed bytes (`ImageAsset.data`, droppable, reload
from blob row or external `path`); Tier 2 = decoded pixels (`ImageCache` + GPU textures —
the heavy one).

### 4a. Frame→asset enumeration (incl. nested clips — see note below)
A function `assets_needed_at(time) -> HashSet<Uuid>`: walk each visible vector layer's active
`ShapeKeyframe`, collect `fill.image_fill` across its `VectorGraph.fills`, **recursing into
clip instances** with the outer→inner local-time mapping. This is "needed now". Scanning
upcoming keyframes (and upcoming nested-clip keyframes) gives "needed soon" for prefetch.

### 4b. Usage bookkeeping (the multi-frame problem)
Maintain a reverse index `asset_id → usage count` (fills referencing it across the whole
document), updated incrementally as edits add/remove `image_fill`s (hook the fill-mutation
paths in `vector_graph` and the relevant actions).
- count 0 → dead, fully evictable / GC candidate.
- count > 0 → keep metadata; residency of `data`/decoded pixels driven by **proximity to
  playhead**, not by count (a high-count asset far from the playhead is still evicted).

Residency decision: `resident = needed-now ∪ needed-soon`; beyond that, an **LRU with a byte
budget** for referenced-but-distant assets (covers scrubbing back without a reload).
Eviction never touches an asset in needed-now.

### 4c. Bound the decoded tier
Convert `ImageCache`'s two maps to LRU/byte-budgeted (`renderer.rs:25`) and bound the GPU
image-texture cache the same way, keyed to the residency window.

### Nested-clip prefetch (important)
A clip instance placed on an outer frame has its **own internal timeline of keyframes**,
each of which can reference its own image assets. Prefetch must therefore:
- Recurse through clip instances when computing both needed-now and needed-soon.
- Map outer playhead time → each nested clip's local time, and look ahead along the
  **nested** timeline (not just the outer one) so assets used by an upcoming *inner*
  keyframe are loaded before the nested clip reaches it.
- Deduplicate across the whole recursion (an asset shared by outer and inner frames counts
  once); the usage index handles refcounting.

---

## Cross-cutting: a shared residency abstraction

A generic **`PagedStore<Id, Payload>`** with three consumers — always-resident metadata,
disk backing, residency = window/needed-set around playhead + LRU byte budget:

| Consumer | Metadata kept | Paged payload | Backing | "Needed now" key |
|---|---|---|---|---|
| Raster keyframes (Ph 3) | id, dims, time | `raw_pixels` + GPU texture | SQLite blob row (`UPDATE` on write-back) | active keyframe per layer |
| Image assets (Ph 4) | id, dims, storage | `data` bytes + decoded pixels/texture | SQLite blob row or external path | fills' `image_fill` set at time (recursive) |
| Video frames (Ph 2a) | — | RGBA frame | source via ffmpeg seek | requested timestamps |

Audio stays separate (real-time ring buffer, different constraints). The frame→asset
enumeration + usage index is unique to Phase 4.

---

## Sequencing
1. **Phase 1a** — done; independent of the container, works with the current ZIP loader.
2. **Phase 2** — small, isolated, independently shippable; container-independent.
3. **Phase 0 (container)** — `.beam` ZIP → SQLite + `BlobReader` + large-media policy +
   legacy-ZIP migration. Prerequisite for 1b/1c/3/4.
4. **Phase 1b** — streaming pool loader + bulk read-ahead activation (on the SQLite store).
5. **Phase 1c** — depends on 1b's pool path.
6. **Phase 3** — the substantial build; implement `PagedStore` over blob rows.
7. **Phase 4** — thin layer on the same abstraction + the frame→asset/usage index.

Phase 1a and Phase 2 can ship now; everything else waits on Phase 0 (the container).

---

## Status
- [~] Phase 1a — activate compressed-audio disk reader  ← **in progress**
  - [x] Wire `ActivateFile` + assign `clip.read_ahead` on `AddAudioClip` for compressed
        pool files (`engine.rs:909`). Per-clip reader keyed by `clip_id`; matches the
        existing `DeactivateFile` convention in `RemoveAudioClip`. Compiles clean.
  - [ ] Stop `decode_progressive` (`io/audio_file.rs:344`) from accumulating/streaming the
        full PCM; emit only the downsampled waveform overview. (Crosses into the UI
        waveform pipeline — `AudioDecodeProgress` consumer — so handled as its own step.)
  - [ ] Runtime verification: confirm a compressed clip actually plays from the ring
        buffer (was effectively silent before, since `read_ahead` was always `None`).
- [~] **Phase 0 — container migration `.beam` ZIP → SQLite**  ← **in progress**
  - [x] SQLite schema (`media`, `media_chunk`, `project_json`, `meta`) + `rusqlite` dep
        (bundled) — `lightningbeam-core/src/beam_archive.rs`
  - [x] `BlobReader` (`Read + Seek` over `media_chunk`, owns its own read-only connection,
        opens a blob handle per read with rowids resolved once) — for `CompressedReader` /
        video decoder in 1b. 5 integration tests pass (`tests/beam_archive.rs`): json
        round-trip, packed full read, streaming reads + seeks across chunk boundaries,
        referenced-path, overwrite-replaces-chunks.
  - [x] Packed (chunked) + referenced media write/read API; `is_sqlite()` format detection;
        `MediaKind`/`MediaStorage`/`MediaMeta`/`MediaInfo`.
  - [x] `BeamArchive::transaction()` / `BeamTxn` — in-place transactional save (only
        changed rows written; unchanged large media never rewritten); orphan cleanup via
        `retain_media`. 7 archive tests pass (added txn-grouping + rollback). Per user: save
        must NOT copy+rename for existing SQLite files.
  - [x] Wire `save_beam` to `BeamArchive` — in-place txn for existing SQLite, temp+rename
        only for new/migrated files. Audio → packed (or referenced ≥2GB) `media` rows;
        raster → PNG `media` rows keyed by keyframe id. FLAC→WAV→base64 save round-trip
        deleted (now packs original bytes with their codec).
  - [x] Wire `load_beam` — format dispatch: SQLite (`load_beam_sqlite`) vs legacy ZIP
        (`load_beam_zip_legacy`, kept verbatim). SQLite load reconstitutes packed audio into
        `embedded_data` so the existing pool loader is unchanged (streaming = Phase 1b).
  - [x] Legacy ZIP `.beam` → SQLite migration: `is_sqlite()` routes load; saving a
        ZIP-loaded project writes SQLite (migrates on save). Editor compiles end-to-end.
  - [x] Large-media policy: packed (chunked) vs referenced — `LargeMediaMode {Ask,Pack,
        Reference}`; save honors it for files ≥`LARGE_MEDIA_THRESHOLD`. Packing streams from
        disk via `put_media_packed_from_path` (chunk-by-chunk, never loads the whole file).
        `Ask` behaves as `Reference` at save time.
  - [x] `large_media_default` user preference: persisted in `AppConfig`, editable in
        Preferences → Advanced (incl. resetting to `Ask` to re-trigger the prompt).
  - [x] First-import-over-threshold prompt: `note_possible_large_media` (hooked into
        import_audio/video/image) queues a one-time modal; choice persists to config.
        Threshold shown in the modal is derived from the constant.
  - [ ] Runtime verification: save a real project, reopen it, confirm audio + raster survive
        round-trip; confirm an old ZIP `.beam` still opens and migrates on save.
  - [ ] (Optimization, later) FLAC-compress packed PCM/WAV audio; raster disk-dirty flag to
        skip unchanged frames on in-place save (Phase 3).

> Note: the crate's internal `#[cfg(test)]` modules (`clip.rs`, `effect_layer.rs`) have
> pre-existing compile breakage (old `Beats`/`TempoMap` API) unrelated to this work; it
> blocks `cargo test --lib`, so `beam_archive` tests live in `tests/` (integration) which
> build the lib in normal mode. Worth fixing separately.
- [x] Phase 1b — stream on project load (PACKED audio path complete & user-verified: streams on load,
      waveform generates + persists, sample-accurate seeking). Referenced-path streaming + MP3 seek index
      + proper video-audio reload remain as noted follow-ups.
  - **Decision (user):** cross-crate packed streaming via an **inversion-of-control factory** —
    daw-backend defines the interface, core implements it over `BlobReader`. Keeps the audio
    engine container-agnostic. (Alternatives rejected: daw-backend owning rusqlite = layering
    violation; referenced-only-first = leaves packed <2GB in RAM.)
  - **Current load reality (why this is needed):** *nothing* streams on load today — every entry
    is fully decoded to a PCM `Vec<f32>`. Packed audio is base64-reconstituted into `embedded_data`
    (`load_beam_sqlite`) → written to a temp file → `load_file_into_pool` full-decodes; referenced
    audio also full-decodes via `load_file_into_pool`; and the Phase 1a/1c disk-reader activation
    never fires for loaded clips (they bypass `AddAudioClip`).
  - [x] **B1/B2 foundation (DONE, headless-tested):** in `disk_reader.rs` — `trait MediaByteSource:
        Read+Seek+Send+Sync { byte_len }` + `trait AudioBlobSourceFactory: Send+Sync { open(media_id)
        -> Box<dyn MediaByteSource> }`; `SymphoniaByteSource` adapter (impl `MediaSource`,
        is_seekable/byte_len); `CompressedReader::open_source(src, ext)` sharing probe via a
        refactored `from_mss`; `enum StreamOpen { Path, Source{src,ext} }`; `StreamSource::open` and
        `DiskReaderCommand::ActivateFile` now take `StreamOpen` (engine site wraps `Path`); re-exported
        `AudioBlobSourceFactory`/`MediaByteSource` at `daw_backend::audio`. Test
        `tests/compressed_source_stream.rs` decodes an in-memory WAV through a `Cursor`-backed
        `MediaByteSource` (proves probe+decode+seek over a byte stream). daw-backend compiles clean.
  - [x] **B3 (engine, DONE):** `Engine.blob_source_factory: Option<Arc<dyn AudioBlobSourceFactory>>` +
        `EngineController::set_blob_source_factory` (via `Query::SetBlobSourceFactory`, ordered before
        `SetProject` on the same queue). `AudioFile.packed_media_id: Option<String>` (Some ⇒ open via
        factory using `original_format` as the ext hint; None ⇒ `StreamOpen::Path`). Activation factored
        into `Engine::activate_streaming_for(reader_id, pool_index)`, used by `AddAudioClip` and bulk.
  - [x] **C (core factory, DONE):** `file_io::blob_source_factory(beam_path)` → `BeamBlobFactory`
        implementing `AudioBlobSourceFactory` over `BeamArchive::open_blob_reader`. `BlobReader` holds a
        `!Sync` rusqlite `Connection`, so it's wrapped in `SyncBlobReader` (a `Mutex` used via `get_mut`
        on the hot path — no runtime locking) to satisfy Symphonia's `MediaSource: Send + Sync`. Installed
        by the editor between `load_audio_pool` and `set_project`.
  - [x] **D (load-path, DONE — packed audio):** `load_beam_sqlite` now streams packed audio whose codec
        is recognized (`is_streamable_audio_codec`) — leaves `embedded_data` empty so the pool builds a
        Compressed placeholder with `packed_media_id`; no base64, no temp file, no decode. `serialize`
        round-trips packed entries by media id (so in-place re-save keeps the row). Non-audio codecs
        (video-container audio tracks) keep the legacy reconstitution path → **no regression**.
  - [x] **E (bulk activation, DONE):** `SetProject` calls `Engine::activate_all_streaming_clips` —
        walks every loaded audio clip and `activate_streaming_for` (create_buffer + `ActivateFile` + set
        `read_ahead`), the loaded-clip equivalent of the Phase 1a wiring.
  - [x] **Waveform-on-load for streamed audio (DONE):** streaming broke the old waveform path (it came
        from the full in-RAM decode, which no longer happens). Added
        `disk_reader::build_waveform_pyramid_from_source(Box<dyn MediaByteSource>, ext, B)` (load-time
        counterpart of the path-based builder). On load, the editor background-generates a pyramid for any
        streamed entry lacking a persisted one (opens the packed blob via a local factory), sending the
        floor through the same `waveform_result` channel `update()` drains; the next save persists it.
        Verified in-app: packed MP3 **streams + plays** (`Activated reader=0, kind=CompressedAudio`); the
        overview now fills in shortly after load.
  - **Headless tests pass** (compressed_source_stream, video_audio_stream, waveform_pyramid); all three
    crates compile clean. **Needs in-app verification:** the waveform appears after load (background gen),
    then instantly on subsequent loads once saved; RAM stays flat on a big project.
  - [x] **Seek alignment fix (DONE):** streamed compressed audio was ~1.2s off *after seeking*
        (fine from the start). `CompressedReader::seek` used `SeekMode::Coarse`, which for MP3
        byte-estimates the position and seeds the timestamp from that estimate — wrong for VBR / files
        whose header padding the estimate ignores, so `actual_ts` (and thus the buffer's frame labels)
        landed ~1.2s early. Switched to `SeekMode::Accurate`: Symphonia counts frame *headers* (no
        decode) from a true anchor (current pos, or rewind-to-0 for backward seeks) → exact `actual_ts`;
        the existing sub-frame `pending_discard` finishes the job. FLAC/OGG seek cheaply (seek tables);
        a long MP3 backward seek walks headers from 0 (I/O, not decode). Tests still green.
  - [ ] **Deferred (follow-up):** per-file **seek index** for elementary streams (MP3) — a one-time
        header scan (ts↔byte map) to make far seeks O(1) instead of an Accurate header-walk from the
        anchor. Matters for multi-hour MP3s; song-length files are fine as-is.
  - [x] **Proper video-audio reload (DONE):** a video's audio is now stored as a **path reference** to
        the video (never packed/embedded as audio media) and **re-probed via FFmpeg** on load into a
        streaming `VideoAudio` entry — `AudioPoolEntry.is_video_audio` flag drives both `serialize`
        (reference, not pack), `save_beam` (`reference_it |= is_video_audio`), and `load_from_serialized`
        (`VideoAudioReader::open` → `from_video_audio`). Fixes 5.1 audio losing its channels on reload
        (the old Symphonia reconstitution collapsed it); also no more decode-whole-video-to-RAM / temp
        files on load. Old saves (video mis-packed as audio) self-heal on the next save.
  - [ ] **Deferred (follow-up):** stream *referenced* (external-path) **audio** on load too — replace
        `load_file_into_pool`'s full decode with the `do_import_audio` branching (PCM → mmap, compressed
        → `from_compressed` placeholder). Higher risk (touches the working referenced path); packed
        covers the common <2GB case first.
  - [ ] **Deferred (follow-up): packed video streaming.** Let small videos be packed into the `.beam`
        (a `MediaKind::Video` blob, `VideoClip` referencing it by id) and stream **both frames and audio**
        from the DB blob via FFmpeg. ffmpeg-next has no custom-I/O wrapper, so this needs an
        `AVIOContext`-over-`BlobReader` shim via raw FFI. **Decision (user):** that FFI wrapper lives in
        its **own crate, version-pinned to the ffmpeg version**, isolating the unsafe + the ABI coupling.
- [~] Phase 1c — video embedded-audio track  ← **stopgap shipped; proper design next**
  - [x] Stopgap: `extract_audio_from_video_to_wav` streams to a temp WAV → `import_audio_sync`
        (mmap). Fixed the ~2.8GB-`Vec<f32>` OOM. But writes the whole WAV to `/tmp` (fills
        small temp partitions) and the temp path doesn't survive reload.
  - [~] **Proper design** (see "Phase 1c" body): stream the video's audio on demand via a new
        ffmpeg `VideoAudioReader` in the disk reader — no extraction, no `/tmp`, no RAM; path
        reference survives save/load.
    - [x] **Step 1 (DONE):** `VideoAudioReader` (ffmpeg) + `StreamSource` enum + `SourceKind`
          in `disk_reader.rs`. Sample-accurate seek (coarse seek + decode-discard to exact
          frame via PTS). 2 integration tests pass (`daw-backend/tests/video_audio_stream.rs`):
          in-order decode + sample-exact seek at several targets. (Found: mono frames have an
          empty channel layout → must `set_channel_layout` before resampling, else swr returns
          AVERROR_INPUT_CHANGED.) Lib compiles clean; `StreamSource` `#[allow(dead_code)]`
          until wired. `VideoAudioReader` made `pub` for the integration test.
    - [x] **Step 2 (DONE):** `AudioStorage::VideoAudio { decoded_for_waveform, decoded_frames,
          total_frames }` + `AudioFile::from_video_audio` (path = the video file). `data()`
          empty / `read_samples()` 0 (streamed). `Query::AddVideoAudioSync` +
          `do_add_video_audio` (probes via `VideoAudioReader::open`, no decode) +
          `EngineController::add_video_audio_sync`. `GetPoolAudioSamples` surfaces VideoAudio's
          waveform overview too. daw-backend compiles clean; probe `total_frames` test passes.
    - [x] **Step 3 (DONE):** reader thread now holds `StreamSource` (opens via
          `StreamSource::open(path, kind)`, dispatches `sample_rate()/channels()/seek/decode_next`);
          `ActivateFile` carries `kind: SourceKind`; `#[allow(dead_code)]` removed. `AddAudioClip`
          activation maps `Compressed`→`CompressedAudio`, `VideoAudio`→`VideoAudio`, creates the
          read-ahead buffer + `ActivateFile{kind}` + sets `clip.read_ahead`. Compressed path is
          behaviorally identical (StreamSource::Compressed wraps the same CompressedReader).
          daw-backend + editor compile clean; VideoAudioReader tests still pass.
          ⚠️ Not runtime-verified — needs in-app check that compressed audio still plays (no
          regression) and that an activated VideoAudio clip produces sound.
    - [x] **Step 4 (DONE):** `import_video` now calls `add_video_audio_sync(video_path)` →
          pool index, fetches channels/sample_rate via `get_pool_file_info`, makes the
          `AudioClip` with the video's duration. **No WAV / /tmp / RAM.** Removed the stopgap
          (`extract_audio_from_video_to_wav` + WAV helpers + `ExtractedAudioInfo`), dead
          `add_audio_file_sync` (+ `Query::AddAudioFileSync` / `QueryResponse::AudioFileAddedSync`
          / handler), and the now-unreachable `AudioExtractionResult::NoAudio`. Kept
          `import_audio_sync` (still used by normal audio import). daw-backend + editor clean.
          **→ Feature is live end-to-end; ready for in-app testing.**
    - [x] **Step 5 (DONE):** `CompressedReader` now seeks sample-accurately too — coarse
          symphonia seek + decode-discard (`pending_discard` set from `seeked.actual_ts` in
          `seek`, applied in `decode_next`, which continues rather than reporting EOF when a
          whole packet is discarded). So compressed clips no longer drift vs video audio after
          a seek. Test `compressed_reader_seek_is_sample_accurate` passes (the WAV coarse seek
          lands pre-target, exercising the discard). `CompressedReader` made `pub` for the test.
    - [~] Step 6: **bounded waveform overview** — replaces today's full-resolution
          `raw_audio_cache`/GPU waveform (which doesn't scale: it stores every sample at mip 0,
          so a long file is multi-GB on GPU + RAM — the same memory issue, and the Phase 1a
          `decode_progressive` leftover). Design below. Slices: (1a) streaming pyramid builder
          + (1b) persistence + (1c) min/max GPU upload, then (2) LRU tile cache + re-decode floor.
      - [x] **Slice 1a (DONE):** `daw-backend/src/audio/waveform_pyramid.rs` —
            `WaveformPyramidBuilder` streams interleaved samples, accumulates the floor, and
            reduces `BRANCH(4):1` at `finish` into a root-first pyramid (convention B:
            `levels[0]`=root envelope, `levels.last()`=floor, `.root()`/`.floor()` accessors).
            Ragged last buckets reduce over available children (no value padding). Bounded
            (~22 MB/2 h @ B=256). 7 integration tests pass (`tests/waveform_pyramid.rs`):
            bucket min/max, partial flush, multi-level envelope == global min/max, root-first
            ordering, stereo channels, size bound, chunk-agnostic.
      - [~] **Slice 1b (data layer DONE; orchestration folded into 1c):**
            - [x] Generation bridge `disk_reader::build_waveform_pyramid(path, kind, B)` — streams
                  a decode (`StreamSource` over symphonia/ffmpeg) into the builder; bounded
                  memory (one chunk + the pyramid). Test: envelope matches the signal through
                  both backends.
            - [x] Serialization `WaveformPyramid::to_bytes`/`from_bytes` (LBWF blob; f32 texels —
                  f16 a later size optimization). Round-trip test + rejects truncated/garbage.
            - [x] `MediaKind::Waveform` in the SQLite container (keyed by the audio item's id).
            - [ ] Orchestration (with 1c).
      - [~] **Slice 1c (in-memory floor overview DONE; persistence next):**
            - [x] `waveform_gpu`: `PendingUpload.minmax` flag + `pack_texel` helper; `upload_audio`
                  threads `minmax` (frame_stride 4, packs `(Lmin,Lmax,Rmin,Rmax)` directly).
                  The texture is already Rgba16Float and the GPU mipgen builds zoom-out levels, so
                  only the texel-packing differs. Render the floor at **effective rate `sr/B`** (so
                  time→texel maps B samples/texel) and `total_frames = floor_texel_count`.
            - [x] `AppConfig.waveform_floor_samples_per_texel` (default 256, user-configurable).
            - [x] App: `waveform_minmax_pools: HashMap<usize, u32>` (pool → `B`, carries the floor rate
                  with full float precision) + a `(pool, packed_floor, sr, channels, B)` results channel;
                  drained in `update()` → `raw_audio_cache.insert(floor)` + flag pool + `waveform_gpu_dirty`.
            - [x] Generation: on video-audio import Success, the same bg thread streams
                  `disk_reader::build_waveform_pyramid(path, VideoAudio, B)` once and sends the packed
                  `floor()`. (Video-audio has no in-RAM samples, so this is what makes its waveform appear.)
            - [x] Threaded `waveform_minmax_pools` through the pane-context (`panes/mod.rs` +
                  main.rs construction) → `render_layers` → **both** render sites (collapsed-group
                  ~timeline.rs:3048 AND expanded-track ~3613): compute `total_frames = len/4`,
                  `eff_sr = sr/B`, set `minmax`. Compiles clean (editor `cargo check` = 0 errors).
            - [x] Shader fix: `waveform.wgsl` now reads the **nearest integer LOD via `textureLoad`**
                  instead of sampling a fractional mip. Trilinear blends two levels whose row-major
                  linearizations differ → horizontal shift that flips each 0.5 of `mip_f` (= each 2x
                  zoom step), the "every other zoom level is offset" artifact. **User-confirmed fixed:**
                  features hold position at every zoom and line up with playback.
                  See memory `waveform-shader-fractional-mip-offset`.
            - [x] **Persistence (done):** the full pyramid is serialized (`to_bytes`) on generation and
                  kept in `App.waveform_pyramid_blobs`. `save_beam` writes it as a `MediaKind::Waveform`
                  row keyed by a **deterministic id derived from the pool index** (`file_io::waveform_media_id`,
                  "LBWF" sentinel in the high 32 bits) — independent of how the audio bytes are stored, so
                  it works for packed/referenced/video-audio alike, and an in-place re-save reuses the row.
                  Carried in/out via a transient `#[serde(skip)] AudioPoolEntry.waveform_blob` and a
                  `waveform_blobs` field on `FileCommand::Save`. `load_beam_sqlite` reads the row back;
                  the editor restores `raw_audio_cache`/`waveform_minmax_pools`/`waveform_pyramid_blobs`
                  + flags `waveform_gpu_dirty` after the backend loads the pool (using each entry's
                  `sample_rate` for `eff_sr`, the stored `B` for the rate). No re-decode on load.
                  `register_loaded_videos` only loads frames (not audio), so there is no redundant
                  regeneration to suppress. Compiles clean across all three crates.

### Waveform LOD pyramid design (step 6)
A min/max LOD pyramid (tree of zoom-level textures): fully zoomed out → envelope; fully zoomed
in → per-sample; seamless between.

- **One streaming decode pass** builds the whole pyramid down to a configurable **floor**
  `B` samples/texel (default 256), via a hierarchical reduction (each sample updates a running
  per-level min/max accumulator; a filled bucket emits a texel and folds into its parent —
  `branch` 4:1). Bounded memory: holds only the pyramid (~`N/B·4/3` texels ≈ **~14 MB / 2 h
  stereo @ B=256**), never the full samples. Full-res (B=1 ≈ 2.7 GB) is the only level NOT
  stored.
- **Persist the pyramid** in the `.beam` SQLite container (a `waveform` media kind; session
  temp before first save). `B` is stored with it (preference is just the default for new gen).
  Persistence is load-bearing: it makes mid-zoom a cheap **disk read**, not a re-decode.
- **Runtime = LRU tile cache** (GPU textures) loaded from the persisted pyramid on demand.
  Eviction is **ancestor-closed**: only evict an LRU node with no resident children ("a node is
  cleared only after its children") — so rendering can always walk up to a resident ancestor;
  detail sharpens in, never blanks. Root is tiny/hot → effectively pinned for free.
- **Re-decode only below the floor** (texel < `B` samples): by then the visible window spans a
  tiny time range, so decoding it (via the sample-accurate seekable readers from steps 1–5 —
  the payoff) for true per-sample detail is cheap. This removes the large-span re-decode gap:
  above the floor it's a disk read; below it the span is already small.
- **Why a deep floor (not a coarse cutoff):** a coarse-only pinned set would force the first
  on-demand level to re-reduce a huge time span per tile. Persisting deep makes every level a
  disk read; `B` is a size-vs-crossover knob (smaller B = bigger pyramid, cheaper re-decode).
- `waveform_gpu` needs a **min/max texel upload** (`Lmin,Lmax,Rmin,Rmax` per texel) instead of
  min=max-per-sample; the existing compute mipgen still builds the mip chain *within* a tile.

**Decisions (locked):** branch 4:1; floor `B≈256` samples/texel, **user-configurable**
(`AppConfig.waveform_floor_samples_per_texel`, stored per-pyramid); 8192-wide tiles; LRU ~4
viewports of fine tiles; persist pyramid in `.beam`.
- [x] Video decoder concurrency (movie-length lag/freeze): keyframe-index scan now runs
      holding no VideoManager/decoder lock (brief locks only bracket it) → no more multi-second
      UI freeze on import/load; thumbnail generation uses a **dedicated** decoder and samples
      at keyframes (≈1 frame each vs whole-GOP) → no playback contention. Removed dead
      `VideoManager::build_keyframe_index`, `build_and_set_keyframe_index`, `downsample_rgba*`.
- [x] Phase 2a — bound video frame cache. `VideoManager.frame_cache` (was an unbounded
      `HashMap<(Uuid,i64), Arc<VideoFrame>>` that grew per distinct frame during playback) is now an
      `LruCache` evicted by a **byte budget** (`FRAME_CACHE_BYTE_BUDGET` = 256 MB) rather than a frame
      count — robust across resolutions (a 4K frame is ~33 MB vs ~2 MB at 800×600). Byte total tracked
      on insert/evict/remove; `unload_video` pops per-clip keys (LruCache has no `retain`). Decoder-level
      cache was already LRU. Editor compiles clean. *(Not yet runtime-verified.)*
- [x] Phase 2b — stream export mux. `export/mod.rs::mux_video_and_audio` no longer collects every
      packet into two `Vec`s before interleaving; it stream-merges the two inputs by PTS with one pending
      packet per stream (O(1) memory vs O(duration)). Same tie-break (`v_us <= a_us`) and drain-on-EOF
      behavior; output is byte-identical. Editor compiles clean. *(Not yet runtime-verified — needs an
      in-app export to confirm A/V sync.)*
- [ ] Phase 3a — lazy raster fault-in from blob store
- [ ] Phase 3b — raster residency window + eviction
- [ ] Phase 3c — bound raster GPU/CPU caches
- [ ] Phase 3d — spill undo snapshots
- [ ] Phase 4a — frame→asset enumeration (recursive)
- [ ] Phase 4b — usage bookkeeping + LRU residency
- [ ] Phase 4c — bound decoded image tier
- [x] Phase 5 — fixed the broken `#[cfg(test)]` unit tests; **`cargo test --lib` green again**
      (daw-backend 17 passed, lightningbeam-core 264 passed). Wrapped stale raw-`f64` time literals
      in `Beats(...)` / passed `&TempoMap` to changed signatures (automation.rs, clip.rs,
      effect_layer.rs); fixed stale test setup (register a vector clip so `get_clip_duration` resolves)
      and a stale default expectation (shape `fill_color` defaults `None`). Surfaced + fixed one **real
      undo bug**: `DeleteFolderAction(MoveToParent)` reparented child subfolders but never restored them
      on rollback (orphaned them) — now tracked and restored. Production code otherwise untouched.

# `.beam` File Format Specification

**Status:** Normative.
**Container schema version:** `1` (`meta.schema_version`).
**Project payload version:** `1.0.0` (`BeamProject.version` / `meta.version`).
**Last updated:** 2026-06-21.

This document specifies the on-disk format of Lightningbeam `.beam` project files
precisely enough that an independent implementation can read and write them. It
describes the **current** format — a SQLite database — and the **legacy** format
(a ZIP archive) that current readers still accept and migrate.

> An earlier revision of this document described the ZIP container as the current
> format. That is now historical; see [§11 Legacy ZIP format](#11-legacy-zip-format-pre-sqlite).

## 1. Notational conventions

The key words **MUST**, **MUST NOT**, **SHOULD**, **SHOULD NOT**, and **MAY** are
to be interpreted as described in RFC 2119.

Byte sizes use binary units: `KiB = 1024`, `MiB = 1024²`, `GiB = 1024³`.

"UUID" means an RFC 4122 128-bit identifier. Unless stated otherwise, a UUID is
serialized in JSON as its canonical hyphenated lowercase string, and stored in
SQLite as its 16 raw bytes in **network (big-endian) byte order** — i.e. the byte
sequence returned by `Uuid::as_bytes`, most significant byte first.

## 2. Overview

A `.beam` file is a **single SQLite 3 database** (default rollback journal; no WAL,
no special pragmas) containing:

- one JSON document, `project.json`, holding all project state that is small and
  structural (the document/scene tree, audio project, asset metadata); and
- zero or more **media** items (audio, raster frames, image assets, and derived
  blobs such as waveforms and thumbnails), each either **packed** into the database
  in chunks or **referenced** by an external file path.

The design goals are: a single beginner-friendly file (no project folder);
streaming reads of large media via SQLite blob I/O; transactional, crash-safe,
**in-place** saves that do not rewrite unchanged media; and inspectability with the
ordinary `sqlite3` CLI.

## 3. File identification

A reader MUST determine the container type from the first 16 bytes of the file:

- If the first 16 bytes equal the ASCII string `SQLite format 3\0` (i.e.
  `53 51 4C 69 74 65 20 66 6F 72 6D 61 74 20 33 00`), the file is a SQLite
  `.beam` and MUST be read per [§4](#4-container-the-sqlite-schema)–[§10](#10-load-semantics).
- Otherwise the file is treated as a [legacy ZIP `.beam`](#11-legacy-zip-format-pre-sqlite)
  (ZIP local-file-header magic `50 4B 03 04`).

The `.beam` extension is used for both container types; the magic bytes — not the
extension — are authoritative.

## 4. Container: the SQLite schema

A conforming SQLite `.beam` MUST contain exactly these four tables (DDL as created;
column order and types are normative):

```sql
CREATE TABLE media (
    id          BLOB PRIMARY KEY,   -- 16-byte UUID (big-endian)
    kind        INTEGER NOT NULL,   -- MediaKind (§6.1)
    codec       TEXT    NOT NULL,   -- original codec/container, e.g. "flac","mp3","png"
    storage     INTEGER NOT NULL,   -- MediaStorage (§6.2): 0=Packed, 1=Referenced
    ext_path    TEXT,               -- external path, set iff storage=Referenced
    total_len   INTEGER NOT NULL DEFAULT 0,  -- payload length in bytes (packed); 0 if referenced
    channels    INTEGER,            -- audio: channel count (nullable)
    sample_rate INTEGER,            -- audio: sample rate in Hz (nullable)
    width       INTEGER,            -- visual media: pixel width (nullable)
    height      INTEGER             -- visual media: pixel height (nullable)
);

CREATE TABLE media_chunk (
    id          INTEGER PRIMARY KEY,  -- rowid (used as the handle for blob streaming)
    media_id    BLOB NOT NULL,        -- → media.id
    chunk_index INTEGER NOT NULL,     -- 0-based ordinal of this chunk
    bytes       BLOB NOT NULL,        -- chunk payload
    UNIQUE (media_id, chunk_index)
);

CREATE TABLE project_json (
    id   INTEGER PRIMARY KEY CHECK (id = 0),  -- exactly one row, id 0
    data TEXT NOT NULL                        -- the serialized BeamProject (§8)
);

CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
```

There is no foreign-key constraint between `media_chunk.media_id` and `media.id`;
referential integrity is maintained by the writer. A reader MUST NOT assume SQLite
enforces it.

### 4.1 The `meta` table

Defined keys (all values are strings):

| Key              | Value              | Meaning |
|------------------|--------------------|---------|
| `schema_version` | `"1"`              | Container schema version. Set once at creation. |
| `version`        | `"1.0.0"`          | Project payload version (mirrors `project.json`'s `version`). |
| `created`        | RFC 3339 timestamp | Creation time. Preserved across in-place re-saves. |
| `modified`       | RFC 3339 timestamp | Last save time. Updated on every save. |

Writers MAY add additional keys; readers MUST ignore unknown keys.

### 4.2 Two independent version numbers

The format carries **two** version numbers with different compatibility rules:

- **`meta.schema_version`** (`INTEGER`, currently `1`) — versions the SQLite
  container/table layout. A reader MUST reject a file whose `schema_version` is
  **greater** than the highest it supports, and SHOULD accept any value less than
  or equal to its maximum (forward-compatible for older files).
- **`BeamProject.version`** / `meta.version` (currently `"1.0.0"`) — versions the
  `project.json` payload. The current implementation requires **exact string
  equality** and rejects any other value. A future revision MAY relax this to a
  semantic-version range; until then, writers MUST emit exactly `"1.0.0"` and
  readers reject anything else.

These two numbers are orthogonal and MUST both be checked.

## 5. UUIDs and identity

Every media item is identified by a UUID. In `project.json` UUIDs appear as
canonical strings; in the `media`/`media_chunk` tables they appear as 16 raw
big-endian bytes (`media.id`, `media_chunk.media_id`). A reader MUST treat the two
representations as equal by their 128-bit value.

`media_chunk.id` is the SQLite rowid and is the handle a reader uses to open a blob
for streaming (`sqlite3_blob_open` on table `media_chunk`, column `bytes`). It has
no meaning beyond row identity and MUST NOT be relied upon to be stable across
re-saves.

## 6. Media model

### 6.1 `MediaKind` (`media.kind`)

| Value | Kind          | `codec` examples | Notes |
|-------|---------------|------------------|-------|
| `0`   | `Audio`       | `flac`, `mp3`, `wav`, `ogg`, `opus`, `aac`, `m4a`, `alac`, `caf`, `aiff` | Source audio for a pool entry. |
| `1`   | `Video`       | —                | **Reserved/unused.** The current writer never emits video rows; video bytes live in an external file referenced by `VideoClip.file_path`. |
| `2`   | `Raster`      | `png`            | Full-resolution pixels of a raster keyframe (PNG-encoded RGBA). |
| `3`   | `ImageAsset`  | `png`, `jpg`, …  | An imported image asset's original bytes. |
| `4`   | `Waveform`    | `lbwf`           | Precomputed waveform LOD pyramid for an audio item. Opaque blob owned by `daw_backend::audio::waveform_pyramid`. |
| `5`   | `Thumbnail`   | `lbtn`           | A pack of precomputed video thumbnails for a clip. Opaque blob owned by the editor. |
| `6`   | `RasterProxy` | `png`            | Low-resolution PNG proxy of a raster keyframe, shown during cold scrubs while full-res pages in. |

A reader MUST reject a `kind` value it does not recognise only if it needs that
item; unknown kinds MAY otherwise be ignored.

### 6.2 `MediaStorage` (`media.storage`)

| Value | Storage      | Bytes location | `total_len` | `ext_path` |
|-------|--------------|----------------|-------------|------------|
| `0`   | `Packed`     | `media_chunk` rows in this DB | payload length | `NULL` |
| `1`   | `Referenced` | external file at `ext_path`   | `0`           | path string |

### 6.3 Packed storage and chunking

Packed bytes are split into chunks of **`CHUNK_SIZE = 4 MiB`** and stored one chunk
per `media_chunk` row, ordered by `chunk_index` ascending starting at `0`. The
writer MUST:

1. set `media.total_len` to the exact total byte length of the payload;
2. write `ceil(total_len / CHUNK_SIZE)` chunk rows (zero rows iff `total_len == 0`);
3. make every chunk exactly `CHUNK_SIZE` bytes **except the last**, which holds the
   remainder `total_len − (n−1)·CHUNK_SIZE`.

Because chunk lengths are fully determined by `total_len` and `CHUNK_SIZE`, a reader
performing random access MUST compute, for a byte offset `pos < total_len`:

```
chunk_index     = pos / CHUNK_SIZE
offset_in_chunk = pos % CHUNK_SIZE
chunk_len       = min(CHUNK_SIZE, total_len − chunk_index·CHUNK_SIZE)
```

and read from the row with that `chunk_index`. A reader MAY stream the whole payload
by concatenating chunk `bytes` in `chunk_index` order. The chunk size MAY differ in
files written by other tools; a robust reader SHOULD derive chunk boundaries from
actual row lengths rather than assuming 4 MiB. (The reference reader assumes uniform
`CHUNK_SIZE` except for the last chunk; writers targeting it MUST keep chunks
uniform.)

### 6.4 Referenced storage

A referenced item stores only `ext_path` (with `total_len = 0` and no chunk rows).
The path is resolved relative to the directory containing the `.beam` file unless it
is absolute. If the file is absent at load time, the item is reported as a *missing
file* ([§10.4](#104-missing-referenced-files)) — this is non-fatal.

## 7. Derived media IDs

Three media kinds are keyed by UUIDs **derived** from another id rather than by an
independent random UUID. A reader/writer MUST compute them exactly as follows
(`from_u128` constructs a UUID from a 128-bit big-endian integer):

| Kind          | Derived from        | Formula |
|---------------|---------------------|---------|
| `Waveform`    | audio pool **index** `i` (`usize`) | `from_u128((0x4C42_5746 << 96) \| (i as u128))` — high 32 bits = `0x4C425746` ("LBWF"), low 96 bits = the pool index. |
| `Thumbnail`   | video clip UUID `c` | `from_u128(c.as_u128() ^ 0x4C42_544E_4C42_544E_4C42_544E_4C42_544E)` — full-width XOR with "LBTN" repeated 4×. |
| `RasterProxy` | raster keyframe UUID `k` | `from_u128(k.as_u128() ^ 0x4C42_5058_4C42_5058_4C42_5058_4C42_5058)` — full-width XOR with "LBPX" repeated 4×. |

The full-resolution `Raster` row is keyed by the keyframe's **own** UUID (no
derivation); an `ImageAsset` row is keyed by the asset's own UUID; a packed `Audio`
row is keyed by the pool entry's `media_id`.

> Note the asymmetry: the waveform id ORs the pool index into the high bits, while
> thumbnail/proxy ids XOR a 128-bit sentinel with a source UUID. This is intentional
> (waveforms are keyed by *index*, not by a UUID) but is a sharp edge for
> implementers.

## 8. `project.json`

`project_json.data` is a UTF-8 JSON document: the serde serialization of a
`BeamProject`. This section specifies the top-level structure and the entities a
reader needs to resolve media; the deep UI/scene tree is defined by the
corresponding Rust types (serde field names match struct field names unless a
`#[serde(rename)]` is noted) and is intentionally **not** enumerated exhaustively
here.

### 8.1 `BeamProject` (root)

| Field           | Type                      | Notes |
|-----------------|---------------------------|-------|
| `version`       | string                    | MUST be `"1.0.0"`. |
| `created`       | string (RFC 3339)         | |
| `modified`      | string (RFC 3339)         | |
| `ui_state`      | `Document`                | The scene/document tree (§8.2). |
| `audio_backend` | `SerializedAudioBackend`  | Audio engine state (§8.3). |

### 8.2 `Document` (top-level fields)

Collections that carry media linkage are **bold**:

- `id: Uuid`, `name: string`, `background_color`, `width: f64`, `height: f64`,
  `framerate: f64`, `duration: f64`
- `time_signature`, `master_layer`, `timeline_mode` — all `#[serde(default)]`
- `root: GraphicsObject` — the layer tree; raster keyframes live here and inside
  nested group/clip layers
- **`image_assets: map<Uuid, ImageAsset>`** — keyed by asset UUID (= the media id)
- **`video_clips: map<Uuid, VideoClip>`** — `VideoClip.file_path` is the external
  video file; thumbnails are derived media (§7)
- `vector_clips`, `audio_clips`, `instance_groups`, `effect_definitions`,
  `script_definitions`, the various `*_folders` asset trees — structural, no direct
  media-row linkage
- `ui_layout`, `ui_layout_base` — `Option`, skipped when `None`
- `current_time`, `layer_to_clip_map` — `#[serde(skip)]` (not persisted)

`RasterKeyframe` (within the layer tree) has `id: Uuid` (= its full-res `Raster`
media id and the seed for its `RasterProxy` id), `time`, `width`, `height`,
`tween_after`, and `stroke_log`. Pixel buffers are `#[serde(skip)]` and faulted in
from media rows. (The legacy ZIP entry path `media/raster/<id>.png` is derived from
`id`; older files may also carry a now-ignored `media_path` field.)

### 8.3 `SerializedAudioBackend`

| Field                | Type                | Notes |
|----------------------|---------------------|-------|
| `sample_rate`        | `u32`               | The project's system sample rate (mirrors `AudioProject.sample_rate`). Per-file rates are on each `AudioPoolEntry.sample_rate`. |
| `project`            | `AudioProject`      | Tracks + MIDI clip pool (§8.4). |
| `audio_pool_entries` | `[AudioPoolEntry]`  | Audio source registry (§8.5). |
| `layer_to_track_map` | `map<Uuid, u32>`    | UI layer UUID → engine track id. `#[serde(default)]`. |

### 8.4 `AudioProject` (top-level fields)

`tracks: map<TrackId, TrackNode>`, `root_tracks: [TrackId]`, `next_track_id`,
`sample_rate: u32`, `midi_clip_pool`, `next_midi_clip_instance_id`. DSP graphs are
not serialized; they are rebuilt on load.

### 8.5 `AudioPoolEntry` (media linkage)

| Field           | Type                       | Role |
|-----------------|----------------------------|------|
| `pool_index`    | `usize`                    | Stable index; seeds the `Waveform` media id (§7). |
| `name`          | string                     | |
| `media_id`      | `Option<string>` (UUID)    | Set ⇔ audio bytes are **packed** in this DB under that id. `#[serde(default, skip_serializing_if=None)]`. |
| `relative_path` | `Option<string>`           | Set ⇔ audio is **referenced** (external file) or missing. |
| `embedded_data` | `Option<EmbeddedAudioData>`| Legacy/inline: `{ data_base64, format }`. Used when bytes are neither packed nor referenced. |
| `sample_rate`   | `u32`                      | Authoritative sample rate. |
| `channels`      | `u32`                      | Channel count. |
| `duration`      | `f64`                      | Seconds. |
| `is_video_audio`| `bool`                     | If set, the entry is the audio track of a video; always stored **referenced**. `#[serde(default, skip_if=false)]`. |
| `waveform_blob` | (transient)                | `#[serde(skip)]`; carries waveform bytes in memory only. |

### 8.6 Media linkage summary

| Media kind         | Referenced from `project.json` by | Media-row id |
|--------------------|-----------------------------------|--------------|
| Audio (packed)     | `AudioPoolEntry.media_id`         | that UUID |
| Audio (referenced) | `AudioPoolEntry.relative_path`    | — (external) |
| Audio (embedded)   | `AudioPoolEntry.embedded_data`    | — (inline base64) |
| Raster (full)      | `RasterKeyframe.id`               | that UUID |
| Raster proxy       | derived from `RasterKeyframe.id`  | §7 |
| Image asset        | `Document.image_assets` key / `ImageAsset.id` | that UUID |
| Video bytes        | `VideoClip.file_path`             | — (always external) |
| Video thumbnails   | derived from `video_clips` key    | §7 |
| Waveform           | derived from `AudioPoolEntry.pool_index` | §7 |

## 9. Save semantics

A conforming writer MUST perform all media, `project.json`, and `meta` writes for a
save inside **one** SQLite transaction.

### 9.1 In-place vs. create

- If the target path exists **and** is already a SQLite `.beam`, the writer opens it
  and writes in place. Unchanged packed media MUST NOT be rewritten (§9.3); this is
  the central performance/crash-safety property, and writers MUST NOT use a
  copy-to-temp-and-rename flow for in-place saves.
- Otherwise (new file, or migrating a legacy ZIP), the writer creates a fresh DB at
  `<path>.beam.tmp`, writes everything, commits, then atomically `rename`s it over
  the target.

### 9.2 Order of operations within the transaction

1. Audio pool entries → `Audio` (+ `Waveform`) media rows.
2. Raster keyframes → `Raster` (+ `RasterProxy`) media rows.
3. Video thumbnail packs → `Thumbnail` media rows.
4. Image assets → `ImageAsset` media rows.
5. **Garbage-collect**: delete every `media` row (and its chunks) whose id is **not**
   in the set of live ids accumulated in steps 1–4.
6. Write `project.json` and the `meta` keys (`version`, `created`, `modified`).
7. Commit; then (create path only) rename the temp file over the target.

### 9.3 Packed vs. referenced decision (audio)

For each audio pool entry, in order:

- If a packed row for its `media_id` already exists in the archive, keep it untouched
  (do not rewrite the bytes) and keep the entry packed.
- Else if its external file resolves and exists: store **referenced** if the entry is
  video audio, *or* the file is `≥ LARGE_MEDIA_THRESHOLD` (2 GiB) and the large-media
  mode is not `Pack`; otherwise **pack** it (streamed from disk chunk-by-chunk).
- Else if it has `embedded_data`: base64-decode and pack it.
- Else: leave its references as-is (it will be reported missing on load).

Raster, image, and thumbnail media follow the same "write if present, else keep the
existing row" rule so that paged-out content survives a save without being held in
memory.

## 10. Load semantics

### 10.1 Dispatch and version checks

Open the file, read `project.json`, parse `BeamProject`. Reject unless
`version == "1.0.0"` (§4.2). The container's `schema_version` is verified on open
(`≤` the reader's maximum).

### 10.2 Audio resolution

Per pool entry with a `media_id`: look up the media row. If its `codec` is a
**streamable** audio codec (`mp3`, `flac`, `ogg`/`oga`, `wav`/`wave`, `aiff`/`aif`,
`aac`, `m4a`, `opus`, `alac`, `caf`), the bytes are **streamed lazily** from the blob
at playback time (the entry keeps `media_id`, with `embedded_data`/`path` cleared).
Otherwise the bytes are read eagerly into `embedded_data`. A precomputed `Waveform`
blob, if present, is read into the entry.

### 10.3 Visual media paging

- **Raster** full-res pixels are **not** eagerly decoded; each keyframe is flagged
  for fault-in and its pixels are paged from its `Raster` row on demand. Its
  `RasterProxy` PNG **is** read and decoded eagerly so cold scrubs show a low-res
  frame immediately.
- **Image assets** are **not** eagerly read; they page from their `ImageAsset` rows
  on demand.
- **Thumbnail** packs are read eagerly per video clip.

### 10.4 Missing referenced files

A pool entry that is neither packed nor embedded, whose `relative_path` resolves to a
non-existent file, MUST be reported as a missing file (non-fatal). The host
application prompts the user to relocate it.

## 11. Legacy ZIP format (pre-SQLite)

Files whose magic is not `SQLite format 3\0` are read as a ZIP archive with this
internal layout:

- `project.json` — the same serialized `BeamProject` (`version` must be `"1.0.0"`).
- `media/audio/<name>.<ext>` — audio source files. The codec is taken from the
  extension. **FLAC entries are decoded to PCM `f32` and re-encoded in memory as
  IEEE-float WAV (WAV format tag 3)** before being base64-embedded; other formats are
  embedded as-is. The entry's `relative_path` is cleared after extraction.
- `media/raster/<uuid>.png` — raster keyframe pixels, named `media/raster/<id>.png`
  after the keyframe's `id`.

A reader MUST NOT modify a legacy ZIP on open; it loads into memory only. **The next
save migrates the project to SQLite** (the save sees a non-SQLite target, so it takes
the create-and-rename path and writes a fresh `.beam` database).

> Known limitation of the legacy reader: raster pixels are loaded only for
> **top-level** layers; raster keyframes nested inside groups/clips are not populated
> from a ZIP. The SQLite path recurses correctly. Re-saving to SQLite is the remedy.

## 12. Large-media policy

- `LARGE_MEDIA_THRESHOLD = 2 GiB`. Files at or above it trigger the packed-vs-
  referenced decision below; smaller files are always packed (unless video audio,
  which is always referenced).
- `LargeMediaMode` ∈ `{ Ask, Pack, Reference }`:
  - `Pack` — pack large files into the database.
  - `Reference` — store large files by external path.
  - `Ask` (default) — prompt the user on the first large import, then persist their
    choice; treated as `Reference` at save time until answered.
- **The chosen mode is an application preference, not part of the `.beam` file.** It
  is stored in the editor's `config.json` (`AppConfig.large_media_default`) under the
  platform config directory, not in the project. Readers/writers of the format need
  not consult it except to drive the packed/referenced save decision.

## 13. Conformance summary

A conforming **reader** MUST:

1. dispatch on the 16-byte magic (§3);
2. reject `schema_version` greater than supported and `version != "1.0.0"` (§4.2);
3. resolve packed media via `media`/`media_chunk` (§6.3) and referenced media
   relative to the file (§6.4), reporting missing referenced files non-fatally;
4. compute derived media ids exactly as in §7.

A conforming **writer** MUST:

1. produce the four tables of §4 with `schema_version="1"` and `version="1.0.0"`;
2. store UUIDs as 16 big-endian bytes and chunk packed media at a uniform size with a
   remainder final chunk, setting `total_len` correctly (§6.3);
3. perform each save in a single transaction and garbage-collect orphaned media (§9),
   and prefer in-place writes that do not rewrite unchanged packed media.

## 14. Security considerations

- **Path traversal:** referenced media and legacy ZIP entries carry filesystem paths.
  Readers SHOULD resolve relative paths against the project directory and SHOULD
  reject or sandbox paths that escape it (`..`, absolute paths) when opening
  untrusted files.
- **Resource limits:** `project.json` and packed blobs are attacker-controllable in
  an untrusted file. Readers SHOULD bound JSON size and avoid loading entire large
  blobs into memory (stream via §6.3) to resist memory-exhaustion.
- **Decoder safety:** audio/image bytes are handed to media decoders (Symphonia,
  image, FFmpeg); keep those dependencies current, as they parse untrusted input.
- **Legacy ZIP bombs:** the ZIP path SHOULD enforce sane decompression-ratio/size
  limits.

## 15. Non-normative notes / known quirks

These reflect the current reference implementation and are flagged for implementers
and future spec revisions:

- `MediaKind::Video` (`1`) is defined but never written; video media is always an
  external file via `VideoClip.file_path`.
- The project `version` check is exact-match with no compatibility window; a future
  revision should define semantic-version tolerance before bumping it.

(Earlier revisions of this section flagged a hardcoded save sample rate, a vestigial
`RasterKeyframe.media_path` field, and unused `SaveSettings` fields; these have since
been fixed/removed.)

## 16. Inspecting a `.beam` file

Because the container is plain SQLite, any `.beam` (SQLite variant) can be inspected
with standard tooling, e.g.:

```sh
sqlite3 project.beam '.tables'
sqlite3 project.beam 'SELECT key,value FROM meta;'
sqlite3 project.beam 'SELECT hex(id),kind,codec,storage,total_len FROM media;'
sqlite3 project.beam 'SELECT data FROM project_json;' | jq .
```

## 17. References

- Container: `lightningbeam-ui/lightningbeam-core/src/beam_archive.rs`
- Save/load orchestration & `BeamProject`: `lightningbeam-ui/lightningbeam-core/src/file_io.rs`
- Audio pool entry: `daw-backend/src/audio/pool.rs`
- Document / scene tree: `lightningbeam-ui/lightningbeam-core/src/document.rs`
- RFC 2119 (requirement keywords), RFC 4122 (UUID), RFC 3339 (timestamps)

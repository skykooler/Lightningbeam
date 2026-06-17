//! SQLite-backed `.beam` project container.
//!
//! The `.beam` format is a single SQLite database file. It replaces the older
//! ZIP-archive format. SQLite gives us, in one file:
//!
//! - **Streaming reads** — packed media is split into chunk rows and read on
//!   demand through [`BlobReader`] (`Read + Seek`), so arbitrary-length audio /
//!   video never has to be fully decoded into RAM on load.
//! - **In-place, crash-safe mutation** — raster frame write-back and re-save are
//!   transactional `UPDATE`s rather than rewriting a whole archive.
//! - **Single-file UX** — behaves like a file on every platform.
//!
//! ## Media storage
//!
//! Each media item is one row in `media` plus, when *packed*, N rows in
//! `media_chunk`:
//!
//! - **Packed** (`MediaStorage::Packed`) — bytes live in the database, split
//!   into [`CHUNK_SIZE`]-byte chunks. Chunking keeps each blob well under
//!   SQLite's ~2 GB per-blob ceiling and bounds the working set of a streaming
//!   reader to a single chunk.
//! - **Referenced** (`MediaStorage::Referenced`) — only an external path is
//!   stored; the bytes stay on disk (useful for shared media on a network drive,
//!   or media too large/volatile to pack). Callers open the path directly.
//!
//! `project.json` (the serialized `BeamProject`) is stored verbatim in the
//! single-row `project_json` table; only the container and media storage change
//! relative to the legacy format.

use rusqlite::blob::Blob;
use rusqlite::{Connection, DatabaseName, OpenFlags, OptionalExtension};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use uuid::Uuid;

/// Default packed-media chunk size: 4 MiB.
///
/// Small enough to bound a streaming reader's per-chunk work and any
/// whole-chunk buffering, large enough to keep row counts modest (a 1 GB file
/// is 256 rows). Comfortably under SQLite's ~2 GB per-blob limit.
pub const CHUNK_SIZE: u64 = 4 * 1024 * 1024;

/// Files at or above this size prompt the user to pick packed vs referenced
/// (and the choice is then persisted as the default). Matches SQLite's
/// practical large-blob threshold.
pub const LARGE_MEDIA_THRESHOLD: u64 = 2 * 1024 * 1024 * 1024;

/// Kind of media stored in the `media` table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Audio = 0,
    Video = 1,
    Raster = 2,
    ImageAsset = 3,
    /// A precomputed waveform LOD pyramid blob for an audio item (keyed by the
    /// same id as the audio it describes). See `daw_backend::audio::waveform_pyramid`.
    Waveform = 4,
}

impl MediaKind {
    fn from_i64(v: i64) -> Option<Self> {
        match v {
            0 => Some(Self::Audio),
            1 => Some(Self::Video),
            2 => Some(Self::Raster),
            3 => Some(Self::ImageAsset),
            4 => Some(Self::Waveform),
            _ => None,
        }
    }
}

/// How a media item's bytes are stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaStorage {
    /// Bytes are chunked into `media_chunk` rows inside the database.
    Packed = 0,
    /// Only an external path is stored; bytes live on disk.
    Referenced = 1,
}

impl MediaStorage {
    fn from_i64(v: i64) -> Option<Self> {
        match v {
            0 => Some(Self::Packed),
            1 => Some(Self::Referenced),
            _ => None,
        }
    }
}

/// Metadata row for a media item (no bytes).
#[derive(Debug, Clone)]
pub struct MediaInfo {
    pub id: Uuid,
    pub kind: MediaKind,
    /// Original codec / container extension, e.g. `"flac"`, `"mp3"`, `"png"`.
    pub codec: String,
    pub storage: MediaStorage,
    /// Set when `storage == Referenced`.
    pub ext_path: Option<String>,
    /// Total byte length of the media payload (packed only; 0 for referenced).
    pub total_len: u64,
    // Kind-specific metadata (nullable; meaning depends on `kind`).
    pub channels: Option<u32>,
    pub sample_rate: Option<u32>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// Optional kind-specific metadata supplied when writing a media item.
#[derive(Debug, Clone, Copy, Default)]
pub struct MediaMeta {
    pub channels: Option<u32>,
    pub sample_rate: Option<u32>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// A `.beam` project container backed by a SQLite database.
pub struct BeamArchive {
    conn: Connection,
    chunk_size: u64,
}

impl BeamArchive {
    /// Schema version stored in `meta` under `"schema_version"`.
    pub const SCHEMA_VERSION: i64 = 1;

    /// Create a new (empty) archive at `path`, replacing any existing file.
    pub fn create(path: &Path) -> Result<Self, String> {
        // Remove any existing file so we start from a clean schema.
        if path.exists() {
            std::fs::remove_file(path).map_err(|e| format!("Failed to replace {:?}: {}", path, e))?;
        }
        let conn = Connection::open(path).map_err(map_sql)?;
        let mut archive = Self { conn, chunk_size: CHUNK_SIZE };
        archive.init_schema()?;
        Ok(archive)
    }

    /// Open an existing archive for read/write.
    pub fn open(path: &Path) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(map_sql)?;
        let archive = Self { conn, chunk_size: CHUNK_SIZE };
        archive.verify_schema()?;
        Ok(archive)
    }

    /// Quick check: does `path` look like a SQLite database (vs. a legacy ZIP)?
    /// Reads the 16-byte SQLite header magic. Used to route between the SQLite
    /// loader and the legacy-ZIP migration path.
    pub fn is_sqlite(path: &Path) -> bool {
        use std::io::Read as _;
        let mut f = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(_) => return false,
        };
        let mut magic = [0u8; 16];
        if f.read_exact(&mut magic).is_err() {
            return false;
        }
        &magic == b"SQLite format 3\0"
    }

    fn init_schema(&mut self) -> Result<(), String> {
        self.conn
            .execute_batch(
                "BEGIN;
                 CREATE TABLE media (
                     id          BLOB PRIMARY KEY,   -- 16-byte Uuid
                     kind        INTEGER NOT NULL,
                     codec       TEXT    NOT NULL,
                     storage     INTEGER NOT NULL,
                     ext_path    TEXT,
                     total_len   INTEGER NOT NULL DEFAULT 0,
                     channels    INTEGER,
                     sample_rate INTEGER,
                     width       INTEGER,
                     height      INTEGER
                 );
                 CREATE TABLE media_chunk (
                     id          INTEGER PRIMARY KEY,  -- rowid, for blob_open
                     media_id    BLOB NOT NULL,
                     chunk_index INTEGER NOT NULL,
                     bytes       BLOB NOT NULL,
                     UNIQUE (media_id, chunk_index)
                 );
                 CREATE TABLE project_json (
                     id   INTEGER PRIMARY KEY CHECK (id = 0),
                     data TEXT NOT NULL
                 );
                 CREATE TABLE meta (
                     key   TEXT PRIMARY KEY,
                     value TEXT NOT NULL
                 );
                 COMMIT;",
            )
            .map_err(map_sql)?;
        self.set_meta("schema_version", &Self::SCHEMA_VERSION.to_string())?;
        Ok(())
    }

    fn verify_schema(&self) -> Result<(), String> {
        let v: Option<String> = self.get_meta("schema_version")?;
        match v.as_deref().and_then(|s| s.parse::<i64>().ok()) {
            Some(n) if n <= Self::SCHEMA_VERSION => Ok(()),
            Some(n) => Err(format!(
                "Unsupported .beam schema version {} (this build supports up to {})",
                n,
                Self::SCHEMA_VERSION
            )),
            None => Err("Not a valid .beam archive (missing schema_version)".to_string()),
        }
    }

    /// Begin a write transaction grouping multiple media/json writes into one
    /// atomic, crash-safe commit. Used by saves so unchanged (large) media is
    /// never rewritten — only dirty rows are touched, in place.
    pub fn transaction(&mut self) -> Result<BeamTxn<'_>, String> {
        let tx = self.conn.transaction().map_err(map_sql)?;
        Ok(BeamTxn { tx, chunk_size: self.chunk_size })
    }

    // -- meta key/value --------------------------------------------------

    pub fn set_meta(&self, key: &str, value: &str) -> Result<(), String> {
        set_meta_conn(&self.conn, key, value)
    }

    pub fn get_meta(&self, key: &str) -> Result<Option<String>, String> {
        self.conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()
            .map_err(map_sql)
    }

    // -- project.json ----------------------------------------------------

    /// Store the serialized `project.json` (single row).
    pub fn set_project_json(&self, json: &str) -> Result<(), String> {
        set_project_json_conn(&self.conn, json)
    }

    /// Read the serialized `project.json`.
    pub fn get_project_json(&self) -> Result<String, String> {
        self.conn
            .query_row("SELECT data FROM project_json WHERE id = 0", [], |r| r.get(0))
            .optional()
            .map_err(map_sql)?
            .ok_or_else(|| "Archive has no project.json".to_string())
    }

    // -- media write -----------------------------------------------------

    /// Write a media item whose bytes are packed (chunked) into the database.
    /// Replaces any existing rows for `id`.
    pub fn put_media_packed(
        &mut self,
        id: Uuid,
        kind: MediaKind,
        codec: &str,
        bytes: &[u8],
        meta: MediaMeta,
    ) -> Result<(), String> {
        let tx = self.conn.transaction().map_err(map_sql)?;
        write_media_packed(&tx, self.chunk_size, id, kind, codec, bytes, meta)?;
        tx.commit().map_err(map_sql)?;
        Ok(())
    }

    /// Write a media item that references an external file by path (no bytes
    /// stored). Replaces any existing rows for `id`.
    pub fn put_media_referenced(
        &mut self,
        id: Uuid,
        kind: MediaKind,
        codec: &str,
        ext_path: &str,
        meta: MediaMeta,
    ) -> Result<(), String> {
        let tx = self.conn.transaction().map_err(map_sql)?;
        write_media_referenced(&tx, id, kind, codec, ext_path, meta)?;
        tx.commit().map_err(map_sql)?;
        Ok(())
    }

    // -- media read ------------------------------------------------------

    /// Look up a media item's metadata.
    pub fn media_info(&self, id: Uuid) -> Result<Option<MediaInfo>, String> {
        let id_bytes = id.as_bytes().to_vec();
        self.conn
            .query_row(
                "SELECT kind, codec, storage, ext_path, total_len, channels, sample_rate, width, height
                 FROM media WHERE id = ?1",
                [&id_bytes],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, i64>(4)?,
                        r.get::<_, Option<u32>>(5)?,
                        r.get::<_, Option<u32>>(6)?,
                        r.get::<_, Option<u32>>(7)?,
                        r.get::<_, Option<u32>>(8)?,
                    ))
                },
            )
            .optional()
            .map_err(map_sql)?
            .map(|(kind, codec, storage, ext_path, total_len, channels, sample_rate, width, height)| {
                Ok(MediaInfo {
                    id,
                    kind: MediaKind::from_i64(kind)
                        .ok_or_else(|| format!("Unknown media kind {}", kind))?,
                    codec,
                    storage: MediaStorage::from_i64(storage)
                        .ok_or_else(|| format!("Unknown media storage {}", storage))?,
                    ext_path,
                    total_len: total_len.max(0) as u64,
                    channels,
                    sample_rate,
                    width,
                    height,
                })
            })
            .transpose()
    }

    /// List every media item of a given kind.
    pub fn media_ids_of_kind(&self, kind: MediaKind) -> Result<Vec<Uuid>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM media WHERE kind = ?1")
            .map_err(map_sql)?;
        let rows = stmt
            .query_map([kind as i64], |r| r.get::<_, Vec<u8>>(0))
            .map_err(map_sql)?;
        let mut out = Vec::new();
        for row in rows {
            let bytes = row.map_err(map_sql)?;
            out.push(uuid_from_bytes(&bytes)?);
        }
        Ok(out)
    }

    /// Read an entire packed media item into memory. Convenience for small
    /// media (raster frames, image assets); large media should stream via
    /// [`BeamArchive::open_blob_reader`] instead.
    pub fn read_media_full(&self, id: Uuid) -> Result<Vec<u8>, String> {
        let info = self
            .media_info(id)?
            .ok_or_else(|| format!("Media {} not found", id))?;
        if info.storage != MediaStorage::Packed {
            return Err(format!("Media {} is referenced, not packed", id));
        }
        let id_bytes = id.as_bytes().to_vec();
        let mut stmt = self
            .conn
            .prepare("SELECT bytes FROM media_chunk WHERE media_id = ?1 ORDER BY chunk_index")
            .map_err(map_sql)?;
        let rows = stmt
            .query_map([&id_bytes], |r| r.get::<_, Vec<u8>>(0))
            .map_err(map_sql)?;
        let mut out = Vec::with_capacity(info.total_len as usize);
        for row in rows {
            out.extend_from_slice(&row.map_err(map_sql)?);
        }
        Ok(out)
    }

    /// Open a streaming reader over a packed media item. The reader owns its own
    /// SQLite connection (read-only) so it can live on a separate thread (e.g.
    /// the audio disk reader) independent of this archive handle.
    pub fn open_blob_reader(&self, db_path: &Path, id: Uuid) -> Result<BlobReader, String> {
        let info = self
            .media_info(id)?
            .ok_or_else(|| format!("Media {} not found", id))?;
        if info.storage != MediaStorage::Packed {
            return Err(format!("Media {} is referenced, not packed", id));
        }
        BlobReader::open(db_path, id, info.total_len, self.chunk_size)
    }

    /// Override the chunk size (testing / tuning). Affects subsequent writes.
    #[doc(hidden)]
    pub fn set_chunk_size(&mut self, chunk_size: u64) {
        assert!(chunk_size > 0);
        self.chunk_size = chunk_size;
    }
}

/// Streaming reader (`Read + Seek`) over a packed media item's chunk rows.
///
/// Owns a dedicated read-only SQLite connection so it is independent of the
/// writing [`BeamArchive`] handle and can be moved to another thread. Each
/// `read` opens a blob handle on the current chunk's row via `blob_open` (no
/// per-read query — chunk rowids are resolved once up front) and reads up to the
/// chunk boundary; callers that issue many tiny reads should wrap this in a
/// `BufReader`.
pub struct BlobReader {
    conn: Connection,
    chunk_rowids: Vec<i64>,
    chunk_size: u64,
    total_len: u64,
    pos: u64,
}

impl BlobReader {
    fn open(db_path: &Path, id: Uuid, total_len: u64, chunk_size: u64) -> Result<Self, String> {
        let conn = Connection::open_with_flags(
            db_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(map_sql)?;
        let id_bytes = id.as_bytes().to_vec();
        let mut stmt = conn
            .prepare("SELECT id FROM media_chunk WHERE media_id = ?1 ORDER BY chunk_index")
            .map_err(map_sql)?;
        let rows = stmt
            .query_map([&id_bytes], |r| r.get::<_, i64>(0))
            .map_err(map_sql)?;
        let mut chunk_rowids = Vec::new();
        for row in rows {
            chunk_rowids.push(row.map_err(map_sql)?);
        }
        drop(stmt);
        Ok(Self { conn, chunk_rowids, chunk_size, total_len, pos: 0 })
    }

    /// Total length of the media payload in bytes.
    pub fn len(&self) -> u64 {
        self.total_len
    }

    pub fn is_empty(&self) -> bool {
        self.total_len == 0
    }

    fn chunk_blob(&self, chunk_index: usize) -> io::Result<Blob<'_>> {
        let rowid = *self
            .chunk_rowids
            .get(chunk_index)
            .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "chunk index out of range"))?;
        self.conn
            .blob_open(DatabaseName::Main, "media_chunk", "bytes", rowid, true)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
    }
}

impl Read for BlobReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.pos >= self.total_len || buf.is_empty() {
            return Ok(0);
        }
        let chunk_index = (self.pos / self.chunk_size) as usize;
        let off_in_chunk = self.pos % self.chunk_size;

        // The chunk's length is derivable from total_len/chunk_size, so we don't
        // depend on Blob::len(): every chunk but the last is exactly chunk_size.
        let chunk_start = chunk_index as u64 * self.chunk_size;
        let chunk_len = (self.total_len - chunk_start).min(self.chunk_size);
        let avail_in_chunk = (chunk_len - off_in_chunk) as usize;
        let avail_total = (self.total_len - self.pos) as usize;
        let want = buf.len().min(avail_in_chunk).min(avail_total);

        // Scope the blob borrow (it borrows `self.conn`) so it ends before we
        // mutate `self.pos`.
        let n = {
            let mut blob = self.chunk_blob(chunk_index)?;
            blob.seek(SeekFrom::Start(off_in_chunk))?;
            blob.read(&mut buf[..want])?
        };
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for BlobReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new_pos = match pos {
            SeekFrom::Start(n) => n as i64,
            SeekFrom::End(n) => self.total_len as i64 + n,
            SeekFrom::Current(n) => self.pos as i64 + n,
        };
        if new_pos < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek before start of media",
            ));
        }
        // Allow seeking to/past end (reads then return 0), matching File semantics.
        self.pos = new_pos as u64;
        Ok(self.pos)
    }
}

/// A write transaction over a [`BeamArchive`]. All writes are buffered until
/// [`BeamTxn::commit`]; dropping without committing rolls back. Lets a save
/// touch only the rows that changed, in place, without rewriting unchanged media.
pub struct BeamTxn<'a> {
    tx: rusqlite::Transaction<'a>,
    chunk_size: u64,
}

impl BeamTxn<'_> {
    pub fn put_media_packed(
        &self,
        id: Uuid,
        kind: MediaKind,
        codec: &str,
        bytes: &[u8],
        meta: MediaMeta,
    ) -> Result<(), String> {
        write_media_packed(&self.tx, self.chunk_size, id, kind, codec, bytes, meta)
    }

    pub fn put_media_referenced(
        &self,
        id: Uuid,
        kind: MediaKind,
        codec: &str,
        ext_path: &str,
        meta: MediaMeta,
    ) -> Result<(), String> {
        write_media_referenced(&self.tx, id, kind, codec, ext_path, meta)
    }

    /// Like [`BeamTxn::put_media_packed`] but streams the bytes from a file on
    /// disk chunk-by-chunk, so an arbitrarily large file is never fully loaded
    /// into memory. `total_len` is taken from the bytes actually read.
    pub fn put_media_packed_from_path(
        &self,
        id: Uuid,
        kind: MediaKind,
        codec: &str,
        path: &Path,
        meta: MediaMeta,
    ) -> Result<(), String> {
        write_media_packed_from_path(&self.tx, self.chunk_size, id, kind, codec, path, meta)
    }

    pub fn set_project_json(&self, json: &str) -> Result<(), String> {
        set_project_json_conn(&self.tx, json)
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<(), String> {
        set_meta_conn(&self.tx, key, value)
    }

    /// Does a media row with this id already exist?
    pub fn media_exists(&self, id: Uuid) -> Result<bool, String> {
        let id_bytes = id.as_bytes().to_vec();
        let n: i64 = self
            .tx
            .query_row("SELECT COUNT(*) FROM media WHERE id = ?1", [&id_bytes], |r| r.get(0))
            .map_err(map_sql)?;
        Ok(n > 0)
    }

    /// Every media id currently in the archive.
    pub fn all_media_ids(&self) -> Result<Vec<Uuid>, String> {
        let mut stmt = self.tx.prepare("SELECT id FROM media").map_err(map_sql)?;
        let rows = stmt.query_map([], |r| r.get::<_, Vec<u8>>(0)).map_err(map_sql)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(uuid_from_bytes(&row.map_err(map_sql)?)?);
        }
        Ok(out)
    }

    /// Delete a media row (and its chunks).
    pub fn delete_media(&self, id: Uuid) -> Result<(), String> {
        let id_bytes = id.as_bytes().to_vec();
        self.tx
            .execute("DELETE FROM media_chunk WHERE media_id = ?1", [&id_bytes])
            .map_err(map_sql)?;
        self.tx
            .execute("DELETE FROM media WHERE id = ?1", [&id_bytes])
            .map_err(map_sql)?;
        Ok(())
    }

    /// Delete every media row whose id is not in `keep` (orphan cleanup).
    pub fn retain_media(&self, keep: &std::collections::HashSet<Uuid>) -> Result<usize, String> {
        let mut removed = 0;
        for id in self.all_media_ids()? {
            if !keep.contains(&id) {
                self.delete_media(id)?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    pub fn commit(self) -> Result<(), String> {
        self.tx.commit().map_err(map_sql)
    }
}

// -- shared write helpers (used by both BeamArchive and BeamTxn) --------------

fn write_media_packed(
    conn: &Connection,
    chunk_size: u64,
    id: Uuid,
    kind: MediaKind,
    codec: &str,
    bytes: &[u8],
    meta: MediaMeta,
) -> Result<(), String> {
    let id_bytes = id.as_bytes().to_vec();
    conn.execute("DELETE FROM media WHERE id = ?1", [&id_bytes]).map_err(map_sql)?;
    conn.execute("DELETE FROM media_chunk WHERE media_id = ?1", [&id_bytes])
        .map_err(map_sql)?;
    conn.execute(
        "INSERT INTO media
            (id, kind, codec, storage, ext_path, total_len, channels, sample_rate, width, height)
         VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            id_bytes,
            kind as i64,
            codec,
            MediaStorage::Packed as i64,
            bytes.len() as i64,
            meta.channels,
            meta.sample_rate,
            meta.width,
            meta.height,
        ],
    )
    .map_err(map_sql)?;
    for (chunk_index, chunk) in bytes.chunks(chunk_size as usize).enumerate() {
        conn.execute(
            "INSERT INTO media_chunk (media_id, chunk_index, bytes) VALUES (?1, ?2, ?3)",
            rusqlite::params![id_bytes, chunk_index as i64, chunk],
        )
        .map_err(map_sql)?;
    }
    Ok(())
}

fn write_media_packed_from_path(
    conn: &Connection,
    chunk_size: u64,
    id: Uuid,
    kind: MediaKind,
    codec: &str,
    path: &Path,
    meta: MediaMeta,
) -> Result<(), String> {
    let id_bytes = id.as_bytes().to_vec();
    conn.execute("DELETE FROM media WHERE id = ?1", [&id_bytes]).map_err(map_sql)?;
    conn.execute("DELETE FROM media_chunk WHERE media_id = ?1", [&id_bytes])
        .map_err(map_sql)?;

    let file = std::fs::File::open(path).map_err(|e| format!("Failed to open {:?}: {}", path, e))?;
    let mut reader = std::io::BufReader::new(file);
    let mut buf = vec![0u8; chunk_size as usize];
    let mut chunk_index: i64 = 0;
    let mut total_len: u64 = 0;

    loop {
        // Fill `buf` up to chunk_size, tolerating short reads.
        let mut filled = 0usize;
        while filled < buf.len() {
            let n = reader
                .read(&mut buf[filled..])
                .map_err(|e| format!("Failed to read {:?}: {}", path, e))?;
            if n == 0 {
                break;
            }
            filled += n;
        }
        if filled == 0 {
            break;
        }
        conn.execute(
            "INSERT INTO media_chunk (media_id, chunk_index, bytes) VALUES (?1, ?2, ?3)",
            rusqlite::params![id_bytes, chunk_index, &buf[..filled]],
        )
        .map_err(map_sql)?;
        chunk_index += 1;
        total_len += filled as u64;
        if filled < buf.len() {
            break; // reached EOF
        }
    }

    conn.execute(
        "INSERT INTO media
            (id, kind, codec, storage, ext_path, total_len, channels, sample_rate, width, height)
         VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            id_bytes,
            kind as i64,
            codec,
            MediaStorage::Packed as i64,
            total_len as i64,
            meta.channels,
            meta.sample_rate,
            meta.width,
            meta.height,
        ],
    )
    .map_err(map_sql)?;
    Ok(())
}

fn write_media_referenced(
    conn: &Connection,
    id: Uuid,
    kind: MediaKind,
    codec: &str,
    ext_path: &str,
    meta: MediaMeta,
) -> Result<(), String> {
    let id_bytes = id.as_bytes().to_vec();
    conn.execute("DELETE FROM media WHERE id = ?1", [&id_bytes]).map_err(map_sql)?;
    conn.execute("DELETE FROM media_chunk WHERE media_id = ?1", [&id_bytes])
        .map_err(map_sql)?;
    conn.execute(
        "INSERT INTO media
            (id, kind, codec, storage, ext_path, total_len, channels, sample_rate, width, height)
         VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            id_bytes,
            kind as i64,
            codec,
            MediaStorage::Referenced as i64,
            ext_path,
            meta.channels,
            meta.sample_rate,
            meta.width,
            meta.height,
        ],
    )
    .map_err(map_sql)?;
    Ok(())
}

fn set_project_json_conn(conn: &Connection, json: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO project_json (id, data) VALUES (0, ?1)
         ON CONFLICT(id) DO UPDATE SET data = excluded.data",
        [json],
    )
    .map_err(map_sql)?;
    Ok(())
}

fn set_meta_conn(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, value],
    )
    .map_err(map_sql)?;
    Ok(())
}

fn map_sql(e: rusqlite::Error) -> String {
    format!("SQLite error: {}", e)
}

fn uuid_from_bytes(bytes: &[u8]) -> Result<Uuid, String> {
    let arr: [u8; 16] = bytes
        .try_into()
        .map_err(|_| format!("Invalid uuid blob length {}", bytes.len()))?;
    Ok(Uuid::from_bytes(arr))
}

// Tests live in `tests/beam_archive.rs` (integration tests), so they compile the
// library in non-test mode and don't depend on the crate's other `#[cfg(test)]`
// modules.

//! Integration tests for the SQLite-backed `.beam` container.
//!
//! These are integration tests (not `#[cfg(test)]` unit tests) so they build the
//! library in normal mode and exercise only the public API — independent of any
//! pre-existing breakage in the crate's internal test modules.

use lightningbeam_core::beam_archive::{BeamArchive, MediaKind, MediaMeta, MediaStorage};
use std::io::{Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicU64, Ordering};
use uuid::Uuid;

fn temp_db_path(tag: &str) -> std::path::PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let mut p = std::env::temp_dir();
    p.push(format!("beam_archive_it_{}_{}_{}.beam", std::process::id(), tag, n));
    let _ = std::fs::remove_file(&p);
    p
}

#[test]
fn project_json_roundtrip() {
    let path = temp_db_path("json");
    let archive = BeamArchive::create(&path).unwrap();
    archive.set_project_json("{\"hello\":\"world\"}").unwrap();
    assert_eq!(archive.get_project_json().unwrap(), "{\"hello\":\"world\"}");
    drop(archive);
    let archive = BeamArchive::open(&path).unwrap();
    assert_eq!(archive.get_project_json().unwrap(), "{\"hello\":\"world\"}");
    assert!(BeamArchive::is_sqlite(&path));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn packed_media_roundtrip_full() {
    let path = temp_db_path("full");
    let mut archive = BeamArchive::create(&path).unwrap();
    let id = Uuid::from_u128(0x1234);
    archive.set_chunk_size(1000);
    let data: Vec<u8> = (0..3500u32).map(|i| (i % 251) as u8).collect();
    archive
        .put_media_packed(
            id,
            MediaKind::Audio,
            "flac",
            &data,
            MediaMeta { channels: Some(2), sample_rate: Some(44100), ..Default::default() },
        )
        .unwrap();

    let info = archive.media_info(id).unwrap().unwrap();
    assert_eq!(info.kind, MediaKind::Audio);
    assert_eq!(info.codec, "flac");
    assert_eq!(info.storage, MediaStorage::Packed);
    assert_eq!(info.total_len, 3500);
    assert_eq!(info.channels, Some(2));
    assert_eq!(info.sample_rate, Some(44100));

    assert_eq!(archive.read_media_full(id).unwrap(), data);
    assert_eq!(archive.media_ids_of_kind(MediaKind::Audio).unwrap(), vec![id]);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn blob_reader_streams_and_seeks() {
    let path = temp_db_path("stream");
    let mut archive = BeamArchive::create(&path).unwrap();
    archive.set_chunk_size(1000);
    let id = Uuid::from_u128(0xBEEF);
    let data: Vec<u8> = (0..3500u32).map(|i| (i % 251) as u8).collect();
    archive
        .put_media_packed(id, MediaKind::Audio, "mp3", &data, MediaMeta::default())
        .unwrap();

    let mut reader = archive.open_blob_reader(&path, id).unwrap();
    assert_eq!(reader.len(), 3500);

    // Sequential read in odd-sized buffers crosses chunk boundaries.
    let mut got = Vec::new();
    let mut buf = [0u8; 333];
    loop {
        let n = reader.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        got.extend_from_slice(&buf[..n]);
    }
    assert_eq!(got, data);

    // Seek to a mid-chunk position and read across a boundary.
    reader.seek(SeekFrom::Start(990)).unwrap();
    let mut window = [0u8; 20];
    let mut filled = 0;
    while filled < window.len() {
        let n = reader.read(&mut window[filled..]).unwrap();
        assert!(n > 0);
        filled += n;
    }
    assert_eq!(&window[..], &data[990..1010]);

    // Seek from end and read the tail.
    reader.seek(SeekFrom::End(-10)).unwrap();
    let mut tail = Vec::new();
    reader.read_to_end(&mut tail).unwrap();
    assert_eq!(tail, &data[3490..]);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn referenced_media_records_path() {
    let path = temp_db_path("ref");
    let mut archive = BeamArchive::create(&path).unwrap();
    let id = Uuid::from_u128(0xCAFE);
    archive
        .put_media_referenced(
            id,
            MediaKind::Video,
            "mp4",
            "/mnt/share/big.mp4",
            MediaMeta { width: Some(3840), height: Some(2160), ..Default::default() },
        )
        .unwrap();
    let info = archive.media_info(id).unwrap().unwrap();
    assert_eq!(info.storage, MediaStorage::Referenced);
    assert_eq!(info.ext_path.as_deref(), Some("/mnt/share/big.mp4"));
    assert_eq!(info.width, Some(3840));
    // Streaming a referenced item is an error (caller opens the path directly).
    assert!(archive.open_blob_reader(&path, id).is_err());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn transaction_groups_writes_and_orphan_cleanup() {
    let path = temp_db_path("txn");
    let keep = Uuid::from_u128(1);
    let orphan = Uuid::from_u128(2);

    // First save: two media items committed in one transaction.
    {
        let mut archive = BeamArchive::create(&path).unwrap();
        let txn = archive.transaction().unwrap();
        txn.put_media_packed(keep, MediaKind::Audio, "flac", &vec![9u8; 10], MediaMeta::default())
            .unwrap();
        txn.put_media_packed(orphan, MediaKind::Audio, "mp3", &vec![8u8; 10], MediaMeta::default())
            .unwrap();
        txn.set_project_json("{}").unwrap();
        txn.commit().unwrap();
    }
    {
        let archive = BeamArchive::open(&path).unwrap();
        assert!(archive.media_info(keep).unwrap().is_some());
        assert!(archive.media_info(orphan).unwrap().is_some());
    }

    // Second save (in place): keep only `keep`; `orphan` should be retained-out.
    {
        let mut archive = BeamArchive::open(&path).unwrap();
        let txn = archive.transaction().unwrap();
        // `keep` already present → in-place save leaves it untouched.
        assert!(txn.media_exists(keep).unwrap());
        let mut live = std::collections::HashSet::new();
        live.insert(keep);
        let removed = txn.retain_media(&live).unwrap();
        assert_eq!(removed, 1);
        txn.commit().unwrap();
    }
    {
        let archive = BeamArchive::open(&path).unwrap();
        assert!(archive.media_info(keep).unwrap().is_some());
        assert!(archive.media_info(orphan).unwrap().is_none());
        // `keep`'s bytes survived untouched.
        assert_eq!(archive.read_media_full(keep).unwrap(), vec![9u8; 10]);
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn rolled_back_transaction_writes_nothing() {
    let path = temp_db_path("rollback");
    let id = Uuid::from_u128(42);
    let mut archive = BeamArchive::create(&path).unwrap();
    {
        let txn = archive.transaction().unwrap();
        txn.put_media_packed(id, MediaKind::Audio, "flac", &vec![1u8; 5], MediaMeta::default())
            .unwrap();
        // Drop without commit → rollback.
    }
    assert!(archive.media_info(id).unwrap().is_none());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn overwrite_media_replaces_chunks() {
    let path = temp_db_path("overwrite");
    let mut archive = BeamArchive::create(&path).unwrap();
    archive.set_chunk_size(100);
    let id = Uuid::from_u128(7);
    archive
        .put_media_packed(id, MediaKind::Raster, "png", &vec![1u8; 250], MediaMeta::default())
        .unwrap();
    // Overwrite with shorter data — stale chunks must be gone.
    archive
        .put_media_packed(id, MediaKind::Raster, "png", &vec![2u8; 50], MediaMeta::default())
        .unwrap();
    assert_eq!(archive.read_media_full(id).unwrap(), vec![2u8; 50]);
    let _ = std::fs::remove_file(&path);
}

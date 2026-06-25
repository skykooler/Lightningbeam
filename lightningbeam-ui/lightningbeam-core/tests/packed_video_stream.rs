//! Integration test for the packed-video streaming path:
//! SQLite `MediaKind::Video` blob -> `BlobReader` -> `ffmpeg-blob-io` AVIO shim ->
//! ffmpeg `Input`. This is exactly what `video.rs`'s `VideoSource::Packed` does to
//! decode frames (and what `daw-backend` does for the embedded audio), minus the
//! thin wrapper. We use a hand-built WAV as the packed container — FFmpeg must
//! demux it through our blob reader, proving the production byte path end to end.
//!
//! (Decoding real video frames is left to user runtime verification; here we prove
//! the container streams from the blob and exposes its streams.)

use ffmpeg_blob_io::BlobInput;
use lightningbeam_core::beam_archive::{BeamArchive, MediaKind, MediaMeta, MediaStorage};
use std::sync::atomic::{AtomicU64, Ordering};
use uuid::Uuid;

fn temp_db_path(tag: &str) -> std::path::PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let mut p = std::env::temp_dir();
    p.push(format!("packed_video_it_{}_{}_{}.beam", std::process::id(), tag, n));
    let _ = std::fs::remove_file(&p);
    p
}

/// Minimal 16-bit PCM WAV (a real, demuxable container).
fn make_wav(sample_rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
    let bits: u16 = 16;
    let block_align: u16 = channels * (bits / 8);
    let byte_rate: u32 = sample_rate * block_align as u32;
    let data_len: u32 = (samples.len() * 2) as u32;
    let mut v = Vec::new();
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data_len).to_le_bytes());
    v.extend_from_slice(b"WAVE");
    v.extend_from_slice(b"fmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&channels.to_le_bytes());
    v.extend_from_slice(&sample_rate.to_le_bytes());
    v.extend_from_slice(&byte_rate.to_le_bytes());
    v.extend_from_slice(&block_align.to_le_bytes());
    v.extend_from_slice(&bits.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        v.extend_from_slice(&s.to_le_bytes());
    }
    v
}

#[test]
fn packed_media_streams_through_avio_to_ffmpeg() {
    let path = temp_db_path("stream");
    let id = Uuid::new_v4();
    let samples: Vec<i16> = (0..4000).map(|i| ((i % 200) as i16 - 100) * 100).collect();
    let container = make_wav(8000, 1, &samples);

    // Pack as a Video media row (the save path does this for real videos).
    let mut archive = BeamArchive::create(&path).unwrap();
    archive
        .put_media_packed(id, MediaKind::Video, "wav", &container, MediaMeta::default())
        .unwrap();
    let info = archive.media_info(id).unwrap().unwrap();
    assert_eq!(info.kind, MediaKind::Video);
    assert_eq!(info.storage, MediaStorage::Packed);
    drop(archive);

    // Reproduce VideoSource::Packed::open(): fresh read-only archive + blob reader.
    let archive = BeamArchive::open(&path).unwrap();
    let hint = archive.media_info(id).unwrap().map(|i| i.codec);
    let reader = archive.open_blob_reader(&path, id).unwrap();

    let input = BlobInput::open(Box::new(reader), hint.as_deref())
        .expect("open the packed container by streaming from the SQLite blob");
    assert!(
        input.streams().count() >= 1,
        "demuxer found streams via the blob-backed AVIO shim"
    );

    let _ = std::fs::remove_file(&path);
}

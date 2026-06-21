//! Integration test for `CompressedReader::open_source` — decoding a streaming
//! audio source from an in-memory byte stream (the packed-in-container path)
//! rather than a filesystem path. Proves the `MediaByteSource` adapter feeds
//! Symphonia correctly (probe + decode + seekable byte length).

use std::io::{Cursor, Read, Seek, SeekFrom};

use daw_backend::audio::disk_reader::{CompressedReader, MediaByteSource};

/// A `MediaByteSource` over an in-memory buffer (stands in for core's BlobReader).
struct VecSource(Cursor<Vec<u8>>);

impl Read for VecSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}
impl Seek for VecSource {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.0.seek(pos)
    }
}
impl MediaByteSource for VecSource {
    fn byte_len(&self) -> u64 {
        self.0.get_ref().len() as u64
    }
}

/// Build a minimal PCM16 mono WAV byte buffer holding `frames` samples of a ramp.
fn make_wav(sample_rate: u32, frames: u32) -> Vec<u8> {
    let channels: u16 = 1;
    let bits: u16 = 16;
    let block_align: u16 = channels * bits / 8;
    let byte_rate: u32 = sample_rate * block_align as u32;
    let data_len: u32 = frames * block_align as u32;

    let mut v = Vec::new();
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data_len).to_le_bytes());
    v.extend_from_slice(b"WAVE");
    v.extend_from_slice(b"fmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes()); // PCM
    v.extend_from_slice(&channels.to_le_bytes());
    v.extend_from_slice(&sample_rate.to_le_bytes());
    v.extend_from_slice(&byte_rate.to_le_bytes());
    v.extend_from_slice(&block_align.to_le_bytes());
    v.extend_from_slice(&bits.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..frames {
        // A ramp from -16000..16000 so values are recognizable.
        let s = (((i % 1000) as i32 - 500) * 32) as i16;
        v.extend_from_slice(&s.to_le_bytes());
    }
    v
}

#[test]
fn open_source_decodes_in_memory_wav() {
    let sample_rate = 8000;
    let frames = 4096;
    let bytes = make_wav(sample_rate, frames);

    let src = Box::new(VecSource(Cursor::new(bytes)));
    let mut reader = CompressedReader::open_source(src, Some("wav"))
        .expect("open_source should probe the in-memory WAV");

    assert_eq!(reader.sample_rate(), sample_rate);
    assert_eq!(reader.channels(), 1);

    // Decode the whole stream and count emitted frames.
    let mut buf = Vec::new();
    let mut decoded = 0usize;
    loop {
        let n = reader.decode_next(&mut buf).expect("decode_next");
        if n == 0 {
            break;
        }
        decoded += n;
    }
    // Should recover (approximately) all frames — codec frame counts can round.
    assert!(
        (decoded as i64 - frames as i64).abs() < 64,
        "decoded {} vs expected {}",
        decoded,
        frames
    );
}

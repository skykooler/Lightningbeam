//! Exercises the custom AVIO shim end to end against a hand-built WAV, fed through
//! an in-memory `Cursor` (a `Read + Seek` source). FFmpeg can only demux it by
//! calling our `read_cb`/`seek_cb`, so a successful open proves the shim works.

use ffmpeg_blob_io::BlobInput;
use std::io::Cursor;

/// Build a minimal 16-bit PCM WAV in memory.
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
    v.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    v.extend_from_slice(&1u16.to_le_bytes()); // PCM
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

fn sample_wav() -> Vec<u8> {
    let samples: Vec<i16> = (0..1600).map(|i| ((i % 100) as i16 - 50) * 200).collect();
    make_wav(8000, 1, &samples)
}

#[test]
fn opens_from_reader_and_reports_stream() {
    let input = BlobInput::open(Box::new(Cursor::new(sample_wav())), Some("wav"))
        .expect("open WAV through the blob shim");

    let stream = input
        .streams()
        .best(ffmpeg_next::media::Type::Audio)
        .expect("an audio stream");
    let ctx = ffmpeg_next::codec::context::Context::from_parameters(stream.parameters())
        .expect("codec context");
    let decoder = ctx.decoder().audio().expect("audio decoder");

    assert_eq!(decoder.rate(), 8000, "sample rate read via AVIO");
    assert_eq!(decoder.channels(), 1, "channel count read via AVIO");
}

#[test]
fn reads_packets_through_callbacks() {
    let mut input = BlobInput::open(Box::new(Cursor::new(sample_wav())), Some("wav"))
        .expect("open WAV");
    let mut packets = 0usize;
    let mut bytes = 0usize;
    for (_stream, packet) in input.packets() {
        packets += 1;
        bytes += packet.size();
    }
    assert!(packets > 0, "demuxer produced packets via read_cb");
    assert!(bytes > 0, "packets carried PCM payload");
}

#[test]
fn seek_then_read() {
    let mut input = BlobInput::open(Box::new(Cursor::new(sample_wav())), Some("wav"))
        .expect("open WAV");
    // Seek back to the start (exercises seek_cb SEEK_SET + AVSEEK_SIZE).
    input.seek(0, ..).expect("seek to start");
    let got_packet = input.packets().next().is_some();
    assert!(got_packet, "can still read after seek");
}

#[test]
fn open_drop_loop_is_clean() {
    // Repeated open+drop surfaces double-free / leak in the Drop teardown
    // (run under `RUSTFLAGS=-Zsanitizer=address` on nightly for full coverage).
    for _ in 0..200 {
        let input = BlobInput::open(Box::new(Cursor::new(sample_wav())), Some("wav"))
            .expect("open WAV");
        assert!(input.streams().count() >= 1);
        drop(input);
    }
}

#[test]
fn bad_format_hint_errors_without_leak() {
    // Garbage bytes with a real hint: open should fail cleanly (error path frees
    // buffer + avio + reader), not panic or leak.
    let garbage = vec![0u8; 4096];
    let res = BlobInput::open(Box::new(Cursor::new(garbage)), Some("wav"));
    assert!(res.is_err(), "non-WAV bytes should fail to open as WAV");
}

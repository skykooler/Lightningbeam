/// Incremental WAV file writer for streaming audio to disk
use std::fs::File;
use std::io::{self, Seek, SeekFrom, Write};
use std::path::Path;

/// WAV file writer that supports incremental writing
pub struct WavWriter {
    file: File,
    sample_rate: u32,
    channels: u32,
    frames_written: usize,
}

impl WavWriter {
    /// Create a new WAV file and write initial header
    /// The header is written with placeholder sizes that will be updated on finalization
    pub fn create(path: impl AsRef<Path>, sample_rate: u32, channels: u32) -> io::Result<Self> {
        let mut file = File::create(path)?;

        // Write initial WAV header with placeholder sizes
        write_wav_header(&mut file, sample_rate, channels, 0)?;

        Ok(Self {
            file,
            sample_rate,
            channels,
            frames_written: 0,
        })
    }

    /// Append audio samples to the file
    /// Expects interleaved f32 samples in range [-1.0, 1.0]
    pub fn write_samples(&mut self, samples: &[f32]) -> io::Result<()> {
        // Convert f32 samples to 16-bit PCM
        let pcm_data: Vec<u8> = samples
            .iter()
            .flat_map(|&sample| {
                let clamped = sample.clamp(-1.0, 1.0);
                let pcm_value = (clamped * 32767.0) as i16;
                pcm_value.to_le_bytes()
            })
            .collect();

        self.file.write_all(&pcm_data)?;
        self.frames_written += samples.len() / self.channels as usize;

        Ok(())
    }

    /// Get the current number of frames written
    pub fn frames_written(&self) -> usize {
        self.frames_written
    }

    /// Get the current duration in seconds
    pub fn duration(&self) -> f64 {
        self.frames_written as f64 / self.sample_rate as f64
    }

    /// Finalize the WAV file by updating the header with correct sizes
    pub fn finalize(mut self) -> io::Result<()> {
        // Flush any remaining data
        self.file.flush()?;

        // Calculate total data size
        let data_size = self.frames_written * self.channels as usize * 2; // 2 bytes per sample (16-bit)
        let file_size = 36 + data_size; // 36 = size of header before data

        // Seek to RIFF chunk size (offset 4)
        self.file.seek(SeekFrom::Start(4))?;
        self.file.write_all(&((file_size - 8) as u32).to_le_bytes())?;

        // Seek to data chunk size (offset 40)
        self.file.seek(SeekFrom::Start(40))?;
        self.file.write_all(&(data_size as u32).to_le_bytes())?;

        self.file.flush()?;

        Ok(())
    }
}

/// Write WAV header with specified parameters
fn write_wav_header(file: &mut File, sample_rate: u32, channels: u32, frames: usize) -> io::Result<()> {
    let bytes_per_sample = 2u16; // 16-bit PCM
    let data_size = (frames * channels as usize * bytes_per_sample as usize) as u32;
    let file_size = 36 + data_size;

    // RIFF header
    file.write_all(b"RIFF")?;
    file.write_all(&(file_size - 8).to_le_bytes())?;
    file.write_all(b"WAVE")?;

    // fmt chunk
    file.write_all(b"fmt ")?;
    file.write_all(&16u32.to_le_bytes())?; // fmt chunk size
    file.write_all(&1u16.to_le_bytes())?; // PCM format
    file.write_all(&(channels as u16).to_le_bytes())?;
    file.write_all(&sample_rate.to_le_bytes())?;

    let byte_rate = sample_rate * channels * bytes_per_sample as u32;
    file.write_all(&byte_rate.to_le_bytes())?;

    let block_align = channels as u16 * bytes_per_sample;
    file.write_all(&block_align.to_le_bytes())?;
    file.write_all(&(bytes_per_sample * 8).to_le_bytes())?; // bits per sample

    // data chunk header
    file.write_all(b"data")?;
    file.write_all(&data_size.to_le_bytes())?;

    Ok(())
}

/// CPU-side FFT computation for spectrogram visualization.
///
/// Uses rayon to parallelize FFT across time slices on all CPU cores.
/// Produces a 2D magnitude grid (time bins x frequency bins) for GPU texture upload.

use rayon::prelude::*;
use std::f32::consts::PI;

/// Pre-computed spectrogram data ready for GPU upload
pub struct SpectrogramData {
    /// Flattened 2D array of normalized magnitudes [time_bins * freq_bins], row-major
    /// Each value is 0.0 (silence) to 1.0 (peak), log-scale normalized
    pub magnitudes: Vec<f32>,
    pub time_bins: usize,
    pub freq_bins: usize,
    pub sample_rate: u32,
    pub hop_size: usize,
    pub fft_size: usize,
    pub duration: f64,
}

/// Compute a spectrogram from raw audio samples using parallel FFT.
///
/// Each time slice is processed independently via rayon, making this
/// scale well across all CPU cores.
pub fn compute_spectrogram(
    samples: &[f32],
    sample_rate: u32,
    channels: u32,
    fft_size: usize,
    hop_size: usize,
) -> SpectrogramData {
    // Mix to mono
    let mono: Vec<f32> = if channels >= 2 {
        samples
            .chunks(channels as usize)
            .map(|frame| frame.iter().sum::<f32>() / channels as f32)
            .collect()
    } else {
        samples.to_vec()
    };

    let freq_bins = fft_size / 2 + 1;
    let duration = mono.len() as f64 / sample_rate as f64;

    if mono.len() < fft_size {
        return SpectrogramData {
            magnitudes: Vec::new(),
            time_bins: 0,
            freq_bins,
            sample_rate,
            hop_size,
            fft_size,
            duration,
        };
    }

    let time_bins = (mono.len().saturating_sub(fft_size)) / hop_size + 1;

    // Precompute Hann window
    let window: Vec<f32> = (0..fft_size)
        .map(|i| 0.5 * (1.0 - (2.0 * PI * i as f32 / fft_size as f32).cos()))
        .collect();

    // Precompute twiddle factors for Cooley-Tukey radix-2 FFT
    let twiddles: Vec<(f32, f32)> = (0..fft_size / 2)
        .map(|k| {
            let angle = -2.0 * PI * k as f32 / fft_size as f32;
            (angle.cos(), angle.sin())
        })
        .collect();

    // Bit-reversal permutation table
    let bits = (fft_size as f32).log2() as u32;
    let bit_rev: Vec<usize> = (0..fft_size)
        .map(|i| (i as u32).reverse_bits().wrapping_shr(32 - bits) as usize)
        .collect();

    // Process all time slices in parallel
    let magnitudes: Vec<f32> = (0..time_bins)
        .into_par_iter()
        .flat_map(|t| {
            let offset = t * hop_size;
            let mut re = vec![0.0f32; fft_size];
            let mut im = vec![0.0f32; fft_size];

            // Load windowed samples in bit-reversed order
            for i in 0..fft_size {
                let sample = if offset + i < mono.len() {
                    mono[offset + i]
                } else {
                    0.0
                };
                re[bit_rev[i]] = sample * window[i];
            }

            // Cooley-Tukey radix-2 DIT FFT
            let mut half_size = 1;
            while half_size < fft_size {
                let step = half_size * 2;
                let twiddle_step = fft_size / step;

                for k in (0..fft_size).step_by(step) {
                    for j in 0..half_size {
                        let tw_idx = j * twiddle_step;
                        let (tw_re, tw_im) = twiddles[tw_idx];

                        let a = k + j;
                        let b = a + half_size;

                        let t_re = tw_re * re[b] - tw_im * im[b];
                        let t_im = tw_re * im[b] + tw_im * re[b];

                        re[b] = re[a] - t_re;
                        im[b] = im[a] - t_im;
                        re[a] += t_re;
                        im[a] += t_im;
                    }
                }
                half_size = step;
            }

            // Extract magnitudes for positive frequencies
            let mut mags = Vec::with_capacity(freq_bins);
            for f in 0..freq_bins {
                let mag = (re[f] * re[f] + im[f] * im[f]).sqrt();
                // dB normalization: -80dB floor to 0dB ceiling → 0.0 to 1.0
                let db = 20.0 * (mag + 1e-10).log10();
                mags.push(((db + 80.0) / 80.0).clamp(0.0, 1.0));
            }
            mags
        })
        .collect();

    SpectrogramData {
        magnitudes,
        time_bins,
        freq_bins,
        sample_rate,
        hop_size,
        fft_size,
        duration,
    }
}

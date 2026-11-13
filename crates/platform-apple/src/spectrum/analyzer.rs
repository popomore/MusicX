//! Windowed stereo FFT. Runs on the visualization worker, never on the audio thread.
use musicx_core::protocol::spectrum::{SPECTRUM_BANDS, SPECTRUM_FLOOR_DB, band_edge};
use rustfft::{Fft, FftPlanner, num_complex::Complex};
use std::{f32::consts::TAU, sync::Arc};

pub(super) const FFT_SIZE: usize = 4096;
pub(super) const HOP_SIZE: usize = FFT_SIZE / 2;

pub(super) struct Analyzer {
    fft: Arc<dyn Fft<f32>>,
    window: [f32; FFT_SIZE],
    scale: f32,
    buffer: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    power: [f32; FFT_SIZE / 2 + 1],
}

impl Analyzer {
    pub(super) fn new() -> Self {
        let fft = FftPlanner::new().plan_fft_forward(FFT_SIZE);
        let window = std::array::from_fn(|i| 0.5 - 0.5 * (TAU * i as f32 / FFT_SIZE as f32).cos());
        Self {
            scratch: vec![Complex::default(); fft.get_inplace_scratch_len()],
            fft,
            scale: 2.0 / window.iter().sum::<f32>(),
            window,
            buffer: vec![Complex::default(); FFT_SIZE],
            power: [0.0; FFT_SIZE / 2 + 1],
        }
    }

    pub(super) fn analyze(
        &mut self,
        samples: &[[f32; 2]; FFT_SIZE],
        sample_rate: f32,
    ) -> [f32; SPECTRUM_BANDS] {
        let mut bands = [SPECTRUM_FLOOR_DB; SPECTRUM_BANDS];
        if !sample_rate.is_finite() || sample_rate <= 0.0 {
            return bands;
        }
        self.power.fill(0.0);
        for channel in 0..2 {
            // Remove DC and sanitize non-finite input before the transform.
            let mean = samples.iter().map(|s| finite(s[channel])).sum::<f32>() / FFT_SIZE as f32;
            for ((bin, sample), window) in self.buffer.iter_mut().zip(samples).zip(self.window) {
                *bin = Complex::new((finite(sample[channel]) - mean) * window, 0.0);
            }
            self.fft
                .process_with_scratch(&mut self.buffer, &mut self.scratch);
            for (power, bin) in self.power.iter_mut().zip(&self.buffer) {
                *power += bin.norm_sqr() * self.scale.powi(2) * 0.5;
            }
        }
        let hz_per_bin = sample_rate / FFT_SIZE as f32;
        for (index, level) in bands.iter_mut().enumerate() {
            let low = band_edge(index);
            let high = band_edge(index + 1).min(sample_rate / 2.0);
            if low >= high {
                continue;
            }
            let start = ((low / hz_per_bin).ceil() as usize).max(1);
            let end = ((high / hz_per_bin).ceil() as usize).min(self.power.len());
            let power = if start < end {
                self.power[start..end].iter().copied().fold(0.0, f32::max)
            } else {
                // Narrow bass bands may lie between FFT bins. Interpolate
                // their power instead of inventing an empty notch.
                let center = (low * high).sqrt() / hz_per_bin;
                let left = (center.floor() as usize).clamp(1, self.power.len() - 1);
                let right = (left + 1).min(self.power.len() - 1);
                self.power[left] + (self.power[right] - self.power[left]) * center.fract()
            };
            *level = (10.0 * power.max(1e-8).log10()).clamp(SPECTRUM_FLOOR_DB, 0.0);
        }
        bands
    }
}

fn finite(sample: f32) -> f32 {
    if sample.is_finite() { sample } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(frequency: f32, rate: f32, amplitude: f32, right: f32) -> [[f32; 2]; FFT_SIZE] {
        std::array::from_fn(|i| {
            let sample = amplitude * (TAU * frequency * i as f32 / rate).sin();
            [sample, sample * right]
        })
    }

    #[test]
    fn tones_land_in_their_logarithmic_bands_at_multiple_sample_rates() {
        let mut analyzer = Analyzer::new();
        for rate in [44_100.0, 48_000.0, 96_000.0] {
            for frequency in [100.0, 1_000.0, 8_000.0] {
                let bins = analyzer.analyze(&tone(frequency, rate, 0.5, 1.0), rate);
                let peak = bins
                    .iter()
                    .enumerate()
                    .max_by(|a, b| a.1.total_cmp(b.1))
                    .unwrap()
                    .0;
                assert!(
                    frequency >= band_edge(peak.saturating_sub(1))
                        && frequency < band_edge(peak + 2),
                    "{frequency} Hz at {rate} Hz landed in band {peak}"
                );
                assert!(bins[peak] > -10.0 && bins[peak] < -5.0);
            }
        }
    }

    #[test]
    fn stereo_power_preserves_opposite_phase_and_single_channel_audio() {
        let mut analyzer = Analyzer::new();
        let same = analyzer.analyze(&tone(1_000.0, 48_000.0, 0.5, 1.0), 48_000.0);
        let opposite = analyzer.analyze(&tone(1_000.0, 48_000.0, 0.5, -1.0), 48_000.0);
        assert_eq!(same, opposite);
        let left_only = analyzer.analyze(&tone(1_000.0, 48_000.0, 0.5, 0.0), 48_000.0);
        let peak = same.iter().copied().fold(SPECTRUM_FLOOR_DB, f32::max);
        assert!(
            (left_only.iter().copied().fold(SPECTRUM_FLOOR_DB, f32::max) - peak + 3.0103).abs()
                < 0.01
        );
    }

    #[test]
    fn silence_dc_invalid_samples_and_nyquist_are_bounded() {
        let mut analyzer = Analyzer::new();
        let floor = [SPECTRUM_FLOOR_DB; SPECTRUM_BANDS];
        assert_eq!(analyzer.analyze(&[[0.0; 2]; FFT_SIZE], 48_000.0), floor);
        assert_eq!(analyzer.analyze(&[[0.4; 2]; FFT_SIZE], 48_000.0), floor);
        assert_eq!(
            analyzer.analyze(&[[f32::NAN, f32::INFINITY]; FFT_SIZE], 48_000.0),
            floor
        );
        assert_eq!(
            analyzer.analyze(&tone(1_000.0, 48_000.0, 1.0, 1.0), f32::NAN),
            floor
        );
        let low_rate = analyzer.analyze(&tone(1_000.0, 16_000.0, 1.0, 1.0), 16_000.0);
        for (i, level) in low_rate.iter().enumerate() {
            assert!(level.is_finite() && *level >= SPECTRUM_FLOOR_DB && *level <= 0.0);
            if band_edge(i) >= 8_000.0 {
                assert_eq!(*level, SPECTRUM_FLOOR_DB);
            }
        }
    }
}

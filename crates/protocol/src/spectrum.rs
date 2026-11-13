//! Small, disposable visualization frames, independent of persistent player state.
use serde::{Deserialize, Serialize};

pub const SPECTRUM_BANDS: usize = 32;
pub const SPECTRUM_MIN_HZ: f32 = 20.0;
pub const SPECTRUM_MAX_HZ: f32 = 20_000.0;
pub const SPECTRUM_FLOOR_DB: f32 = -80.0;

#[derive(Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpectrumState {
    #[default]
    Idle,
    Waiting,
    Live,
    Unavailable,
}

#[derive(Clone, PartialEq, Deserialize, Serialize)]
pub struct Spectrum {
    pub state: SpectrumState,
    pub track_key: String,
    pub sample_rate: f32,
    /// Peak FFT amplitude per logarithmic band, in dBFS, before player volume.
    pub bands: [f32; SPECTRUM_BANDS],
}

impl Default for Spectrum {
    fn default() -> Self {
        Self {
            state: SpectrumState::Idle,
            track_key: String::new(),
            sample_rate: 0.0,
            bands: [SPECTRUM_FLOOR_DB; SPECTRUM_BANDS],
        }
    }
}

pub fn band_edge(index: usize) -> f32 {
    SPECTRUM_MIN_HZ
        * (SPECTRUM_MAX_HZ / SPECTRUM_MIN_HZ)
            .powf(index.min(SPECTRUM_BANDS) as f32 / SPECTRUM_BANDS as f32)
}

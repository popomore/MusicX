use anyhow::Result;
use musicx_plugin_api::Media;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AudioState {
    Paused,
    Playing,
    Waiting,
}

pub struct AudioSnapshot {
    pub position: f64,
    pub duration: Option<f64>,
    pub state: AudioState,
    pub volume: f32,
    pub error: Option<String>,
}

/// Native audio operations used by the playback state machine. The platform
/// schedules them on the thread that owns its audio objects.
pub trait AudioOutput: Sized {
    fn set_normalization(&self, _enabled: bool) {}
    fn load(output: &mut Option<Self>, media: &Media, play: bool, volume: f32) -> Result<()>;
    fn snapshot(&self) -> AudioSnapshot;
    fn prepare_resume(&self) -> Result<()>;
    fn play(&self);
    fn pause(&self);
    fn set_volume(&self, volume: f32);
    fn seek(&self, seconds: f64, complete: impl Fn(bool) + Send + 'static);
}

/// AVPlayer's scalar cannot amplify above unity; attenuation and peak
/// protection still apply without changing the listener's volume preference.
pub fn normalization_scale(gain_db: Option<f32>, peak: Option<f32>) -> f32 {
    if gain_db.is_none() && peak.is_none() {
        return 1.0;
    }
    let mut scale = 10.0_f32.powf((gain_db.filter(|v| v.is_finite()).unwrap_or(0.0) + 4.0) / 20.0);
    if let Some(peak) = peak.filter(|v| v.is_finite() && *v > 0.0) {
        scale = scale.min(1.0 / peak);
    }
    scale.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalization_attenuates_and_limits_peaks() {
        assert_eq!(normalization_scale(None, None), 1.0);
        assert!((normalization_scale(Some(-10.0), None) - 0.5011872).abs() < 0.00001);
        assert_eq!(normalization_scale(Some(10.0), Some(2.0)), 0.5);
        assert_eq!(normalization_scale(Some(10.0), None), 1.0);
    }
}

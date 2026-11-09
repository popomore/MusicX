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
    fn load(output: &mut Option<Self>, media: &Media, play: bool, volume: f32) -> Result<()>;
    fn snapshot(&self) -> AudioSnapshot;
    fn prepare_resume(&self) -> Result<()>;
    fn play(&self);
    fn pause(&self);
    fn set_volume(&self, volume: f32);
    fn seek(&self, seconds: f64, complete: impl Fn(bool) + Send + 'static);
}

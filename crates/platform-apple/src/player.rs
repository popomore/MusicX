use anyhow::{Result, anyhow};
use musicx_core::protocol::tr;
use musicx_core::{
    audio::{AudioOutput, AudioSnapshot, AudioState},
    player::{Message, Player, Reply},
};
use musicx_plugin_api::{DataSource, Media};
use objc2::{
    MainThreadMarker,
    rc::{Retained, autoreleasepool},
    runtime::AnyObject,
};
use objc2_av_foundation::{
    AVPlayer, AVPlayerItem, AVPlayerTimeControlStatus, AVURLAsset, AVURLAssetHTTPUserAgentKey,
};
use objc2_core_media::CMTime;
use objc2_foundation::{NSDictionary, NSString, NSURL};
use std::{
    cell::{Cell, RefCell},
    sync::{Arc, OnceLock},
};

pub use musicx_core::player::{Action, State, Status};

static SOURCE: OnceLock<&'static dyn DataSource> = OnceLock::new();
thread_local! {
    static PLAYER: RefCell<Player<AppleAudio>> =
        RefCell::new(Player::new(dispatch, *SOURCE.get().expect("configured data source")));
}

pub fn configure(source: &'static dyn DataSource) -> Result<()> {
    SOURCE
        .set(source)
        .map_err(|_| anyhow!(tr!("Playback core already configured")))
}
#[cfg(target_os = "macos")]
pub(crate) fn artwork(url: &str) -> Option<Vec<u8>> {
    SOURCE.get()?.artwork(url)
}

pub fn submit(action: Action, reply: Reply) {
    dispatch(Message::Action(action, reply));
}

fn dispatch(message: Message) {
    dispatch2::DispatchQueue::main().exec_async(move || {
        autoreleasepool(|_| PLAYER.with_borrow_mut(|player| player.handle(message)));
    });
}

pub fn tick() -> Status {
    PLAYER.with_borrow_mut(Player::tick)
}

pub fn shutdown() {
    PLAYER.with_borrow_mut(Player::shutdown);
    crate::spectrum::clear();
}

struct AppleAudio {
    player: Retained<AVPlayer>,
    volume: Cell<f32>,
    gain: Cell<f32>,
    scale: Cell<f32>,
    capture: Arc<crate::spectrum::Capture>,
}

fn activate_audio_session() -> Result<()> {
    #[cfg(target_os = "ios")]
    crate::mobile::activate_audio_session()?;
    Ok(())
}

impl AudioOutput for AppleAudio {
    fn load(output: &mut Option<Self>, media: &Media, play: bool, volume: f32) -> Result<()> {
        activate_audio_session()?;
        let url = NSURL::URLWithString(&NSString::from_str(media.url.as_str()))
            .ok_or_else(|| anyhow!(tr!("Invalid audio URL")))?;
        let main = MainThreadMarker::new().expect("AVPlayer main thread");
        let capture = crate::spectrum::begin(musicx_core::protocol::track_key(&media.track));
        let output = output.get_or_insert_with(|| {
            let audio = Self {
                player: unsafe { AVPlayer::new(main) },
                volume: Cell::new(volume),
                gain: Cell::new(1.0),
                scale: Cell::new(1.0),
                capture: capture.clone(),
            };
            audio.set_volume(volume);
            audio
        });
        output.capture.set_running(false);
        output.capture = capture.clone();
        output.gain.set(musicx_core::audio::normalization_scale(
            media.gain_db,
            media.peak,
        ));
        output.set_normalization(musicx_core::audio_quality::normalization_enabled());
        output.set_volume(volume);
        unsafe {
            let item = if let Some(agent) = &media.user_agent {
                let value = NSString::from_str(agent);
                let object: &AnyObject = &value;
                let options = NSDictionary::from_slices(&[AVURLAssetHTTPUserAgentKey], &[object]);
                let asset = AVURLAsset::URLAssetWithURL_options(&url, Some(&options));
                AVPlayerItem::playerItemWithAsset(&asset, main)
            } else {
                AVPlayerItem::playerItemWithURL(&url, main)
            };
            output.player.replaceCurrentItemWithPlayerItem(Some(&item));
            crate::spectrum::attach(&item, capture);
        }
        if play {
            output.play();
        } else {
            output.pause();
        }
        Ok(())
    }

    fn snapshot(&self) -> AudioSnapshot {
        unsafe {
            let item = self.player.currentItem();
            let state = match self.player.timeControlStatus() {
                AVPlayerTimeControlStatus::Playing => AudioState::Playing,
                AVPlayerTimeControlStatus::WaitingToPlayAtSpecifiedRate => AudioState::Waiting,
                _ => AudioState::Paused,
            };
            self.capture.set_running(state == AudioState::Playing);
            AudioSnapshot {
                position: self.player.currentTime().seconds(),
                duration: item.as_ref().map(|item| item.duration().seconds()),
                state,
                volume: self.volume.get(),
                error: item
                    .and_then(|item| item.error())
                    .map(|error| error.localizedDescription().to_string()),
            }
        }
    }

    fn prepare_resume(&self) -> Result<()> {
        activate_audio_session()
    }

    fn play(&self) {
        self.capture.set_running(true);
        unsafe { self.player.play() };
    }

    fn pause(&self) {
        self.capture.set_running(false);
        unsafe { self.player.pause() };
    }

    fn set_normalization(&self, enabled: bool) {
        self.scale.set(if enabled { self.gain.get() } else { 1.0 });
        self.set_volume(self.volume.get());
    }
    fn set_volume(&self, volume: f32) {
        self.volume.set(volume);
        unsafe { self.player.setVolume(volume * self.scale.get()) };
    }

    fn seek(&self, seconds: f64, complete: impl Fn(bool) + Send + 'static) {
        let serial = self.capture.begin_seek();
        let capture = self.capture.clone();
        let completion = block2::RcBlock::new(move |finished: objc2::runtime::Bool| {
            capture.complete_seek(serial);
            complete(finished.as_bool());
        });
        unsafe {
            self.player
                .seekToTime_toleranceBefore_toleranceAfter_completionHandler(
                    CMTime::with_seconds(seconds, 1000),
                    CMTime::with_seconds(0.0, 1000),
                    CMTime::with_seconds(0.0, 1000),
                    &completion,
                );
        }
    }
}

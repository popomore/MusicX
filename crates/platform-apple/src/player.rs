use anyhow::{Result, anyhow};
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
use std::{cell::RefCell, sync::OnceLock};

pub use musicx_core::player::{Action, State, Status};

static SOURCE: OnceLock<&'static dyn DataSource> = OnceLock::new();
thread_local! {
    static PLAYER: RefCell<Player<AppleAudio>> =
        RefCell::new(Player::new(dispatch, *SOURCE.get().expect("configured data source")));
}

pub fn configure(source: &'static dyn DataSource) -> Result<()> {
    SOURCE.set(source).map_err(|_| anyhow!("播放核心已配置"))
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
}

struct AppleAudio(Retained<AVPlayer>);

fn activate_audio_session() -> Result<()> {
    #[cfg(target_os = "ios")]
    crate::mobile::activate_audio_session()?;
    Ok(())
}

impl AudioOutput for AppleAudio {
    fn load(output: &mut Option<Self>, media: &Media, play: bool, volume: f32) -> Result<()> {
        activate_audio_session()?;
        let url = NSURL::URLWithString(&NSString::from_str(media.url.as_str()))
            .ok_or_else(|| anyhow!("无效的音频地址"))?;
        let main = MainThreadMarker::new().expect("AVPlayer main thread");
        let output = output.get_or_insert_with(|| {
            let audio = Self(unsafe { AVPlayer::new(main) });
            audio.set_volume(volume);
            audio
        });
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
            output.0.replaceCurrentItemWithPlayerItem(Some(&item));
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
            let item = self.0.currentItem();
            let state = match self.0.timeControlStatus() {
                AVPlayerTimeControlStatus::Playing => AudioState::Playing,
                AVPlayerTimeControlStatus::WaitingToPlayAtSpecifiedRate => AudioState::Waiting,
                _ => AudioState::Paused,
            };
            AudioSnapshot {
                position: self.0.currentTime().seconds(),
                duration: item.as_ref().map(|item| item.duration().seconds()),
                state,
                volume: self.0.volume(),
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
        unsafe { self.0.play() };
    }

    fn pause(&self) {
        unsafe { self.0.pause() };
    }

    fn set_volume(&self, volume: f32) {
        unsafe { self.0.setVolume(volume) };
    }

    fn seek(&self, seconds: f64, complete: impl Fn(bool) + Send + 'static) {
        let completion = block2::RcBlock::new(move |finished: objc2::runtime::Bool| {
            complete(finished.as_bool());
        });
        unsafe {
            self.0
                .seekToTime_toleranceBefore_toleranceAfter_completionHandler(
                    CMTime::with_seconds(seconds, 1000),
                    CMTime::with_seconds(0.0, 1000),
                    CMTime::with_seconds(0.0, 1000),
                    &completion,
                );
        }
    }
}

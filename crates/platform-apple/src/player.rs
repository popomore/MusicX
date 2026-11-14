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
    runtime::{AnyObject, ProtocolObject},
};
use objc2_av_foundation::{
    AVPlayer, AVPlayerItem, AVPlayerItemDidPlayToEndTimeNotification, AVPlayerTimeControlStatus,
    AVURLAsset, AVURLAssetHTTPUserAgentKey,
};
use objc2_core_media::CMTime;
use objc2_foundation::{
    NSDictionary, NSNotification, NSNotificationCenter, NSObjectProtocol, NSString, NSURL,
};
use std::{
    cell::{Cell, RefCell},
    ptr::NonNull,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
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
}

struct AppleAudio {
    player: Retained<AVPlayer>,
    end_observer: Option<EndObserver>,
    volume: Cell<f32>,
    gain: Cell<f32>,
    scale: Cell<f32>,
    remote: Option<Arc<dyn musicx_plugin_api::RemotePlayback>>,
}

struct EndObserver {
    token: Retained<ProtocolObject<dyn NSObjectProtocol>>,
    ended: Arc<AtomicBool>,
}

impl EndObserver {
    fn new(item: &AnyObject) -> Self {
        let ended = Arc::new(AtomicBool::new(false));
        let signal = ended.clone();
        let block = block2::RcBlock::new(move |_: NonNull<NSNotification>| {
            signal.store(true, Ordering::Relaxed);
        });
        // With no queue, the callback runs on the posting thread. It only
        // touches an atomic flag, so no player or main-thread state is borrowed.
        let token = unsafe {
            NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
                Some(AVPlayerItemDidPlayToEndTimeNotification),
                Some(item),
                None,
                &block,
            )
        };
        Self { token, ended }
    }
}

impl Drop for EndObserver {
    fn drop(&mut self) {
        unsafe { NSNotificationCenter::defaultCenter().removeObserver(self.token.as_ref()) };
    }
}

fn activate_audio_session() -> Result<()> {
    #[cfg(target_os = "ios")]
    crate::mobile::activate_audio_session()?;
    Ok(())
}

impl AudioOutput for AppleAudio {
    fn load(output: &mut Option<Self>, media: &Media, play: bool, volume: f32) -> Result<()> {
        let url = NSURL::URLWithString(&NSString::from_str(media.url.as_str()))
            .ok_or_else(|| anyhow!(tr!("Invalid audio URL")))?;
        let main = MainThreadMarker::new().expect("AVPlayer main thread");
        let output = output.get_or_insert_with(|| {
            let audio = Self {
                player: unsafe { AVPlayer::new(main) },
                end_observer: None,
                volume: Cell::new(volume),
                gain: Cell::new(1.0),
                scale: Cell::new(1.0),
                remote: None,
            };
            audio.set_volume(volume);
            audio
        });
        if media.url.scheme() == musicx_plugin_api::REMOTE_SCHEME {
            let remote = SOURCE
                .get()
                .and_then(|source| source.remote(&media.track))
                .ok_or_else(|| anyhow!(tr!("Remote playback is not available for this source")))?;
            remote.start(play, volume)?;
            output.end_observer.take();
            output.remote = Some(remote);
            output.volume.set(volume);
            unsafe {
                output.player.pause();
                output.player.replaceCurrentItemWithPlayerItem(None);
            }
            return Ok(());
        }
        output.remote.take();
        activate_audio_session()?;
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
            // Give each item its own flag so a late notification from the
            // replaced item can never end its successor.
            output.end_observer = Some(EndObserver::new(&item));
            output.player.replaceCurrentItemWithPlayerItem(Some(&item));
        }
        if play {
            output.play();
        } else {
            output.pause();
        }
        Ok(())
    }

    fn snapshot(&self) -> AudioSnapshot {
        if let Some(remote) = &self.remote {
            let snapshot = remote.snapshot();
            return AudioSnapshot {
                position: snapshot.position,
                duration: snapshot.duration,
                state: if snapshot.waiting {
                    AudioState::Waiting
                } else if snapshot.playing {
                    AudioState::Playing
                } else {
                    AudioState::Paused
                },
                volume: snapshot.volume,
                error: snapshot.error,
            };
        }
        unsafe {
            let item = self.player.currentItem();
            let state = if self
                .end_observer
                .as_ref()
                .is_some_and(|observer| observer.ended.load(Ordering::Relaxed))
            {
                AudioState::Ended
            } else {
                match self.player.timeControlStatus() {
                    AVPlayerTimeControlStatus::Playing => AudioState::Playing,
                    AVPlayerTimeControlStatus::WaitingToPlayAtSpecifiedRate => AudioState::Waiting,
                    _ => AudioState::Paused,
                }
            };
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
        if self.remote.is_some() {
            return Ok(());
        }
        activate_audio_session()
    }

    fn play(&self) {
        if let Some(remote) = &self.remote {
            remote.play();
            return;
        }
        unsafe { self.player.play() };
    }

    fn pause(&self) {
        if let Some(remote) = &self.remote {
            remote.pause();
            return;
        }
        unsafe { self.player.pause() };
    }

    fn set_normalization(&self, enabled: bool) {
        if self.remote.is_some() {
            return;
        }
        self.scale.set(if enabled { self.gain.get() } else { 1.0 });
        self.set_volume(self.volume.get());
    }
    fn set_volume(&self, volume: f32) {
        self.volume.set(volume);
        if let Some(remote) = &self.remote {
            remote.set_volume(volume);
            return;
        }
        unsafe { self.player.setVolume(volume * self.scale.get()) };
    }

    fn seek(&self, seconds: f64, complete: impl Fn(bool) + Send + 'static) {
        if let Some(remote) = &self.remote {
            remote.seek(seconds, Box::new(complete));
            return;
        }
        if let Some(observer) = &self.end_observer {
            observer.ended.store(false, Ordering::Relaxed);
        }
        let completion = block2::RcBlock::new(move |finished: objc2::runtime::Bool| {
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

#[cfg(test)]
mod tests {
    use super::*;
    use objc2_foundation::NSObject;

    fn post_end(item: &AnyObject) {
        unsafe {
            NSNotificationCenter::defaultCenter()
                .postNotificationName_object(AVPlayerItemDidPlayToEndTimeNotification, Some(item));
        }
    }

    #[test]
    fn end_notifications_are_scoped_to_the_loaded_item() {
        autoreleasepool(|_| {
            let old_item = NSObject::new();
            let item = NSObject::new();
            let mut observer = EndObserver::new(&old_item);
            post_end(&old_item);
            assert!(observer.ended.load(Ordering::Relaxed));
            observer = EndObserver::new(&item);
            post_end(&old_item);
            assert!(!observer.ended.load(Ordering::Relaxed));
            post_end(&item);
            assert!(observer.ended.load(Ordering::Relaxed));
        });
    }

    #[test]
    fn dropping_end_observer_unregisters_its_callback() {
        autoreleasepool(|_| {
            let item = NSObject::new();
            let observer = EndObserver::new(&item);
            let ended = observer.ended.clone();
            drop(observer);
            post_end(&item);
            assert!(!ended.load(Ordering::Relaxed));
        });
    }
}

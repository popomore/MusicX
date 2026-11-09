use objc2::{AllocAnyThread, define_class, msg_send, rc::Retained, runtime::AnyObject, sel};
#[cfg(not(target_os = "macos"))]
use objc2_foundation::NSDictionary as NowPlayingDictionary;
#[cfg(target_os = "macos")]
use objc2_foundation::NSMutableDictionary as NowPlayingDictionary;
use objc2_foundation::{NSNumber, NSObject, NSObjectProtocol, NSString};
use objc2_media_player::*;
use std::time::Instant;

use crate::player::{Action, State, Status, submit};

#[cfg(target_os = "macos")]
mod cover;

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "MiniMusicRemoteCommands"]
    struct RemoteTarget;

    unsafe impl NSObjectProtocol for RemoteTarget {}

    impl RemoteTarget {
        #[unsafe(method(play:))]
        fn play(&self, _event: &MPRemoteCommandEvent) -> MPRemoteCommandHandlerStatus {
            submit(Action::Resume, None);
            MPRemoteCommandHandlerStatus::Success
        }
        #[unsafe(method(pause:))]
        fn pause(&self, _event: &MPRemoteCommandEvent) -> MPRemoteCommandHandlerStatus {
            submit(Action::Pause, None);
            MPRemoteCommandHandlerStatus::Success
        }
        #[unsafe(method(toggle:))]
        fn toggle(&self, _event: &MPRemoteCommandEvent) -> MPRemoteCommandHandlerStatus {
            submit(Action::Toggle, None);
            MPRemoteCommandHandlerStatus::Success
        }
        #[unsafe(method(previous:))]
        fn previous(&self, _event: &MPRemoteCommandEvent) -> MPRemoteCommandHandlerStatus {
            submit(Action::Previous, None);
            MPRemoteCommandHandlerStatus::Success
        }
        #[unsafe(method(next:))]
        fn next(&self, _event: &MPRemoteCommandEvent) -> MPRemoteCommandHandlerStatus {
            submit(Action::Next, None);
            MPRemoteCommandHandlerStatus::Success
        }
        #[unsafe(method(seek:))]
        fn seek(&self, event: &MPChangePlaybackPositionCommandEvent) -> MPRemoteCommandHandlerStatus {
            submit(Action::Seek { seconds: unsafe { event.positionTime() }, track_key: None }, None);
            MPRemoteCommandHandlerStatus::Success
        }
    }
);

pub struct SystemMedia {
    center: Retained<MPNowPlayingInfoCenter>,
    commands: Retained<MPRemoteCommandCenter>,
    target: Retained<RemoteTarget>,
    #[cfg(target_os = "macos")]
    cover: cover::Cover,
    last: Option<(Status, Instant)>,
}

impl SystemMedia {
    pub fn new() -> Self {
        // The target only dispatches thread-safe messages to the main queue.
        let target: Retained<RemoteTarget> = unsafe { msg_send![RemoteTarget::alloc(), init] };
        unsafe {
            let commands = MPRemoteCommandCenter::sharedCommandCenter();
            for (command, selector) in [
                (commands.playCommand(), sel!(play:)),
                (commands.pauseCommand(), sel!(pause:)),
                (commands.togglePlayPauseCommand(), sel!(toggle:)),
                (commands.previousTrackCommand(), sel!(previous:)),
                (commands.nextTrackCommand(), sel!(next:)),
            ] {
                command.addTarget_action(&target, selector);
                command.setEnabled(false);
            }
            commands
                .changePlaybackPositionCommand()
                .addTarget_action(&target, sel!(seek:));
            commands.changePlaybackPositionCommand().setEnabled(false);
            commands.stopCommand().setEnabled(false);
            Self {
                center: MPNowPlayingInfoCenter::defaultCenter(),
                commands,
                target,
                #[cfg(target_os = "macos")]
                cover: cover::Cover::default(),
                last: None,
            }
        }
    }

    pub fn update(&mut self, status: Status) {
        let playing = status.state == State::Playing;
        #[cfg(target_os = "macos")]
        let cover_changed = self
            .cover
            .update(status.track.as_ref().and_then(|t| t.cover_url.as_deref()));
        #[cfg(not(target_os = "macos"))]
        let cover_changed = false;
        let unchanged = self.last.as_ref().is_some_and(|(old, at)| {
            let expected = old.position
                + if playing {
                    at.elapsed().as_secs_f64()
                } else {
                    0.0
                };
            old.track == status.track
                && old.state == status.state
                && old.index == status.index
                && old.queue_length == status.queue_length
                && old.switching == status.switching
                && (old.duration - status.duration).abs() < 0.1
                && (expected - status.position).abs() < 1.5
                && at.elapsed().as_secs() < 30
        });
        if unchanged && !cover_changed {
            return;
        }
        unsafe {
            let loaded = status.track.is_some() && !status.switching;
            self.commands.playCommand().setEnabled(loaded && !playing);
            self.commands
                .pauseCommand()
                .setEnabled(loaded && matches!(status.state, State::Playing | State::Loading));
            self.commands.togglePlayPauseCommand().setEnabled(loaded);
            self.commands
                .previousTrackCommand()
                .setEnabled(loaded && status.index.is_some_and(|i| i > 0));
            self.commands
                .nextTrackCommand()
                .setEnabled(loaded && status.index.is_some_and(|i| i + 1 < status.queue_length));
            self.commands
                .changePlaybackPositionCommand()
                .setEnabled(loaded && status.duration > 0.0);
            if let Some(track) = &status.track {
                let title = NSString::from_str(&track.title);
                let artist = NSString::from_str(&track.artist);
                let duration = NSNumber::numberWithDouble(status.duration);
                let position = NSNumber::numberWithDouble(status.position);
                let rate = NSNumber::numberWithDouble(if playing { 1.0 } else { 0.0 });
                let kind = NSNumber::numberWithUnsignedInteger(MPNowPlayingInfoMediaType::Audio.0);
                let values: [&AnyObject; 6] = [&title, &artist, &duration, &position, &rate, &kind];
                let info = NowPlayingDictionary::from_slices(
                    &[
                        MPMediaItemPropertyTitle,
                        MPMediaItemPropertyArtist,
                        MPMediaItemPropertyPlaybackDuration,
                        MPNowPlayingInfoPropertyElapsedPlaybackTime,
                        MPNowPlayingInfoPropertyPlaybackRate,
                        MPNowPlayingInfoPropertyMediaType,
                    ],
                    &values,
                );
                #[cfg(target_os = "macos")]
                if let Some(artwork) = &self.cover.artwork {
                    info.insert(MPMediaItemPropertyArtwork, artwork);
                }
                self.center.setNowPlayingInfo(Some(&info));
            } else {
                self.center.setNowPlayingInfo(None);
            }
            #[cfg(target_os = "macos")]
            self.center.setPlaybackState(match status.state {
                State::Playing | State::Loading => MPNowPlayingPlaybackState::Playing,
                State::Paused => MPNowPlayingPlaybackState::Paused,
                _ => MPNowPlayingPlaybackState::Stopped,
            });
        }
        self.last = Some((status, Instant::now()));
    }
}

impl Drop for SystemMedia {
    fn drop(&mut self) {
        unsafe {
            #[cfg(target_os = "macos")]
            self.center
                .setPlaybackState(MPNowPlayingPlaybackState::Stopped);
            self.center.setNowPlayingInfo(None);
            for command in [
                self.commands.playCommand(),
                self.commands.pauseCommand(),
                self.commands.togglePlayPauseCommand(),
                self.commands.previousTrackCommand(),
                self.commands.nextTrackCommand(),
            ] {
                command.removeTarget(Some(&self.target));
            }
            self.commands
                .changePlaybackPositionCommand()
                .removeTarget(Some(&self.target));
        }
    }
}

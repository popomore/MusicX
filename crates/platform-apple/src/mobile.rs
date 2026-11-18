//! iPhone owns its AVPlayer and local service; it never controls the Mac process.
use crate::{player, system_media::SystemMedia};
use anyhow::Result;
use musicx_core::protocol::tr;
use objc2::{
    MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::{Retained, autoreleasepool},
    sel,
};
use objc2_foundation::{NSObject, NSObjectProtocol, NSRunLoop, NSRunLoopCommonModes, NSTimer};
use std::{cell::RefCell, sync::OnceLock};

thread_local! { static MOBILE: RefCell<Option<MobileRuntime>> = const { RefCell::new(None) }; }
struct MobileRuntime {
    _target: Retained<MobileTick>,
    _timer: Retained<NSTimer>,
    media: Option<SystemMedia>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MiniCloudMobileTick"]
    struct MobileTick;
    unsafe impl NSObjectProtocol for MobileTick {}
    impl MobileTick {
        #[unsafe(method(tick:))]
        fn tick(&self, _timer: &NSTimer) {
            autoreleasepool(|_| {
                let status = player::tick();
                MOBILE.with_borrow_mut(|runtime| {
                    if let Some(runtime) = runtime && status.track.is_some() {
                        runtime.media.get_or_insert_with(SystemMedia::new).update(status);
                    }
                });
            });
        }
    }
);

pub fn start() -> Result<()> {
    static STARTED: OnceLock<std::result::Result<(), String>> = OnceLock::new();
    let result = STARTED.get_or_init(|| {
        dispatch2::DispatchQueue::main().exec_async(|| {
            let mtm = MainThreadMarker::new().expect("UIKit main thread");
            let target: Retained<MobileTick> = unsafe { msg_send![MobileTick::alloc(mtm), init] };
            let timer = unsafe {
                NSTimer::timerWithTimeInterval_target_selector_userInfo_repeats(
                    0.5,
                    &target,
                    sel!(tick:),
                    None,
                    true,
                )
            };
            timer.setTolerance(0.1);
            unsafe {
                NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes);
            }
            MOBILE.with_borrow_mut(|runtime| {
                *runtime = Some(MobileRuntime {
                    _target: target,
                    _timer: timer,
                    media: None,
                })
            });
        });
        Ok(())
    });
    result
        .as_ref()
        .map_err(|error| anyhow::anyhow!(error.clone()))
        .copied()
}

pub fn activate_audio_session() -> Result<()> {
    use objc2_avf_audio::{AVAudioSession, AVAudioSessionCategoryPlayback};
    unsafe {
        let session = AVAudioSession::sharedInstance();
        session
            .setCategory_error(
                AVAudioSessionCategoryPlayback.ok_or_else(|| {
                    anyhow::anyhow!(tr!("System playback audio category missing"))
                })?,
            )
            .map_err(|e| anyhow::anyhow!(tr!("Could not enable background audio: {e}"; e = e)))?;
        session
            .setActive_error(true)
            .map_err(|e| anyhow::anyhow!(tr!("Could not activate audio: {e}"; e = e)))?;
    }
    Ok(())
}

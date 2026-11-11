use objc2_foundation::{NSDistributedNotificationCenter, NSString};

pub fn version() -> String {
    let version =
        std::env::var("MUSICX_APP_VERSION").unwrap_or_else(|_| env!("CARGO_PKG_VERSION").into());
    format!("MusicX {version}")
}

pub fn check() {
    let Ok(pid) = std::env::var("MUSICX_UPDATER_PID") else {
        return;
    };
    dispatch2::DispatchQueue::main().exec_async(move || unsafe {
        NSDistributedNotificationCenter::defaultCenter()
            .postNotificationName_object_userInfo_deliverImmediately(
                &NSString::from_str(&crate::protocol::check_updates_notification()),
                Some(&NSString::from_str(&pid)),
                None,
                true,
            );
    });
}

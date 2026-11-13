use objc2_foundation::{NSDistributedNotificationCenter, NSString};

pub fn version() -> String {
    format!("MusicX {}", release_version())
}

pub fn release_version() -> String {
    std::env::var("MUSICX_APP_VERSION").unwrap_or_else(|_| env!("CARGO_PKG_VERSION").into())
}

pub fn check() {
    notify(&crate::protocol::check_updates_notification());
}

pub fn channel_changed() {
    notify(&crate::protocol::update_channel_notification());
}

fn notify(name: &str) {
    let Ok(pid) = std::env::var("MUSICX_UPDATER_PID") else {
        return;
    };
    let name = name.to_owned();
    dispatch2::DispatchQueue::main().exec_async(move || unsafe {
        NSDistributedNotificationCenter::defaultCenter()
            .postNotificationName_object_userInfo_deliverImmediately(
                &NSString::from_str(&name),
                Some(&NSString::from_str(&pid)),
                None,
                true,
            );
    });
}

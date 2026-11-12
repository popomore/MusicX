//! Sparkle runs only in the outer, persistent application, never in the panel.
use base64::{Engine, prelude::BASE64_STANDARD};
use musicx_core::protocol::UpdateChannel;
use musicx_core::protocol::tr;
use objc2::{
    MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::{Allocated, Retained},
    runtime::{AnyClass, AnyObject},
};
use objc2_app_kit::{NSAlert, NSApplication};
use objc2_foundation::{NSBundle, NSObject, NSObjectProtocol, NSSet, NSString, ns_string};
use std::path::Path;

pub const FEED_URL: &str =
    "https://github.com/popomore/MusicX/releases/download/update-feed/appcast.xml";

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MusicXUpdaterDelegate"]
    struct UpdateDelegate;

    unsafe impl NSObjectProtocol for UpdateDelegate {}

    impl UpdateDelegate {
        #[unsafe(method_id(allowedChannelsForUpdater:))]
        fn channels(&self, _updater: &AnyObject) -> Retained<NSSet<NSString>> {
            match UpdateChannel::load() {
                UpdateChannel::Stable => NSSet::new(),
                UpdateChannel::All => NSSet::from_slice(&[ns_string!("preview")]),
            }
        }
    }
);

pub struct Updater {
    _framework: Retained<NSBundle>,
    controller: Retained<AnyObject>,
    _delegate: Retained<UpdateDelegate>,
}

impl Updater {
    pub fn start() -> anyhow::Result<Option<Self>> {
        MainThreadMarker::new().expect("updater main thread");
        let bundle = NSBundle::mainBundle();
        if bundle
            .objectForInfoDictionaryKey(ns_string!("SUPublicEDKey"))
            .is_none()
        {
            return Ok(None);
        }
        let url = bundle
            .privateFrameworksURL()
            .ok_or_else(|| anyhow::anyhow!(tr!("缺少 Frameworks 目录")))?
            .URLByAppendingPathComponent(ns_string!("Sparkle.framework"))
            .ok_or_else(|| anyhow::anyhow!(tr!("无效的 Sparkle.framework 路径")))?;
        let framework = NSBundle::bundleWithURL(&url)
            .ok_or_else(|| anyhow::anyhow!(tr!("缺少 Sparkle.framework")))?;
        anyhow::ensure!(
            unsafe { framework.load() },
            tr!("无法加载 Sparkle.framework")
        );
        let delegate: Retained<UpdateDelegate> = unsafe {
            msg_send![
                UpdateDelegate::alloc(MainThreadMarker::new().expect("main thread")),
                init
            ]
        };
        let class = AnyClass::get(c"SPUStandardUpdaterController")
            .ok_or_else(|| anyhow::anyhow!(tr!("缺少 Sparkle 更新控制器")))?;
        // Sparkle's documented initializer; the retained controller and framework
        // stay alive on AppKit's main thread until the helper terminates.
        let controller = unsafe {
            let allocated: Allocated<AnyObject> = msg_send![class, alloc];
            msg_send![allocated, initWithStartingUpdater: true,
                updaterDelegate: &*delegate, userDriverDelegate: None::<&AnyObject>]
        };
        Ok(Some(Self {
            _framework: framework,
            controller,
            _delegate: delegate,
        }))
    }

    pub fn controller(&self) -> Retained<AnyObject> {
        self.controller.clone()
    }

    pub fn channel_changed(&self) {
        unsafe {
            let updater: Retained<AnyObject> = msg_send![&*self.controller, updater];
            let _: () = msg_send![&*updater, resetUpdateCycleAfterShortDelay];
        }
    }

    pub fn check(controller: &AnyObject) {
        unsafe {
            let updater: Retained<AnyObject> = msg_send![controller, updater];
            let ready: bool = msg_send![&*updater, canCheckForUpdates];
            if ready {
                NSApplication::sharedApplication(MainThreadMarker::new().expect("main thread"))
                    .activate();
                let _: () = msg_send![controller, checkForUpdates: None::<&AnyObject>];
            }
        }
    }
}

pub fn unavailable(message: &str) {
    let mtm = MainThreadMarker::new().expect("main thread");
    NSApplication::sharedApplication(mtm).activate();
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(tr!("无法检查更新")));
    alert.setInformativeText(&NSString::from_str(message));
    alert.addButtonWithTitle(&NSString::from_str(tr!("好")));
    alert.runModal();
}

/// Configure only distribution bundles. Development caches never update themselves.
pub fn package(bundle: &Path, signing: Option<&std::ffi::OsStr>) -> anyhow::Result<String> {
    let Some(key) = std::env::var_os("MUSICX_SPARKLE_PUBLIC_KEY") else {
        return Ok(String::new());
    };
    let key = key.to_str().unwrap_or_default().trim();
    anyhow::ensure!(
        BASE64_STANDARD.decode(key).is_ok_and(|key| key.len() == 32),
        tr!("Sparkle 公钥必须为 Base64 编码的 32 字节 Ed25519 公钥")
    );
    let source = std::env::var_os("MUSICX_SPARKLE_FRAMEWORK")
        .ok_or_else(|| anyhow::anyhow!(tr!("缺少 MUSICX_SPARKLE_FRAMEWORK")))?;
    let destination = bundle.join("Contents/Frameworks/Sparkle.framework");
    std::fs::create_dir_all(destination.parent().expect("Frameworks"))?;
    let copied = std::process::Command::new("/usr/bin/ditto")
        .arg(&source)
        .arg(&destination)
        .status()?;
    anyhow::ensure!(copied.success(), tr!("复制 Sparkle.framework 失败"));
    // Sign nested code from the inside out, keeping hardened runtime enabled.
    for relative in [
        "Versions/B/XPCServices/Downloader.xpc",
        "Versions/B/XPCServices/Installer.xpc",
        "Versions/B/Autoupdate",
        "Versions/B/Updater.app",
        "",
    ] {
        let mut command = std::process::Command::new("/usr/bin/codesign");
        command
            .args(["--force", "--sign"])
            .arg(signing.unwrap_or(std::ffi::OsStr::new("-")));
        if signing.is_some() {
            command.args(["--options", "runtime", "--timestamp"]);
        }
        let result = command.arg(destination.join(relative)).output()?;
        anyhow::ensure!(
            result.status.success(),
            tr!(
                "Sparkle 签名失败：{}",
                String::from_utf8_lossy(&result.stderr)
            )
        );
    }
    Ok(format!(
        r#"
<key>SUFeedURL</key><string>{FEED_URL}</string>
<key>SUPublicEDKey</key><string>{key}</string>
<key>SUBundleName</key><string>{name}</string>
<key>SUVerifyUpdateBeforeExtraction</key><true/>
<key>SURequireSignedFeed</key><true/>
<key>SUEnableAutomaticChecks</key><true/>
<key>SUAutomaticallyUpdate</key><true/>
<key>SUScheduledCheckInterval</key><integer>86400</integer>
<key>SUEnableSystemProfiling</key><false/>
"#,
        name = musicx_core::protocol::APP_NAME,
    ))
}

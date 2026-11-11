use objc2::{
    AnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::{Retained, autoreleasepool},
    runtime::{AnyObject, ProtocolObject},
    sel,
};
use objc2_app_kit::*;
use objc2_foundation::{
    NSBundle, NSData, NSDistributedNotificationCenter, NSNotification,
    NSNotificationSuspensionBehavior, NSObject, NSObjectProtocol, NSRunLoop, NSRunLoopCommonModes,
    NSSize, NSString, NSTimer, ns_string,
};
use std::{
    cell::RefCell,
    ffi::OsStr,
    path::{Path, PathBuf},
    process::ExitCode,
};

use crate::{
    player::{self, State},
    system_media::SystemMedia,
};
pub use musicx_core::protocol::APP_NAME;
use musicx_core::protocol::component_app_id;

const APP_RELEASE_VERSION: &str = match option_env!("MUSICX_RELEASE_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};
const APP_BUNDLE_VERSION: &str = match option_env!("MUSICX_BUNDLE_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};
const WEB_AUTH_APP: &str = "WebAuth.app";
const PANEL_BIN: &str = "musicx-panel";
const WEB_AUTH_BIN: &str = "musicx-webauth";

thread_local! {
    static DESKTOP: RefCell<Option<Desktop>> = const { RefCell::new(None) };
}

struct PanelProcess {
    child: std::process::Child,
}

struct Desktop {
    working_dir: PathBuf,
    panel_bundle: PathBuf,
    _delegate: Retained<Delegate>,
    status_item: Retained<NSStatusItem>,
    status_icon: Option<Retained<NSImage>>,
    playing_icon: Option<Retained<NSImage>>,
    timer: Retained<NSTimer>,
    media: Option<SystemMedia>,
    panel: Option<PanelProcess>,
    updater: Option<crate::updater::Updater>,
    updater_error: String,
    stopping: bool,
    last_title: String,
    last_artist: String,
    last_state: Option<State>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MiniMusicAppDelegate"]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    impl Delegate {
        #[unsafe(method(tick:))]
        fn tick(&self, _timer: &NSTimer) { refresh(); }
        #[unsafe(method(statusClicked:))]
        fn status_clicked(&self, button: &NSStatusBarButton) {
            let mtm = self.mtm();
            let app = NSApplication::sharedApplication(mtm);
            if let Some(event) = app.currentEvent().filter(|event| {
                event.r#type() == NSEventType::RightMouseUp
                    || (event.r#type() == NSEventType::LeftMouseUp
                        && event.modifierFlags().contains(NSEventModifierFlags::Control))
            }) {
                let menu = NSMenu::new(mtm);
                unsafe {
                    menu.addItemWithTitle_action_keyEquivalent(
                        ns_string!("检查更新…"), Some(sel!(checkForUpdates:)), ns_string!(""),
                    ).setTarget(Some(self));
                    menu.addItem(&NSMenuItem::separatorItem(mtm));
                    menu.addItemWithTitle_action_keyEquivalent(
                        ns_string!("退出 MusicX"), Some(sel!(quitFromStatus:)), ns_string!(""),
                    ).setTarget(Some(self));
                }
                NSMenu::popUpContextMenu_withEvent_forView(&menu, &event, button);
            } else {
                show_panel();
            }
        }
        #[unsafe(method(quitFromStatus:))]
        fn quit_from_status(&self, _sender: Option<&AnyObject>) { quit(); }
        #[unsafe(method(checkForUpdates:))]
        fn check_updates(&self, _sender: Option<&AnyObject>) { check_for_updates(); }
    }

    unsafe impl NSApplicationDelegate for Delegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn launched(&self, _notification: &NSNotification) {
            let result = crate::updater::Updater::start();
            DESKTOP.with_borrow_mut(|desktop| {
                if let Some(desktop) = desktop {
                    match result {
                        Ok(updater) => desktop.updater = updater,
                        Err(error) => desktop.updater_error = format!("更新器启动失败：{error}"),
                    }
                }
            });
        }
        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn keep_playing(&self, _app: &NSApplication) -> bool { false }
        #[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
        fn reopen(&self, _app: &NSApplication, _visible: bool) -> bool { show_panel(); false }
        #[unsafe(method(applicationShouldTerminate:))]
        fn terminate(&self, _app: &NSApplication) -> NSApplicationTerminateReply {
            shutdown();
            NSApplicationTerminateReply::TerminateNow
        }
    }


);

/// Each process has its own macOS application identity.
fn package(
    executable: &Path,
    bundle: &Path,
    name: &str,
    id: &str,
    binary: &str,
    agent: bool,
    signing_identity: Option<&OsStr>,
) -> anyhow::Result<()> {
    use std::fs;
    let contents = bundle.join("Contents");
    let bin = contents.join("MacOS").join(binary);
    fs::create_dir_all(bin.parent().expect("bundle executable parent"))?;
    let temporary = bin.with_extension("next");
    fs::copy(executable, &temporary)?;
    fs::rename(temporary, &bin)?;
    let resources = contents.join("Resources");
    fs::create_dir_all(&resources)?;
    fs::write(
        resources.join("musicx.icns"),
        include_bytes!("../../../assets/musicx.icns"),
    )?;
    let ui_element = if agent { "true" } else { "false" };
    let updater = if binary == "musicx" && contents.join("Helpers").is_dir() {
        crate::updater::package(bundle, signing_identity)?
    } else {
        String::new()
    };
    fs::write(
        contents.join("Info.plist"),
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>{binary}</string>
<key>CFBundleIdentifier</key><string>{id}</string>
<key>CFBundleName</key><string>{name}</string>
<key>CFBundleDisplayName</key><string>{name}</string>
<key>CFBundleIconFile</key><string>musicx.icns</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>{APP_BUNDLE_VERSION}</string>
<key>CFBundleVersion</key><string>{APP_BUNDLE_VERSION}</string>
<key>MusicXReleaseVersion</key><string>{APP_RELEASE_VERSION}</string>
<key>LSUIElement</key><{ui_element}/>
<key>LSMinimumSystemVersion</key><string>14.0</string>
<key>NSAppTransportSecurity</key><dict><key>NSAllowsLocalNetworking</key><true/></dict>
{updater}
</dict></plist>"#,
        ),
    )?;
    let mut codesign = std::process::Command::new("/usr/bin/codesign");
    codesign.args(["--force", "--sign"]);
    if let Some(identity) = signing_identity {
        codesign
            .arg(identity)
            .args(["--options", "runtime", "--timestamp"]);
    } else {
        codesign.arg("-");
    }
    let signature = codesign.arg(bundle).output()?;
    anyhow::ensure!(
        signature.status.success(),
        "无法签名 {name}：{}",
        String::from_utf8_lossy(&signature.stderr).trim()
    );
    Ok(())
}

fn runtime_dir() -> anyhow::Result<PathBuf> {
    use std::os::unix::fs::DirBuilderExt;
    let path = musicx_core::storage::state_dir()?.join("runtime");
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&path)?;
    Ok(path.canonicalize()?)
}

pub struct DesktopRuntime {
    working_dir: PathBuf,
    panel_bundle: PathBuf,
    pub web_auth_executable: PathBuf,
    open_panel: bool,
}

fn app_executable(bundle: &Path, name: &str) -> PathBuf {
    bundle.join("Contents/MacOS").join(name)
}

fn build_component(executable: &Path, name: &str) -> anyhow::Result<PathBuf> {
    let component = executable.with_file_name(name);
    anyhow::ensure!(component.is_file(), "缺少构建产物 {name}，请先运行 ./run");
    Ok(component)
}

pub fn package_app(bundle: &Path, web_auth_name: &str) -> anyhow::Result<()> {
    let executable = std::env::current_exe()?;
    let signing = std::env::var_os("MUSICX_CODESIGN_IDENTITY").filter(|id| !id.is_empty());
    package_app_from_executable(&executable, bundle, web_auth_name, signing.as_deref())
}

fn package_app_from_executable(
    executable: &Path,
    bundle: &Path,
    web_auth_name: &str,
    signing: Option<&OsStr>,
) -> anyhow::Result<()> {
    anyhow::ensure!(!bundle.exists(), "目标应用包已存在：{}", bundle.display());
    let panel = build_component(executable, PANEL_BIN)?;
    let webauth = build_component(executable, WEB_AUTH_BIN)?;
    let helpers = bundle.join("Contents/Helpers");
    package(
        &panel,
        &helpers.join(format!("{APP_NAME}.app")),
        APP_NAME,
        &component_app_id("panel"),
        PANEL_BIN,
        false,
        signing,
    )?;
    package(
        &webauth,
        &helpers.join(WEB_AUTH_APP),
        web_auth_name,
        &component_app_id("webauth"),
        WEB_AUTH_BIN,
        true,
        signing,
    )?;
    package(
        executable,
        bundle,
        &format!("{APP_NAME} Helper"),
        &component_app_id("playback"),
        "musicx",
        true,
        signing,
    )
}

pub fn prepare_runtime(web_auth_name: &str) -> anyhow::Result<DesktopRuntime> {
    use std::{os::unix::process::CommandExt, process::Command};
    let executable = std::env::current_exe()?;
    if let Some(contents) = executable.parent().and_then(Path::parent)
        && contents.file_name() == Some(OsStr::new("Contents"))
    {
        let helpers = contents.join("Helpers");
        if helpers.is_dir() {
            let panel_bundle = helpers.join(format!("{APP_NAME}.app"));
            let web_auth_executable = app_executable(&helpers.join(WEB_AUTH_APP), WEB_AUTH_BIN);
            anyhow::ensure!(
                app_executable(&panel_bundle, PANEL_BIN).is_file() && web_auth_executable.is_file(),
                "MusicX 应用包不完整"
            );
            return Ok(DesktopRuntime {
                working_dir: musicx_core::storage::state_dir()?,
                panel_bundle,
                web_auth_executable,
                open_panel: true,
            });
        }
    }
    let base = runtime_dir()?;
    let core_bundle = base.join(format!("{APP_NAME}.app"));
    let private_executable = app_executable(&core_bundle, "musicx");
    let panel_bundle = base.join("panel").join(format!("{APP_NAME}.app"));
    let web_auth_bundle = base.join(format!("{web_auth_name}.app"));
    if executable == private_executable {
        return Ok(DesktopRuntime {
            panel_bundle,
            web_auth_executable: app_executable(&web_auth_bundle, WEB_AUTH_BIN),
            working_dir: base,
            open_panel: false,
        });
    }
    if executable
        .parent()
        .is_some_and(|p| p.file_name().is_some_and(|n| n == "MacOS"))
    {
        // A project-directory app redirects to the CLI-prepared private cache.
        // It never needs access to sibling files in the Downloads folder.
        anyhow::ensure!(
            private_executable.is_file(),
            "请先在项目中运行 ./run 准备私有运行文件"
        );
    } else {
        let panel = build_component(&executable, PANEL_BIN)?;
        let webauth = build_component(&executable, WEB_AUTH_BIN)?;
        package(
            &webauth,
            &web_auth_bundle,
            web_auth_name,
            &component_app_id("webauth"),
            WEB_AUTH_BIN,
            true,
            None,
        )?;
        // Stage both executables before starting AppKit. Opening the panel later
        // only launches this prepared bundle, with no project-file I/O on main.
        package(
            &panel,
            &panel_bundle,
            APP_NAME,
            &component_app_id("panel"),
            PANEL_BIN,
            false,
            None,
        )?;
        package(
            &executable,
            &core_bundle,
            &format!("{APP_NAME} Helper"),
            &component_app_id("playback"),
            "musicx",
            true,
            None,
        )?;
    }
    Err(Command::new(private_executable)
        .current_dir(&base)
        .env("MUSICX_DATA_DIR", base.parent().expect("runtime parent"))
        .args(std::env::args().skip(1))
        .exec()
        .into())
}

fn load_status_icon(png: &[u8]) -> Option<Retained<NSImage>> {
    let data = NSData::with_bytes(png);
    let icon = NSImage::initWithData(NSImage::alloc(), &data)?;
    icon.setSize(NSSize::new(17.0, 17.0));
    icon.setTemplate(true);
    Some(icon)
}

pub fn run(runtime: DesktopRuntime) -> ExitCode {
    let mtm = MainThreadMarker::new().expect("AppKit main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    let delegate: Retained<Delegate> = unsafe { msg_send![Delegate::alloc(mtm), init] };
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    unsafe {
        NSDistributedNotificationCenter::defaultCenter()
            .addObserver_selector_name_object_suspensionBehavior(
                &delegate,
                sel!(checkForUpdates:),
                Some(&NSString::from_str(
                    &musicx_core::protocol::check_updates_notification(),
                )),
                Some(&NSString::from_str(&std::process::id().to_string())),
                NSNotificationSuspensionBehavior::DeliverImmediately,
            );
    }

    let status_item = NSStatusBar::systemStatusBar().statusItemWithLength(NSSquareStatusItemLength);
    let status_icon = load_status_icon(include_bytes!("../../../assets/musicx-menu.png"));
    let playing_icon = load_status_icon(include_bytes!("../../../assets/musicx-menu-playing.png"));
    if let Some(button) = status_item.button(mtm) {
        button.setImagePosition(NSCellImagePosition::ImageOnly);
        unsafe {
            button.setTarget(Some(&delegate));
            button.setAction(Some(sel!(statusClicked:)));
        }
        button.sendActionOn(NSEventMask::LeftMouseUp | NSEventMask::RightMouseUp);
    }

    let timer = unsafe {
        NSTimer::timerWithTimeInterval_target_selector_userInfo_repeats(
            0.5,
            &delegate,
            sel!(tick:),
            None,
            true,
        )
    };
    timer.setTolerance(0.1);
    unsafe {
        NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes);
    }
    let DesktopRuntime {
        working_dir,
        panel_bundle,
        open_panel,
        ..
    } = runtime;
    DESKTOP.with_borrow_mut(|desktop| {
        *desktop = Some(Desktop {
            working_dir,
            panel_bundle,
            _delegate: delegate.clone(),
            status_item,
            status_icon,
            playing_icon,
            timer,
            media: None,
            panel: None,
            updater: None,
            updater_error: "此构建尚未配置自动更新。请从 MusicX 的 GitHub Releases 下载新版本。"
                .into(),
            stopping: false,
            last_title: String::new(),
            last_artist: String::new(),
            last_state: None,
        })
    });
    refresh();
    if open_panel {
        dispatch2::DispatchQueue::main().exec_async(show_panel);
    }
    app.run();
    app.setDelegate(None);
    ExitCode::SUCCESS
}

/// Reopen the exact child, including a minimized or hidden window. Opening its
/// bundle URL can select another registered copy and doesn't transfer activation.
pub fn reveal_panel(process_id: u32) {
    if let Some(panel) =
        NSRunningApplication::runningApplicationWithProcessIdentifier(process_id as i32)
    {
        NSApplication::sharedApplication(MainThreadMarker::new().expect("main thread"))
            .yieldActivationToApplication(&panel);
    }
    unsafe {
        NSDistributedNotificationCenter::defaultCenter()
            .postNotificationName_object_userInfo_deliverImmediately(
                &NSString::from_str(&musicx_core::protocol::panel_reopen_notification()),
                Some(&NSString::from_str(&process_id.to_string())),
                None,
                true,
            );
    }
}

fn show_panel() {
    let result = DESKTOP.with_borrow_mut(|desktop| -> anyhow::Result<()> {
        let Some(desktop) = desktop else {
            return Ok(());
        };
        if let Some(panel) = &mut desktop.panel
            && panel.child.try_wait()?.is_none()
        {
            reveal_panel(panel.child.id());
            return Ok(());
        }
        let bundle = desktop.panel_bundle.clone();
        let panel_executable = app_executable(&bundle, PANEL_BIN);
        anyhow::ensure!(panel_executable.is_file(), "缺少 MusicX 播放面板");
        NSApplication::sharedApplication(MainThreadMarker::new().expect("main thread"))
            .yieldActivationToApplicationWithBundleIdentifier(&NSString::from_str(
                &component_app_id("panel"),
            ));
        let bundle = NSBundle::mainBundle();
        let bundle_string = |key: &str, fallback: &str| {
            bundle
                .objectForInfoDictionaryKey(&NSString::from_str(key))
                .and_then(|value| value.downcast_ref::<NSString>().map(ToString::to_string))
                .unwrap_or_else(|| fallback.to_owned())
        };
        let child = std::process::Command::new(panel_executable)
            .current_dir(&desktop.working_dir)
            .arg("panel")
            .env("MUSICX_UPDATER_PID", std::process::id().to_string())
            .env(
                "MUSICX_APP_VERSION",
                bundle_string("MusicXReleaseVersion", APP_RELEASE_VERSION),
            )
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .spawn()?;
        desktop.panel = Some(PanelProcess { child });
        Ok(())
    });
    if let Err(error) = result {
        eprintln!("打开选歌面板失败：{error:#}");
    }
}

pub fn refresh() {
    autoreleasepool(|_| {
        let status = player::tick();
        DESKTOP.with_borrow_mut(|desktop| {
            let Some(desktop) = desktop else {
                return;
            };
            if desktop
                .panel
                .as_mut()
                .is_some_and(|p| p.child.try_wait().ok().flatten().is_some())
            {
                desktop.panel = None;
            }
            let title = status.track.as_ref().map_or("", |t| t.title.as_str());
            let artist = status.track.as_ref().map_or("", |t| t.artist.as_str());
            if desktop.last_title != title
                || desktop.last_artist != artist
                || desktop.last_state != Some(status.state)
            {
                let state = match status.state {
                    State::Idle => "等待选歌",
                    State::Loading => "加载中",
                    State::Playing => "播放中",
                    State::Paused => "已暂停",
                    State::Ended => "播放结束",
                    State::Error => "播放出错",
                };
                let detail = if title.is_empty() {
                    format!("{APP_NAME} · {state} · 点击打开播放器")
                } else {
                    format!("{title} · {artist} · {state} · 点击打开播放器")
                };
                if let Some(button) = desktop
                    .status_item
                    .button(MainThreadMarker::new().expect("main thread"))
                {
                    if desktop.last_state != Some(status.state) {
                        let icon = if status.state == State::Playing {
                            desktop
                                .playing_icon
                                .as_deref()
                                .or(desktop.status_icon.as_deref())
                        } else {
                            desktop.status_icon.as_deref()
                        };
                        button.setImage(icon);
                    }
                    button.setToolTip(Some(&NSString::from_str(&detail)));
                    button.setAccessibilityLabel(Some(&NSString::from_str(&detail)));
                }
                desktop.last_title = title.to_owned();
                desktop.last_artist = artist.to_owned();
                desktop.last_state = Some(status.state);
            }
            if status.track.is_some() {
                desktop
                    .media
                    .get_or_insert_with(SystemMedia::new)
                    .update(status);
            }
        });
    });
}

fn check_for_updates() {
    // Clone no AppKit objects across threads and release the RefCell borrow
    // before entering Sparkle's UI (which may run a nested event loop).
    let updater = DESKTOP.with_borrow(|desktop| {
        desktop.as_ref().map(|d| {
            (
                d.updater.as_ref().map(|u| u.controller()),
                d.updater_error.clone(),
            )
        })
    });
    if let Some((controller, error)) = updater {
        if let Some(controller) = controller {
            crate::updater::Updater::check(&controller);
        } else {
            crate::updater::unavailable(&error);
        }
    }
}

fn shutdown() {
    DESKTOP.with_borrow_mut(|desktop| {
        if let Some(desktop) = desktop.as_mut().filter(|d| !d.stopping) {
            desktop.stopping = true;
            desktop.timer.invalidate();
            if let Some(mut panel) = desktop.panel.take() {
                let _ = panel.child.kill();
                let _ = panel.child.wait();
            }
            NSStatusBar::systemStatusBar().removeStatusItem(&desktop.status_item);
        }
    });
    player::shutdown();
}

pub fn quit() {
    dispatch2::DispatchQueue::main().exec_async(|| {
        NSApplication::sharedApplication(MainThreadMarker::new().expect("main thread"))
            .terminate(None);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, process::Command, time::SystemTime};

    #[test]
    fn packaged_player_name_and_component_identities() -> anyhow::Result<()> {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("musicx-package-{}-{nonce}", std::process::id()));
        let build = root.join("build");
        fs::create_dir_all(&build)?;
        for binary in ["musicx", PANEL_BIN, WEB_AUTH_BIN] {
            fs::copy("/usr/bin/true", build.join(binary))?;
        }
        let bundle = root.join(format!("{APP_NAME}.app"));
        let web_auth_name = format!("{APP_NAME} 网页登录");
        package_app_from_executable(&build.join("musicx"), &bundle, &web_auth_name, None)?;

        // The Dock can use the bundle filename; the visible child must carry
        // the product name even when its Info.plist already has the right name.
        let panel = bundle
            .join("Contents/Helpers")
            .join(format!("{APP_NAME}.app"));
        assert!(
            panel.is_dir(),
            "visible player is still packaged as Panel.app"
        );
        assert!(!bundle.join("Contents/Helpers/Panel.app").exists());
        let version = std::env::var("MUSICX_BUNDLE_VERSION")
            .unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_owned());
        let release_version = std::env::var("MUSICX_RELEASE_VERSION")
            .unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_owned());

        for (path, component, name) in [
            (bundle.clone(), "playback", format!("{APP_NAME} Helper")),
            (panel, "panel", APP_NAME.to_owned()),
            (
                bundle.join("Contents/Helpers").join(WEB_AUTH_APP),
                "webauth",
                web_auth_name,
            ),
        ] {
            for (key, expected) in [
                ("CFBundleIdentifier", component_app_id(component)),
                ("CFBundleName", name.clone()),
                ("CFBundleDisplayName", name.clone()),
                ("CFBundleShortVersionString", version.clone()),
                ("CFBundleVersion", version.clone()),
                ("MusicXReleaseVersion", release_version.clone()),
            ] {
                let result = Command::new("/usr/bin/plutil")
                    .args(["-extract", key, "raw", "-o", "-"])
                    .arg(path.join("Contents/Info.plist"))
                    .output()?;
                assert!(result.status.success());
                assert_eq!(String::from_utf8(result.stdout)?.trim(), expected);
            }
        }
        let verify = Command::new("/usr/bin/codesign")
            .args(["--verify", "--deep", "--strict"])
            .arg(&bundle)
            .output()?;
        assert!(
            verify.status.success(),
            "{}",
            String::from_utf8_lossy(&verify.stderr)
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }
}

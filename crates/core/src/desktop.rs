use objc2::{
    AnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::{Retained, autoreleasepool},
    runtime::{AnyObject, ProtocolObject},
    sel,
};
use objc2_app_kit::*;
use objc2_foundation::{
    NSData, NSObject, NSObjectProtocol, NSPoint, NSRunLoop, NSRunLoopCommonModes, NSSize, NSString,
    NSTimer, NSURL, ns_string,
};
use std::{cell::RefCell, process::ExitCode};

use crate::{
    player::{self, State},
    system_media::SystemMedia,
};

const APP_NAME: &str = "MusicX";
const BACKGROUND_NAME: &str = "MusicX Helper";
const APP_BUNDLE_VERSION: &str = "6";

thread_local! {
    static DESKTOP: RefCell<Option<Desktop>> = const { RefCell::new(None) };
}

struct PanelProcess {
    child: std::process::Child,
    bundle: std::path::PathBuf,
}

struct Desktop {
    _delegate: Retained<Delegate>,
    status_item: Retained<NSStatusItem>,
    status_icon: Option<Retained<NSImage>>,
    playing_icon: Option<Retained<NSImage>>,
    timer: Retained<NSTimer>,
    media: Option<SystemMedia>,
    panel: Option<PanelProcess>,
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
    }

    unsafe impl NSApplicationDelegate for Delegate {
        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn keep_playing(&self, _app: &NSApplication) -> bool { false }
        #[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
        fn reopen(&self, _app: &NSApplication, _visible: bool) -> bool { show_panel(); false }
        #[unsafe(method(applicationShouldTerminate:))]
        fn terminate(&self, _app: &NSApplication) -> NSApplicationTerminateReply {
            quit();
            NSApplicationTerminateReply::TerminateCancel
        }
    }


);

/// Each process has an application identity without desktop installation.
fn package(
    executable: &std::path::Path,
    bundle: &std::path::Path,
    name: &str,
    id: &str,
    binary: &str,
    agent: bool,
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
<key>CFBundleVersion</key><string>{APP_BUNDLE_VERSION}</string>
<key>LSUIElement</key><{ui_element}/>
<key>LSMinimumSystemVersion</key><string>14.0</string>
</dict></plist>"#
        ),
    )?;
    let signature = std::process::Command::new("/usr/bin/codesign")
        .args(["--force", "--sign", "-"])
        .arg(bundle)
        .output()?;
    anyhow::ensure!(
        signature.status.success(),
        "无法签名 {name}：{}",
        String::from_utf8_lossy(&signature.stderr).trim()
    );
    Ok(())
}

fn runtime_dir() -> anyhow::Result<std::path::PathBuf> {
    use std::os::unix::fs::DirBuilderExt;
    let path = crate::netease::state_dir()?.join("runtime");
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&path)?;
    Ok(path)
}

pub fn bundle_if_needed() -> anyhow::Result<()> {
    use std::{os::unix::process::CommandExt, process::Command};
    let executable = std::env::current_exe()?;
    let base = runtime_dir()?;
    let core_bundle = base.join(format!("{APP_NAME}.app"));
    let private_executable = core_bundle.join("Contents/MacOS/mini-cloudmusic");
    if executable == private_executable {
        return Ok(());
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
        let panel = executable
            .parent()
            .ok_or_else(|| anyhow::anyhow!("找不到构建目录"))?
            .join("mini-cloudmusic-panel");
        anyhow::ensure!(panel.is_file(), "请先运行 ./run 完成面板构建");
        let webauth = executable
            .parent()
            .expect("build directory")
            .join("mini-cloudmusic-webauth");
        anyhow::ensure!(webauth.is_file(), "请先运行./run准备网页登录组件");
        package(
            &webauth,
            &base.join(format!("{}.app", crate::web_auth::APP_NAME)),
            crate::web_auth::APP_NAME,
            "local.mini-cloudmusic.webauth",
            "mini-cloudmusic-webauth",
            true,
        )?;
        // Stage both executables before starting AppKit. Opening the panel later
        // only launches this prepared bundle, with no project-file I/O on main.
        package(
            &panel,
            &base.join(format!("panel/{APP_NAME}.app")),
            APP_NAME,
            "local.mini-cloudmusic.panel",
            "mini-cloudmusic-panel",
            false,
        )?;
        package(
            &executable,
            &core_bundle,
            BACKGROUND_NAME,
            "local.musicx.playback",
            "mini-cloudmusic",
            true,
        )?;
    }
    Err(Command::new(private_executable)
        .current_dir(&base)
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

pub fn run() -> ExitCode {
    let mtm = MainThreadMarker::new().expect("AppKit main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    let delegate: Retained<Delegate> = unsafe { msg_send![Delegate::alloc(mtm), init] };
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));

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
    DESKTOP.with_borrow_mut(|desktop| {
        *desktop = Some(Desktop {
            _delegate: delegate.clone(),
            status_item,
            status_icon,
            playing_icon,
            timer,
            media: None,
            panel: None,
            last_title: String::new(),
            last_artist: String::new(),
            last_state: None,
        })
    });
    refresh();
    app.run();
    app.setDelegate(None);
    ExitCode::SUCCESS
}

fn show_panel() {
    let result = DESKTOP.with_borrow_mut(|desktop| -> anyhow::Result<()> {
        let Some(desktop) = desktop else {
            return Ok(());
        };
        if let Some(panel) = &mut desktop.panel
            && panel.child.try_wait()?.is_none()
        {
            let url = NSURL::fileURLWithPath(&NSString::from_str(&panel.bundle.to_string_lossy()));
            NSWorkspace::sharedWorkspace().openURL(&url);
            return Ok(());
        }
        let base = runtime_dir()?;
        let bundle = base.join("panel").join(format!("{APP_NAME}.app"));
        let panel_executable = bundle.join("Contents/MacOS/mini-cloudmusic-panel");
        anyhow::ensure!(
            panel_executable.is_file(),
            "缺少选歌面板，请先运行项目的 ./run 完成构建"
        );
        let child = std::process::Command::new(panel_executable)
            .current_dir(base)
            .arg("panel")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .spawn()?;
        desktop.panel = Some(PanelProcess { child, bundle });
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

pub(crate) fn quit() {
    dispatch2::DispatchQueue::main().exec_async(|| {
        DESKTOP.with_borrow_mut(|desktop| {
            if let Some(mut desktop) = desktop.take() {
                desktop.timer.invalidate();
                if let Some(mut panel) = desktop.panel.take() {
                    let _ = panel.child.kill();
                    let _ = panel.child.wait();
                }
                NSStatusBar::systemStatusBar().removeStatusItem(&desktop.status_item);
            }
        });
        player::shutdown();
        stop_application();
    });
}

fn stop_application() {
    let mtm = MainThreadMarker::new().expect("main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.stop(None);
    if let Some(event) = NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
            NSEventType::ApplicationDefined, NSPoint::new(0.0, 0.0), NSEventModifierFlags::empty(), 0.0, 0, None, 0, 0, 0) {
            app.postEvent_atStart(&event, true);
        }
}

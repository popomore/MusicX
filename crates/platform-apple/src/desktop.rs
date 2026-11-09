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

const APP_NAME: &str = "MusicX";
const BACKGROUND_NAME: &str = "MusicX Helper";
const APP_BUNDLE_VERSION: &str = "6";
const PANEL_APP: &str = "Panel.app";
const WEB_AUTH_APP: &str = "WebAuth.app";
const PANEL_BIN: &str = "mini-cloudmusic-panel";
const WEB_AUTH_BIN: &str = "mini-cloudmusic-webauth";

thread_local! {
    static DESKTOP: RefCell<Option<Desktop>> = const { RefCell::new(None) };
}

struct PanelProcess {
    child: std::process::Child,
    bundle: std::path::PathBuf,
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
<key>CFBundleShortVersionString</key><string>{version}</string>
<key>CFBundleVersion</key><string>{APP_BUNDLE_VERSION}</string>
<key>LSUIElement</key><{ui_element}/>
<key>LSMinimumSystemVersion</key><string>14.0</string>
</dict></plist>"#,
            version = env!("CARGO_PKG_VERSION"),
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
    anyhow::ensure!(!bundle.exists(), "目标应用包已存在：{}", bundle.display());
    let executable = std::env::current_exe()?;
    let panel = build_component(&executable, PANEL_BIN)?;
    let webauth = build_component(&executable, WEB_AUTH_BIN)?;
    let helpers = bundle.join("Contents/Helpers");
    let signing = std::env::var_os("MUSICX_CODESIGN_IDENTITY").filter(|id| !id.is_empty());
    let signing = signing.as_deref();
    package(
        &panel,
        &helpers.join(PANEL_APP),
        APP_NAME,
        "local.mini-cloudmusic.panel",
        PANEL_BIN,
        false,
        signing,
    )?;
    package(
        &webauth,
        &helpers.join(WEB_AUTH_APP),
        web_auth_name,
        "local.mini-cloudmusic.webauth",
        WEB_AUTH_BIN,
        true,
        signing,
    )?;
    package(
        &executable,
        bundle,
        BACKGROUND_NAME,
        "local.musicx.playback",
        "mini-cloudmusic",
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
            let panel_bundle = helpers.join(PANEL_APP);
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
    let private_executable = app_executable(&core_bundle, "mini-cloudmusic");
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
            "local.mini-cloudmusic.webauth",
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
            "local.mini-cloudmusic.panel",
            PANEL_BIN,
            false,
            None,
        )?;
        package(
            &executable,
            &core_bundle,
            BACKGROUND_NAME,
            "local.musicx.playback",
            "mini-cloudmusic",
            true,
            None,
        )?;
    }
    Err(Command::new(private_executable)
        .current_dir(&base)
        .env(
            "MINI_CLOUDMUSIC_DATA_DIR",
            base.parent().expect("runtime parent"),
        )
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
        let bundle = desktop.panel_bundle.clone();
        let panel_executable = app_executable(&bundle, PANEL_BIN);
        anyhow::ensure!(panel_executable.is_file(), "缺少 MusicX 播放面板");
        let child = std::process::Command::new(panel_executable)
            .current_dir(&desktop.working_dir)
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

pub fn quit() {
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

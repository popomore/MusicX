use nsh_platform_macos::objc2_app_kit::*;
use nsh_platform_macos::{AppOptions, AppView, Root, set_ui_sets};
use objc2::{
    AnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, ProtocolObject},
    sel,
};
use objc2_foundation::{
    NSBundle, NSDistributedNotificationCenter, NSNotification, NSNotificationSuspensionBehavior,
    NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, ns_string,
};
use std::{cell::RefCell, process::ExitCode, rc::Rc};
pub const PLAYER_WIDTH: f32 = 320.0;
pub const DETAILS_WIDTH: f32 = 296.0;
pub const DETAILS_GAP: f32 = 12.0;
pub const EXPANDED_WIDTH: f32 = PLAYER_WIDTH + DETAILS_GAP + DETAILS_WIDTH;
thread_local! { static PANEL: RefCell<Option<Panel>> = const { RefCell::new(None) }; }
struct Panel {
    window: Retained<NSWindow>,
    view: Rc<AppView>,
    details_left: std::cell::Cell<bool>,
}

fn visible_frame(window: &NSWindow) -> Option<NSRect> {
    window
        .screen()
        .or_else(|| NSScreen::mainScreen(MainThreadMarker::from(window)))
        .map(|screen| screen.visibleFrame())
}

fn prefer_left(player: NSRect, visible: NSRect) -> bool {
    let left = player.origin.x - visible.origin.x;
    let right = visible.origin.x + visible.size.width - (player.origin.x + player.size.width);
    left > right
}

fn details_frame(mut frame: NSRect, visible: Option<NSRect>, expanded: bool, left: bool) -> NSRect {
    let was_expanded = frame.size.width > f64::from(PLAYER_WIDTH) + 1.0;
    let distance = f64::from(DETAILS_WIDTH + DETAILS_GAP);
    let player_x = frame.origin.x + if was_expanded && left { distance } else { 0.0 };
    frame.size.width = f64::from(if expanded {
        EXPANDED_WIDTH
    } else {
        PLAYER_WIDTH
    });
    frame.origin.x = player_x - if expanded && left { distance } else { 0.0 };
    if let Some(visible) = visible {
        let rightmost =
            (visible.origin.x + visible.size.width - frame.size.width).max(visible.origin.x);
        frame.origin.x = frame.origin.x.clamp(visible.origin.x, rightmost);
    }
    frame
}

pub fn request_details_left() -> tokio::sync::oneshot::Receiver<bool> {
    let (send, receive) = tokio::sync::oneshot::channel();
    // NSH handles clicks on its engine thread. Only AppKit's main thread
    // owns PANEL and can read the window's current screen coordinates.
    dispatch2::DispatchQueue::main().exec_async(move || {
        let left = PANEL.with_borrow(|panel| {
            let Some(panel) = panel else {
                return false;
            };
            let frame = panel.window.frame();
            if frame.size.width > f64::from(PLAYER_WIDTH) + 1.0 {
                return panel.details_left.get();
            }
            visible_frame(&panel.window).is_some_and(|visible| prefer_left(frame, visible))
        });
        let _ = send.send(left);
    });
    receive
}

/// One surface expands beside the fixed-width player. Defer native resize
/// until the current NSH commit is over, as with minimize/close.
pub fn set_details_expanded(expanded: bool, left: bool) {
    dispatch2::DispatchQueue::main().exec_async(move || {
        PANEL.with_borrow(|panel| {
            if let Some(panel) = panel {
                let frame = panel.window.frame();
                let next = details_frame(frame, visible_frame(&panel.window), expanded, left);
                panel.details_left.set(left);
                if next != frame {
                    panel.window.setFrame_display(next, true);
                }
            }
        });
    });
}
define_class!(
    #[unsafe(super(NSWindow, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MiniMusicPanelWindow"]
    struct PanelWindow;
    unsafe impl NSObjectProtocol for PanelWindow {}
    impl PanelWindow {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key_window(&self) -> bool { true }
        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main_window(&self) -> bool { true }
        #[unsafe(method(performClose:))]
        fn perform_close(&self, _sender: Option<&AnyObject>) { self.close(); }
    }
);

fn item(
    menu: &NSMenu,
    delegate: &AnyObject,
    title: &str,
    selector: Option<objc2::runtime::Sel>,
    key: &str,
) -> Retained<NSMenuItem> {
    let mtm = MainThreadMarker::new().expect("main thread");
    // Every target is held for the lifetime of the menus and uses a matching selector.
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title),
            selector,
            &NSString::from_str(key),
        )
    };
    unsafe {
        item.setTarget(Some(delegate));
    }
    menu.addItem(&item);
    item
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

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MiniMusicPanelDelegate"]
    struct PanelDelegate;
    unsafe impl NSObjectProtocol for PanelDelegate {}
    impl PanelDelegate {
        #[unsafe(method(revealPanel:))]
        fn reveal(&self, _notification: &NSNotification) {
            show_panel();
        }
        #[unsafe(method(closePanel:))]
        fn close(&self, _sender: Option<&AnyObject>) {
            PANEL.with_borrow(|panel| { if let Some(panel) = panel { panel.window.performClose(None); } });
        }
        #[unsafe(method(quit:))]
        fn quit(&self, _sender: Option<&AnyObject>) { crate::remote::quit_background(); }
    }
    unsafe impl NSApplicationDelegate for PanelDelegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn launched(&self, _notification: &NSNotification) {
            show_panel();
        }
        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn keep_loop(&self, _app: &NSApplication) -> bool { false }
        #[unsafe(method(applicationShouldTerminate:))]
        fn terminate(&self, _app: &NSApplication) -> NSApplicationTerminateReply {
            crate::remote::quit_background(); NSApplicationTerminateReply::TerminateCancel
        }
        #[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
        fn reopen(&self, _app: &NSApplication, _visible: bool) -> bool {
            show_panel();
            false
        }
    }
    unsafe impl NSWindowDelegate for PanelDelegate {
        #[unsafe(method(windowWillClose:))]
        fn closed(&self, _notification: &NSNotification) {
            dispatch2::DispatchQueue::main().exec_async(stop_panel);
        }
    }
);

fn show_panel() {
    let app = NSApplication::sharedApplication(MainThreadMarker::new().expect("main thread"));
    app.activate();
    PANEL.with_borrow(|panel| {
        if let Some(panel) = panel {
            if panel.window.isMiniaturized() {
                panel.window.deminiaturize(None);
            }
            panel.window.makeKeyAndOrderFront(None);
        }
    });
}

pub fn run_panel() -> ExitCode {
    nsh_core::image::budget().set_limit(4 * 1024 * 1024);
    let mtm = MainThreadMarker::new().expect("main thread");
    let app = NSApplication::sharedApplication(mtm);
    // Only the disposable panel owns the Dock icon and the application menu.
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    if let Some(url) = NSBundle::mainBundle()
        .URLForResource_withExtension(Some(ns_string!("musicx")), Some(ns_string!("icns")))
        && let Some(icon) = NSImage::initWithContentsOfURL(NSImage::alloc(), &url)
    {
        unsafe { app.setApplicationIconImage(Some(&icon)) };
    }
    let delegate: Retained<PanelDelegate> = unsafe { msg_send![PanelDelegate::alloc(mtm), init] };
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    let bar = NSMenu::new(mtm);
    let holder = NSMenuItem::new(mtm);
    bar.addItem(&holder);
    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);
    item(&menu, &delegate, "关闭面板", Some(sel!(closePanel:)), "w");
    item(&menu, &delegate, "退出 MusicX", Some(sel!(quit:)), "q");
    holder.setSubmenu(Some(&menu));
    let edit_holder = NSMenuItem::new(mtm);
    edit_holder.setTitle(ns_string!("编辑"));
    let edit = NSMenu::new(mtm);
    for (title, selector, key) in [
        ("撤销", sel!(undo:), "z"),
        ("重做", sel!(redo:), "Z"),
        ("剪切", sel!(cut:), "x"),
        ("复制", sel!(copy:), "c"),
        ("粘贴", sel!(paste:), "v"),
        ("全选", sel!(selectAll:), "a"),
    ] {
        // Standard AppKit text actions follow the responder chain.
        unsafe {
            item(&edit, &delegate, title, Some(selector), key).setTarget(None);
        }
    }
    edit_holder.setSubmenu(Some(&edit));
    bar.addItem(&edit_holder);
    app.setMainMenu(Some(&bar));
    set_ui_sets(vec![nsh_base::registry_of([
        (|registry: &mut nsh_core::element::Registry| {
            registry
                .register::<nsh_base::Button>()
                .register::<nsh_base::Slider>()
                .register::<nsh_base::Sheet>()
                .register::<nsh_base::Popover>();
        }) as nsh_base::parts::Part,
    ])]);
    let options = AppOptions {
        title: "MusicX".into(),
        app_id: "local.musicx.panel".into(),
        size: (320, 512),
        appearance: nsh_platform_macos::Appearance::Light,
        transparent: true,
        cache: nsh_platform_macos::CacheOptions::default().with_images(
            nsh_platform_macos::CachePolicy::MaxSize(nsh_platform_macos::ByteSize::mib(4)),
        ),
        ..Default::default()
    };
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(320.0, 512.0));
    let window: Retained<PanelWindow> = unsafe {
        msg_send![PanelWindow::alloc(mtm), initWithContentRect: frame,
        styleMask: (NSWindowStyleMask::Closable | NSWindowStyleMask::Miniaturizable), backing: NSBackingStoreType::Buffered, defer: false]
    };
    let window = Retained::into_super(window);
    unsafe {
        window.setReleasedWhenClosed(false);
    }
    window.setTitle(ns_string!("MusicX"));
    window.setOpaque(false);
    window.setBackgroundColor(Some(&NSColor::clearColor()));
    window.setHasShadow(true);
    window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    let view = AppView::new(mtm, options);
    view.set_theme_overrides(Some(include_str!("../../../../assets/theme.json")))
        .expect("valid player palette");
    window.setContentView(Some(&view.view()));
    window.center();
    view.load(nsh_platform_macos::App::Rust(Root::of::<crate::ui::App>()));
    // AppKit must finish launching before it can activate this child process.
    // The delegate also uses the same presentation path for Dock reopens.
    PANEL.with_borrow_mut(|panel| {
        *panel = Some(Panel {
            window,
            view,
            details_left: std::cell::Cell::new(false),
        })
    });
    let notifications = NSDistributedNotificationCenter::defaultCenter();
    let reopen = NSString::from_str(crate::protocol::PANEL_REOPEN_NOTIFICATION);
    let process_id = NSString::from_str(&std::process::id().to_string());
    // Observe only our own PID so installed, mounted and QA copies cannot
    // reopen one another. Register on the AppKit thread before launch completes.
    unsafe {
        notifications.addObserver_selector_name_object_suspensionBehavior(
            &delegate,
            sel!(revealPanel:),
            Some(&reopen),
            Some(&process_id),
            NSNotificationSuspensionBehavior::DeliverImmediately,
        );
    }
    app.run();
    unsafe {
        notifications.removeObserver_name_object(&delegate, Some(&reopen), Some(&process_id));
    }
    app.setDelegate(None);
    ExitCode::SUCCESS
}

pub fn stop_panel() {
    crate::source_browser::cancel_active_login();
    let panel = PANEL.with_borrow_mut(Option::take);
    if let Some(panel) = panel {
        panel.window.setDelegate(None);
        panel.window.close();
        panel.window.setContentView(None);
        panel.view.stop();
    }
    stop_application();
}

#[cfg(test)]
mod geometry_tests {
    use super::*;
    fn rect(x: f64, width: f64) -> NSRect {
        NSRect::new(NSPoint::new(x, 100.0), NSSize::new(width, 512.0))
    }
    #[test]
    fn chooses_more_room_even_when_both_sides_fit() {
        let screen = rect(0.0, 1728.0);
        assert!(!prefer_left(rect(520.0, 320.0), screen));
        assert!(!prefer_left(rect(704.0, 320.0), screen));
        assert!(prefer_left(rect(1000.0, 320.0), screen));
    }
    #[test]
    fn opens_outside_player_and_keeps_left_edge_when_room_allows() {
        let open = details_frame(rect(200.0, 320.0), Some(rect(0.0, 1728.0)), true, false);
        assert_eq!(open.origin.x, 200.0);
        assert_eq!(open.size.width, 628.0);
        let detail_left = open.origin.x + f64::from(PLAYER_WIDTH + DETAILS_GAP);
        assert!(detail_left >= open.origin.x + 320.0);
        assert_eq!(
            detail_left + f64::from(DETAILS_WIDTH),
            open.origin.x + open.size.width
        );
    }
    #[test]
    fn right_edge_and_negative_screen_coordinates_stay_visible() {
        let screen = rect(-1920.0, 1920.0);
        let player = rect(-350.0, 320.0);
        let left = prefer_left(player, screen);
        assert!(left);
        let open = details_frame(player, Some(screen), true, left);
        assert_eq!(open.origin.x + open.size.width, -30.0);
        assert_eq!(details_frame(open, Some(screen), false, left), player);
    }
    #[test]
    fn moves_the_pair_only_as_far_as_needed_when_neither_side_fits() {
        let screen = rect(100.0, 700.0);
        let player = rect(300.0, 320.0);
        let left = prefer_left(player, screen);
        assert!(left);
        let open = details_frame(player, Some(screen), true, left);
        assert_eq!(open.origin.x, screen.origin.x);
        assert!(open.origin.x + open.size.width <= 800.0);
    }
    #[test]
    fn closing_preserves_current_dragged_origin_and_height() {
        let closed = details_frame(rect(350.0, 628.0), Some(rect(0.0, 1728.0)), false, false);
        assert_eq!(closed, rect(350.0, 320.0));
    }
    #[test]
    fn left_details_preserve_the_player_screen_position_across_open_and_close() {
        let original = rect(1400.0, 320.0);
        let opened = details_frame(original, Some(rect(0.0, 1728.0)), true, true);
        assert_eq!(opened.origin.x + 308.0, original.origin.x);
        assert_eq!(
            details_frame(opened, Some(rect(0.0, 1728.0)), false, true),
            original
        );
    }
}

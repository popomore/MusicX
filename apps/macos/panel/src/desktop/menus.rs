//! AppKit owns menus; the NSH engine owns the view model and player commands.
use crate::{
    model::{Model, Page},
    protocol::{State, i18n},
    remote::{self, Action},
};
use nsh_core::prelude::*;
use nsh_platform_macos::objc2_app_kit::*;
use objc2::{MainThreadMarker, MainThreadOnly, rc::Retained, runtime::AnyObject, sel};
use objc2_foundation::{NSDictionary, NSString, NSURL};
use std::{cell::RefCell, sync::Mutex};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(isize)]
pub enum Command {
    Settings = 1,
    TogglePlayback,
    Previous,
    Next,
    VolumeUp,
    VolumeDown,
    Queue,
    History,
    Library,
    Sources,
    Lyrics,
    TrackInfo,
    MiniLyrics,
}

impl Command {
    const ALL: [Self; 13] = [
        Self::Settings,
        Self::TogglePlayback,
        Self::Previous,
        Self::Next,
        Self::VolumeUp,
        Self::VolumeDown,
        Self::Queue,
        Self::History,
        Self::Library,
        Self::Sources,
        Self::Lyrics,
        Self::TrackInfo,
        Self::MiniLyrics,
    ];

    fn from_tag(tag: isize) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|command| *command as isize == tag)
    }

    fn page(self) -> Option<Page> {
        match self {
            Self::Queue => Some(Page::Queue),
            Self::History => Some(Page::History),
            Self::Library => Some(Page::Library),
            Self::Lyrics => Some(Page::Lyrics),
            Self::TrackInfo => Some(Page::Info),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq)]
struct MenuState {
    mounted: bool,
    ready: bool,
    has_track: bool,
    playing: bool,
    can_toggle: bool,
    busy: bool,
    can_previous: bool,
    can_next: bool,
    volume: f32,
    page: Option<Page>,
    mini_lyrics: bool,
}

impl MenuState {
    fn read(model: Model) -> Self {
        model.status.with(|status| {
            let playing = matches!(status.state, State::Playing | State::Loading);
            let starting = model.starting.get();
            Self {
                mounted: true,
                ready: model.status_ready.get(),
                has_track: status.track.is_some(),
                playing,
                can_toggle: (!starting || status.switching) && (!status.switching || playing),
                busy: status.switching || starting,
                can_previous: status.index.is_some_and(|index| index > 0),
                can_next: status
                    .index
                    .is_some_and(|index| index + 1 < status.queue_length),
                volume: status.volume,
                page: model.drawer_open.get().then(|| model.page.get()),
                mini_lyrics: model.show_mini_lyrics.get(),
            }
        })
    }

    fn allows(self, command: Command) -> bool {
        self.mounted
            && match command {
                Command::TogglePlayback => self.ready && self.has_track && self.can_toggle,
                Command::Previous => self.ready && !self.busy && self.can_previous,
                Command::Next => self.ready && !self.busy && self.can_next,
                Command::VolumeUp => self.ready && self.volume < 1.0,
                Command::VolumeDown => self.ready && self.volume > 0.0,
                Command::Lyrics | Command::TrackInfo => self.has_track,
                _ => true,
            }
    }
}

static COMMANDS: Mutex<Option<UnboundedSender<Command>>> = Mutex::new(None);
thread_local! {
    static MENUS: RefCell<Menus> = RefCell::new(Menus::default());
}

#[derive(Default)]
struct Menus {
    titles: Vec<(Retained<NSMenuItem>, &'static str)>,
    commands: Vec<(Retained<NSMenuItem>, Command)>,
    state: MenuState,
}

/// Receive on the engine thread, so no AppKit callback touches an NSH Signal.
pub fn connect(model: Model) {
    let (send, mut receive) = unbounded_channel();
    *COMMANDS.lock().unwrap() = Some(send);
    effect(move || {
        let state = MenuState::read(model);
        dispatch2::DispatchQueue::main().exec_async(move || {
            MENUS.with_borrow_mut(|menus| {
                if menus.state != state {
                    menus.state = state;
                    menus.refresh();
                }
            });
        });
    });
    spawn(async move {
        while let Some(command) = receive.recv().await {
            let state = MenuState::read(model);
            if !state.allows(command) {
                continue;
            }
            if let Some(page) = command.page() {
                model.open(page);
                continue;
            }
            let action = match command {
                Command::TogglePlayback if state.playing => Action::Pause,
                Command::TogglePlayback => {
                    model.error.set(String::new());
                    Action::Resume
                }
                Command::Previous => Action::Previous,
                Command::Next => Action::Next,
                Command::VolumeUp => Action::Volume((state.volume + 0.1).min(1.0)),
                Command::VolumeDown => Action::Volume((state.volume - 0.1).max(0.0)),
                Command::MiniLyrics => {
                    model.toggle_mini_lyrics();
                    continue;
                }
                _ => unreachable!("navigation handled above"),
            };
            if matches!(action, Action::Volume(_)) {
                // A repeated volume shortcut must use the previous command's
                // confirmed result, rather than the last polling snapshot.
                let (send, receive) = tokio::sync::oneshot::channel();
                remote::submit(action, Some(send));
                match receive.await {
                    Ok(Ok(status)) => model.update(status),
                    Ok(Err(error)) => model.error.set(error.to_string()),
                    Err(_) => {}
                }
            } else {
                // Navigation keeps the same ordered queue as the player buttons.
                remote::submit(action, None);
            }
        }
    });
}

pub fn enabled(tag: isize) -> bool {
    Command::from_tag(tag)
        .is_none_or(|command| MENUS.with_borrow(|menus| menus.state.allows(command)))
}

pub fn perform(tag: isize) {
    if let Some(command) = Command::from_tag(tag)
        && enabled(tag)
    {
        if command == Command::Settings {
            super::show_settings();
            return;
        }
        if command == Command::Sources {
            super::open_settings(crate::sources_ui::SettingsGroup::Sources);
            return;
        }
        if command.page().is_some() {
            super::show_panel();
        }
        if let Some(sender) = &*COMMANDS.lock().unwrap() {
            let _ = sender.send(command);
        }
    }
}

/// Navigation from another NSH surface still belongs to the player's model.
pub fn request(command: Command) {
    dispatch2::DispatchQueue::main().exec_async(move || perform(command as isize));
}

pub fn refresh_language() {
    MENUS.with_borrow(|menus| menus.refresh());
}

impl Menus {
    fn refresh(&self) {
        for (item, title) in &self.titles {
            item.setTitle(&NSString::from_str(i18n::text(title)));
            if let Some(submenu) = item.submenu() {
                submenu.setTitle(&item.title());
            }
        }
        for (item, command) in &self.commands {
            item.setEnabled(self.state.allows(*command));
            if *command == Command::TogglePlayback {
                item.setTitle(&NSString::from_str(i18n::text(if self.state.playing {
                    "Pause"
                } else {
                    "Play"
                })));
            }
            let selected = if *command == Command::MiniLyrics {
                self.state.mini_lyrics
            } else {
                command
                    .page()
                    .is_some_and(|page| self.state.page == Some(page))
            };
            item.setState(if selected {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
        }
    }

    fn item(
        &mut self,
        menu: &NSMenu,
        target: Option<&AnyObject>,
        title: &'static str,
        action: Option<objc2::runtime::Sel>,
        key: &str,
    ) -> Retained<NSMenuItem> {
        let mtm = MainThreadMarker::new().expect("main thread");
        // Targets live for the application lifetime; nil follows the responder chain.
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(i18n::text(title)),
                action,
                &NSString::from_str(key),
            )
        };
        unsafe { item.setTarget(target) };
        menu.addItem(&item);
        self.titles.push((item.clone(), title));
        item
    }

    fn submenu(&mut self, bar: &NSMenu, title: &'static str) -> Retained<NSMenu> {
        let menu = NSMenu::new(MainThreadMarker::new().expect("main thread"));
        self.item(bar, None, title, None, "")
            .setSubmenu(Some(&menu));
        menu
    }

    fn command(
        &mut self,
        menu: &NSMenu,
        target: &AnyObject,
        title: &'static str,
        command: Command,
        key: &str,
    ) {
        let item = self.item(menu, Some(target), title, Some(sel!(menuAction:)), key);
        item.setTag(command as isize);
        self.commands.push((item, command));
    }
}

pub fn install(app: &NSApplication, delegate: &AnyObject) {
    let mtm = MainThreadMarker::new().expect("main thread");
    let mut menus = Menus::default();
    let bar = NSMenu::new(mtm);
    let separator = |menu: &NSMenu| menu.addItem(&NSMenuItem::separatorItem(mtm));
    let application = menus.submenu(&bar, "MusicX");
    menus.item(
        &application,
        Some(delegate),
        "About MusicX",
        Some(sel!(showAbout:)),
        "",
    );
    menus.item(
        &application,
        Some(delegate),
        "Check for Updates…",
        Some(sel!(checkForUpdates:)),
        "",
    );
    separator(&application);
    menus.command(&application, delegate, "Settings…", Command::Settings, ",");
    separator(&application);
    let services = menus.submenu(&application, "Services");
    app.setServicesMenu(Some(&services));
    separator(&application);
    menus.item(
        &application,
        Some(app),
        "Hide MusicX",
        Some(sel!(hide:)),
        "h",
    );
    let hide_others = menus.item(
        &application,
        Some(app),
        "Hide Others",
        Some(sel!(hideOtherApplications:)),
        "h",
    );
    hide_others
        .setKeyEquivalentModifierMask(NSEventModifierFlags::Command | NSEventModifierFlags::Option);
    menus.item(
        &application,
        Some(app),
        "Show All",
        Some(sel!(unhideAllApplications:)),
        "",
    );
    separator(&application);
    menus.item(
        &application,
        Some(delegate),
        "Quit MusicX",
        Some(sel!(quit:)),
        "q",
    );

    let edit = menus.submenu(&bar, "Edit");
    menus.item(&edit, None, "Undo", Some(sel!(undo:)), "z");
    let redo = menus.item(&edit, None, "Redo", Some(sel!(redo:)), "z");
    redo.setKeyEquivalentModifierMask(NSEventModifierFlags::Command | NSEventModifierFlags::Shift);
    separator(&edit);
    for (title, selector, key) in [
        ("Cut", sel!(cut:), "x"),
        ("Copy", sel!(copy:), "c"),
        ("Paste", sel!(paste:), "v"),
        ("Delete", sel!(delete:), ""),
    ] {
        menus.item(&edit, None, title, Some(selector), key);
    }
    separator(&edit);
    menus.item(&edit, None, "Select All", Some(sel!(selectAll:)), "a");

    let playback = menus.submenu(&bar, "Playback");
    menus.command(&playback, delegate, "Play", Command::TogglePlayback, "p");
    separator(&playback);
    menus.command(
        &playback,
        delegate,
        "Previous track",
        Command::Previous,
        "\u{f702}",
    );
    menus.command(&playback, delegate, "Next track", Command::Next, "\u{f703}");
    separator(&playback);
    menus.command(
        &playback,
        delegate,
        "Increase Volume",
        Command::VolumeUp,
        "\u{f700}",
    );
    menus.command(
        &playback,
        delegate,
        "Decrease Volume",
        Command::VolumeDown,
        "\u{f701}",
    );

    let view = menus.submenu(&bar, "View");
    for (title, command, key) in [
        ("Now playing", Command::Queue, "1"),
        ("History", Command::History, "2"),
        ("My playlists", Command::Library, "3"),
        ("Music sources", Command::Sources, "4"),
    ] {
        menus.command(&view, delegate, title, command, key);
    }
    separator(&view);
    menus.command(&view, delegate, "Lyrics", Command::Lyrics, "l");
    menus.command(
        &view,
        delegate,
        "Track information",
        Command::TrackInfo,
        "i",
    );
    separator(&view);
    menus.command(&view, delegate, "Mini Lyrics", Command::MiniLyrics, "");

    let window = menus.submenu(&bar, "Window");
    menus.item(
        &window,
        Some(delegate),
        "Show Player",
        Some(sel!(showPlayer:)),
        "0",
    );
    separator(&window);
    menus.item(
        &window,
        None,
        "Minimize",
        Some(sel!(performMiniaturize:)),
        "m",
    );
    // AppKit disables performClose: for the player's untitled window even
    // though it implements closing. Route explicitly to the current window.
    menus.item(
        &window,
        Some(delegate),
        "Close window",
        Some(sel!(closeWindow:)),
        "w",
    );
    separator(&window);
    menus.item(
        &window,
        Some(app),
        "Bring All to Front",
        Some(sel!(arrangeInFront:)),
        "",
    );
    app.setWindowsMenu(Some(&window));

    let help = menus.submenu(&bar, "Help");
    menus.item(
        &help,
        Some(delegate),
        "MusicX Help",
        Some(sel!(showHelp:)),
        "",
    );
    menus.item(
        &help,
        Some(delegate),
        "Report an Issue…",
        Some(sel!(reportIssue:)),
        "",
    );
    app.setHelpMenu(Some(&help));
    menus.refresh();
    MENUS.with_borrow_mut(|current| *current = menus);
    app.setMainMenu(Some(&bar));
}

pub fn show_about() {
    let app = NSApplication::sharedApplication(MainThreadMarker::new().expect("main thread"));
    let name = NSString::from_str("MusicX");
    let version = NSString::from_str(&crate::updates::release_version());
    let options = NSDictionary::<NSString, AnyObject>::from_slices(
        // AppKit's option constants are immutable process-lifetime strings.
        &[unsafe { NSAboutPanelOptionApplicationName }, unsafe {
            NSAboutPanelOptionApplicationVersion
        }],
        &[&*name, &*version],
    );
    unsafe { app.orderFrontStandardAboutPanelWithOptions(&options) };
}

pub fn open_help(report_issue: bool) {
    let url = if report_issue {
        "https://github.com/popomore/MusicX/issues/new"
    } else {
        "https://github.com/popomore/MusicX#readme"
    };
    let url = NSURL::URLWithString(&NSString::from_str(url)).expect("valid help URL");
    NSWorkspace::sharedWorkspace().openURL(&url);
}

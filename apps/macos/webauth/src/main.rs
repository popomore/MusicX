//! Disposable, isolated browser login. Credentials travel only over the parent pipe.
use musicx_protocol::tr;
mod request;
use anyhow::{Context, Result};
use objc2::{
    MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained,
    runtime::ProtocolObject, sel,
};
use objc2_app_kit::*;
use objc2_foundation::*;
use objc2_web_kit::*;
use request::Request;
use std::{
    cell::RefCell,
    io::{Read, Write},
    ptr::NonNull,
    time::Instant,
};

struct State {
    request: Request,
    _window: Retained<NSWindow>,
    _web: Retained<WKWebView>,
    store: Retained<WKWebsiteDataStore>,
    _delegate: Retained<Delegate>,
    _timer: Retained<NSTimer>,
    started: Instant,
    polling: bool,
    parent: i32,
}
unsafe extern "C" {
    fn getppid() -> i32;
}
thread_local! {static STATE:RefCell<Option<State>>=const{RefCell::new(None)};}
fn navigation_allowed(action: &WKNavigationAction) -> bool {
    unsafe { action.request().URL() }.is_some_and(|url| {
        url.absoluteString()
            .is_some_and(|s| s.to_string() == "about:blank")
            || url.scheme().is_some_and(|s| s.to_string() == "https")
    })
}
fn finish(value: serde_json::Value) -> ! {
    // Never print cookies to stderr, logs, argv, URLs or temporary files.
    let mut out = std::io::stdout().lock();
    let _ = serde_json::to_writer(&mut out, &value);
    let _ = out.flush();
    std::process::exit(0)
}
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind=MainThreadOnly]
    #[name="MiniMusicWebLoginDelegate"]
    struct Delegate;
    unsafe impl NSObjectProtocol for Delegate {}
    impl Delegate {
        #[unsafe(method(tick:))]
        fn tick(&self,_:&NSTimer) { poll(); }
        #[unsafe(method(closeLogin:))]
        fn close_login(&self,_:Option<&NSObject>) { finish(serde_json::json!({"cancelled":true})); }
    }
    unsafe impl NSApplicationDelegate for Delegate {
        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn close_app(&self,_:&NSApplication)->bool {true}
        #[unsafe(method(applicationShouldTerminate:))]
        fn terminate(&self,_:&NSApplication)->NSApplicationTerminateReply {finish(serde_json::json!({"cancelled":true}));}
    }
    unsafe impl NSWindowDelegate for Delegate {
        #[unsafe(method(windowWillClose:))]
        fn closed(&self,_:&NSNotification){finish(serde_json::json!({"cancelled":true}));}
    }
    unsafe impl WKNavigationDelegate for Delegate {
        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        unsafe fn failed(&self,_:&WKWebView,_:Option<&WKNavigation>,error:&NSError){
            if error.code()!=-999 {finish(serde_json::json!({"error":tr!("登录页面暂时无法加载，请检查网络后重试")}));}
        }
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        unsafe fn navigation(&self,_:&WKWebView,action:&WKNavigationAction,decision:&block2::DynBlock<dyn Fn(WKNavigationActionPolicy)>) {
            let allowed=navigation_allowed(action);
            decision.call((if allowed{WKNavigationActionPolicy::Allow}else{WKNavigationActionPolicy::Cancel},));
        }
    }
    unsafe impl WKUIDelegate for Delegate {
        #[unsafe(method_id(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:))]
        unsafe fn popup(&self,web:&WKWebView,_:&WKWebViewConfiguration,action:&WKNavigationAction,_:&WKWindowFeatures)->Option<Retained<WKWebView>>{
            if navigation_allowed(action){unsafe{web.loadRequest(&action.request());}}None
        }
    }
);
fn poll() {
    let store = STATE.with_borrow_mut(|state| {
        let state = state.as_mut()?;
        if unsafe { getppid() } != state.parent {
            finish(serde_json::json!({"cancelled":true}));
        }
        if state.started.elapsed().as_secs() >= 300 {
            finish(serde_json::json!({"error":tr!("网页登录超时，请重试")}));
        }
        if state.polling {
            return None;
        }
        state.polling = true;
        Some(state.store.clone())
    });
    let Some(store) = store else {
        return;
    };
    let block = block2::RcBlock::new(move |cookies: NonNull<NSArray<NSHTTPCookie>>| {
        let cookies = unsafe { cookies.as_ref() };
        STATE.with_borrow_mut(|state| {
            let Some(state) = state else {
                return;
            };
            state.polling = false;
            let mut selected = std::collections::BTreeMap::new();
            for index in 0..cookies.count() {
                let cookie = cookies.objectAtIndex(index);
                let name = cookie.name().to_string();
                let value = cookie.value().to_string();
                if state
                    .request
                    .accepts_cookie(&cookie.domain().to_string(), &cookie.path().to_string())
                    && !value.is_empty()
                    && !name.contains([';', '=', '\r', '\n'])
                    && !value.contains([';', '\r', '\n'])
                    && cookie
                        .expiresDate()
                        .is_none_or(|d| d.timeIntervalSinceNow() > 0.0)
                {
                    selected.insert(name, value);
                }
            }
            if state
                .request
                .until
                .iter()
                .all(|name| selected.contains_key(name))
            {
                let cookie = selected
                    .into_iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join("; ");
                if cookie.len() > 16384 {
                    finish(serde_json::json!({"error":tr!("登录数据过大")}));
                }
                finish(serde_json::json!({"cookie":cookie}));
            }
        });
    });
    unsafe {
        store.httpCookieStore().getAllCookies(&block);
    }
}
fn run() -> Result<()> {
    use std::io::IsTerminal;
    anyhow::ensure!(
        !std::io::stdin().is_terminal() && !std::io::stdout().is_terminal(),
        tr!("网页登录仅接受播放器私有通道")
    );
    let mut input = String::new();
    std::io::stdin().take(16385).read_to_string(&mut input)?;
    anyhow::ensure!(input.len() <= 16384, tr!("登录请求过大"));
    let request: Request = serde_json::from_str(&input)?;
    request.validate()?;
    let mtm = MainThreadMarker::new().context(tr!("需要主线程"))?;
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    let delegate: Retained<Delegate> = unsafe { msg_send![Delegate::alloc(mtm), init] };
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    let bar = NSMenu::new(mtm);
    for (title, entries) in [
        (
            tr!("登录"),
            vec![
                (tr!("关闭登录"), sel!(closeLogin:), "w"),
                (tr!("退出登录窗口"), sel!(closeLogin:), "q"),
            ],
        ),
        (
            tr!("编辑"),
            vec![
                (tr!("剪切"), sel!(cut:), "x"),
                (tr!("复制"), sel!(copy:), "c"),
                (tr!("粘贴"), sel!(paste:), "v"),
                (tr!("全选"), sel!(selectAll:), "a"),
            ],
        ),
    ] {
        let holder = NSMenuItem::new(mtm);
        holder.setTitle(&NSString::from_str(title));
        let menu = NSMenu::new(mtm);
        for (name, action, key) in entries {
            let item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(name),
                    Some(action),
                    &NSString::from_str(key),
                )
            };
            if title == tr!("登录") {
                unsafe {
                    item.setTarget(Some(&delegate));
                }
            }
            menu.addItem(&item);
        }
        holder.setSubmenu(Some(&menu));
        bar.addItem(&holder);
    }
    app.setMainMenu(Some(&bar));
    // The provider's desktop site has a fixed-width sign-in layout.
    let desktop_width = 1040.0;
    let size = NSScreen::mainScreen(mtm).map_or(NSSize::new(desktop_width, 680.0), |screen| {
        let area = screen.visibleFrame().size;
        NSSize::new(
            (area.width - 48.0).clamp(320.0, desktop_width),
            (area.height - 72.0).clamp(360.0, 680.0),
        )
    });
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), size);
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            frame,
            NSWindowStyleMask::Titled
                | NSWindowStyleMask::Closable
                | NSWindowStyleMask::Miniaturizable
                | NSWindowStyleMask::Resizable,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    let host = url::Url::parse(&request.url)?
        .host_str()
        .unwrap_or_default()
        .to_owned();
    window.setTitle(&NSString::from_str(
        &tr!("登录{} · {host}", request.name.chars().take(60).collect::<String>(); host = host),
    ));
    window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    window.center();
    let store = unsafe { WKWebsiteDataStore::nonPersistentDataStore(mtm) };
    let config = unsafe { WKWebViewConfiguration::new(mtm) };
    unsafe {
        config.setWebsiteDataStore(&store);
    }
    let web =
        unsafe { WKWebView::initWithFrame_configuration(WKWebView::alloc(mtm), frame, &config) };
    unsafe {
        web.setNavigationDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        web.setUIDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        if size.width < desktop_width {
            web.setPageZoom(size.width / desktop_width);
        }
    }
    if let Some(agent) = &request.user_agent {
        unsafe {
            web.setCustomUserAgent(Some(&NSString::from_str(agent)));
        }
    }
    window.setContentView(Some(&web));
    let url =
        NSURL::URLWithString(&NSString::from_str(&request.url)).context(tr!("登录地址无效"))?;
    let load = NSURLRequest::requestWithURL(&url);
    unsafe {
        web.loadRequest(&load);
    }
    let timer = unsafe {
        NSTimer::timerWithTimeInterval_target_selector_userInfo_repeats(
            0.4,
            &delegate,
            sel!(tick:),
            None,
            true,
        )
    };
    unsafe {
        NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes);
    }
    STATE.with_borrow_mut(|s| {
        *s = Some(State {
            request,
            _window: window.clone(),
            _web: web,
            store,
            _delegate: delegate,
            _timer: timer,
            started: Instant::now(),
            polling: false,
            parent: unsafe { getppid() },
        })
    });
    window.makeKeyAndOrderFront(None);
    app.activate();
    app.run();
    Ok(())
}
fn main() {
    if run().is_err() {
        finish(serde_json::json!({"error":tr!("网页登录窗口暂时无法打开")}));
    }
}

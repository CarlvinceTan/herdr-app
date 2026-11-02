// HERDR: a real web page inside the window. On macOS a CEF browser sits in a view added over GPUI's
// own and placed at the rectangle layout gives this element, so it moves in the same frame as
// everything else. Views are kept by id across renders: a page lives until it's told to go, not
// until a mod stops drawing it. Elsewhere this draws nothing.
use gpui::{
    AnyElement, App, Bounds, Element, GlobalElementId, InspectorElementId, IntoElement, LayoutId,
    Pixels, Refineable as _, Style, StyleRefinement, Window,
};

pub(crate) struct WebView {
    id: String,
    refinement: StyleRefinement,
}

pub(crate) fn web_view(id: String, refinement: StyleRefinement) -> AnyElement {
    WebView { id, refinement }.into_any_element()
}

impl IntoElement for WebView {
    type Element = Self;
    fn into_element(self) -> Self { self }
}

impl Element for WebView {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> { None }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> { None }

    fn request_layout(
        &mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>,
        window: &mut Window, cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.refine(&self.refinement);
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>, _: &mut (), window: &mut Window, cx: &mut App,
    ) {
        native::place(window, &self.id, bounds, cx);
    }

    fn paint(
        &mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>,
        _: Bounds<Pixels>, _: &mut (), _: &mut (), _: &mut Window, _: &mut App,
    ) {
    }
}

pub use native::{command, on_click, pages, set_suspended, start};

#[cfg(not(target_os = "macos"))]
mod native {
    use super::*;
    pub fn place(_: &Window, _: &str, _: Bounds<Pixels>, _: &mut App) {}
    pub fn command(_: &str, _: &str, _: &str) {}
    pub fn on_click(_: impl Fn(String) + 'static) {}
    pub fn start(_: i32) -> bool { false }
    pub fn pages() -> Vec<(String, String, String, u64, bool)> { Vec::new() }
    pub fn set_suspended(_: bool) {}
}

#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)] // The objc macros check a feature of their own.
mod native {
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::ffi::{CStr, CString, c_void};
    use std::path::PathBuf;
    use std::sync::Once;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use cef::{
        App, Browser, BrowserSettings, CefString, Client, CommandLine, DictionaryValue,
        Frame, ImplApp, ImplBrowser, ImplBrowserHost, ImplClient, ImplCommandLine, ImplFrame, ImplNavigationEntry,
        ImplLifeSpanHandler, LifeSpanHandler, PopupFeatures, Rect as CefRect, RuntimeStyle, Settings,
        WindowInfo, WindowOpenDisposition, WrapApp, WrapClient, WrapLifeSpanHandler,
        rc::Rc as _, wrap_app, wrap_client, wrap_life_span_handler,
    };
    use gpui::{App as GpuiApp, Bounds, Pixels, Window};
    use objc::declare::ClassDecl;
    use objc::runtime::{BOOL, Class, NO, Object, Sel, YES};
    use objc::{Encode, Encoding, class, msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    type Id = *mut Object;
    const NIL: Id = std::ptr::null_mut();

    #[link(name = "QuartzCore", kind = "framework")]
    unsafe extern "C" {
        static kCAFillRuleEvenOdd: Id;
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGPathCreateMutable() -> *mut c_void;
        fn CGPathAddRect(path: *mut c_void, transform: *const c_void, rect: NsRect);
        fn CGPathAddRoundedRect(path: *mut c_void, transform: *const c_void, rect: NsRect, w: f64, h: f64);
        fn CGPathRelease(path: *mut c_void);
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default, PartialEq)]
    struct Point { x: f64, y: f64 }
    #[repr(C)]
    #[derive(Clone, Copy, Default, PartialEq)]
    struct Size { w: f64, h: f64 }
    #[repr(C)]
    #[derive(Clone, Copy, Default, PartialEq)]
    struct NsRect { origin: Point, size: Size }

    unsafe impl Encode for Point {
        fn encode() -> Encoding { unsafe { Encoding::from_str("{CGPoint=dd}") } }
    }
    unsafe impl Encode for Size {
        fn encode() -> Encoding { unsafe { Encoding::from_str("{CGSize=dd}") } }
    }
    unsafe impl Encode for NsRect {
        fn encode() -> Encoding { unsafe { Encoding::from_str("{CGRect={CGPoint=dd}{CGSize=dd}}") } }
    }

    // Web pages default to 16px text. This brings them to about Herdr's 13 to 14.
    const DEFAULT_ZOOM: f64 = 0.85;
    const ZOOMS: [f64; 11] = [0.5, 0.6, 0.7, 0.75, DEFAULT_ZOOM, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0];
    const CORNER_RADIUS: f64 = 9.0;
    const HOLE_RADIUS: f64 = 8.0;
    const LEFT_MOUSE_DOWN: u64 = 1;
    const RIGHT_MOUSE_DOWN: u64 = 3;
    const COMMAND_KEY: u64 = 1 << 20;
    const SHIFT_KEY: u64 = 1 << 17;
    const WIDTH_AND_HEIGHT: u64 = 2 | 16;
    const BOTTOM_LEFT_AND_RIGHT: u64 = 1 | 2;
    const PUMP_EVERY: Duration = Duration::from_millis(8);
    // Chrome counts zoom in powers of this.
    const ZOOM_BASE: f64 = 1.2;

    struct View {
        host: Id,
        browser: Option<Browser>,
        url: String,
        title: String,
        zoom: f64,
        shown: bool,
        // Top-left in the window, as layout gives it. AppKit counts from the bottom.
        top: f64,
        left: f64,
        asked: u64,
    }

    thread_local! {
        static VIEWS: RefCell<HashMap<String, View>> = RefCell::new(HashMap::new());
        static CLICK: RefCell<Option<Box<dyn Fn(String)>>> = const { RefCell::new(None) };
        static SUSPENDED: Cell<bool> = const { Cell::new(false) };
        static LAST_CLICK: Cell<f64> = const { Cell::new(0.0) };
        static LIBRARY: RefCell<Option<cef::library_loader::LibraryLoader>> = const { RefCell::new(None) };
        static STARTED: Cell<bool> = const { Cell::new(false) };
        static PUMPING: Cell<bool> = const { Cell::new(false) };
        // Addresses sent before their page's view has drawn.
        static WAITING: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    }

    fn state_dir() -> PathBuf {
        std::env::var("XDG_STATE_HOME").map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/state"))
            .join("herdr-browser")
    }

    fn zoom_level(factor: f64) -> f64 {
        factor.ln() / ZOOM_BASE.ln()
    }

    wrap_app! {
        struct HerdrCefApp;

        impl App {
            // Extensions come in as they would for Chromium itself.
            fn on_before_command_line_processing(&self, process_type: Option<&CefString>, command_line: Option<&mut CommandLine>) {
                let is_browser = process_type.is_none_or(|kind| kind.to_string().is_empty());
                let (true, Some(line)) = (is_browser, command_line) else { return };
                // Pages can't post to macOS here, so its notifications helper would sit idle.
                line.append_switch(Some(&CefString::from("disable-notifications")));
                line.append_switch_with_value(Some(&CefString::from("disable-features")), Some(&CefString::from("NativeNotifications,SystemNotifications")));
                let ublock = state_dir().join("ublock");
                if ublock.join("manifest.json").exists() {
                    let path = CefString::from(ublock.to_string_lossy().as_ref());
                    line.append_switch_with_value(Some(&CefString::from("load-extension")), Some(&path));
                    line.append_switch_with_value(Some(&CefString::from("disable-extensions-except")), Some(&path));
                }
            }
        }
    }

    wrap_client! {
        struct PageClient;

        impl Client {
            fn life_span_handler(&self) -> Option<LifeSpanHandler> {
                Some(PagePopups::new())
            }
        }
    }

    wrap_life_span_handler! {
        struct PagePopups;

        impl LifeSpanHandler {
            // Links that ask for a new window open in this one.
            #[allow(clippy::too_many_arguments)]
            fn on_before_popup(
                &self,
                browser: Option<&mut Browser>,
                _frame: Option<&mut Frame>,
                _popup_id: ::std::os::raw::c_int,
                target_url: Option<&CefString>,
                _target_frame_name: Option<&CefString>,
                _target_disposition: WindowOpenDisposition,
                _user_gesture: ::std::os::raw::c_int,
                _popup_features: Option<&PopupFeatures>,
                _window_info: Option<&mut WindowInfo>,
                _client: Option<&mut Option<Client>>,
                _settings: Option<&mut BrowserSettings>,
                _extra_info: Option<&mut Option<DictionaryValue>>,
                _no_javascript_access: Option<&mut ::std::os::raw::c_int>,
            ) -> ::std::os::raw::c_int {
                if let Some(frame) = browser.and_then(|browser| browser.main_frame()) {
                    frame.load_url(target_url);
                }
                1
            }
        }
    }

    /// Starts CEF, as early in a launch as can be. Its message pump counts the run loop's passes and
    /// crashes leaving one it never saw begin, and it takes over malloc for the whole process, which
    /// isn't safe once other threads run. `debug_port` opens Chrome's DevTools protocol to programs on
    /// this computer, for agents; 0 leaves it closed.
    pub fn start(debug_port: i32) -> bool {
        if STARTED.get() {
            return true;
        }
        {
            // Chromium needs to find the app it can talk to, not a plain one it would make itself.
            let _: Id = unsafe { msg_send![class!(GPUIApplication), sharedApplication] };
            let Ok(exe) = std::env::current_exe() else { return false };
            let framework = exe.parent().and_then(|dir| dir.parent()).map(|contents| contents.join("Frameworks/Chromium Embedded Framework.framework"));
            if !framework.is_some_and(|framework| framework.exists()) {
                tracing::warn!("web view: this build wasn't bundled with Chromium");
                return false;
            }
            let loader = cef::library_loader::LibraryLoader::new(&exe, false);
            if !loader.load() {
                tracing::error!("web view: no Chromium Embedded Framework beside {exe:?}");
                return false;
            }
            LIBRARY.with_borrow_mut(|library| *library = Some(loader));
            let _ = cef::api_hash(cef::sys::CEF_API_VERSION_LAST, 0);
            let cache = state_dir().join("cef");
            let cache = CefString::from(cache.to_string_lossy().as_ref());
            let settings = Settings {
                no_sandbox: 1,
                external_message_pump: 1,
                persist_session_cookies: 1,
                remote_debugging_port: debug_port,
                root_cache_path: cache.clone(),
                cache_path: cache,
                ..Default::default()
            };
            let args = cef::args::Args::new();
            let mut app = HerdrCefApp::new();
            if cef::initialize(Some(args.as_main_args()), Some(&settings), Some(&mut app), std::ptr::null_mut()) != 1 {
                tracing::error!("web view: CEF wouldn't start");
                return false;
            }
            STARTED.set(true);
            unsafe {
                let app: Id = msg_send![class!(NSApplication), sharedApplication];
                let _: () = msg_send![app, setBeforeTerminate: finish as extern "C" fn() as *mut c_void];
            }
            true
        }
    }

    // CEF's work runs on the app's own loop, from the first page on.
    fn pump(cx: &mut GpuiApp) {
        if PUMPING.replace(true) {
            return;
        }
        let timer = cx.background_executor().clone();
        cx.foreground_executor().spawn(async move {
            loop {
                timer.timer(PUMP_EVERY).await;
                if STARTED.get() {
                    cef::do_message_loop_work();
                }
            }
        }).detach();
    }

    fn ns_string(text: &str) -> Id {
        let text = CString::new(text.replace('\0', "")).unwrap_or_default();
        unsafe { msg_send![class!(NSString), stringWithUTF8String: text.as_ptr()] }
    }

    fn rust_string(text: Id) -> String {
        if text.is_null() {
            return String::new();
        }
        unsafe {
            let bytes: *const std::os::raw::c_char = msg_send![text, UTF8String];
            if bytes.is_null() { String::new() } else { CStr::from_ptr(bytes).to_string_lossy().into_owned() }
        }
    }

    fn key_of(host: Id) -> Option<String> {
        VIEWS.with_borrow(|views| views.iter().find(|(_, view)| view.host == host).map(|(id, _)| id.clone()))
    }

    // While a popup has a hole cut for it, clicks pass through to the window beneath.
    extern "C" fn host_hit_test(this: &mut Object, _: Sel, point: Point) -> Id {
        unsafe {
            let pass: BOOL = *this.get_ivar("pass");
            if pass != NO {
                return NIL;
            }
            let found: Id = msg_send![super(this, class!(NSView)), hitTest: point];
            if !found.is_null() {
                report_click(this as *mut Object);
            }
            found
        }
    }

    // Herdr can't see a press that lands on a page, so it's told, and the pane takes the focus.
    unsafe fn report_click(host: Id) {
        unsafe {
            let app: Id = msg_send![class!(NSApplication), sharedApplication];
            let event: Id = msg_send![app, currentEvent];
            if event.is_null() {
                return;
            }
            let kind: u64 = msg_send![event, type];
            let time: f64 = msg_send![event, timestamp];
            if (kind != LEFT_MOUSE_DOWN && kind != RIGHT_MOUSE_DOWN) || LAST_CLICK.get() == time {
                return;
            }
            LAST_CLICK.set(time);
            if let Some(id) = key_of(host) {
                CLICK.with_borrow(|click| {
                    if let Some(click) = click {
                        click(id);
                    }
                });
            }
        }
    }

    // A window without an Edit menu gets none of the usual shortcuts.
    extern "C" fn host_key_equivalent(this: &mut Object, _: Sel, event: Id) -> BOOL {
        unsafe {
            let host: Id = this;
            let window: Id = msg_send![host, window];
            let responder: Id = msg_send![window, firstResponder];
            let mine: BOOL = if responder.is_null() {
                NO
            } else {
                let is_view: BOOL = msg_send![responder, isKindOfClass: class!(NSView)];
                if is_view == NO { NO } else { msg_send![responder, isDescendantOf: host] }
            };
            let flags: u64 = msg_send![event, modifierFlags];
            if mine != NO && flags & COMMAND_KEY != 0 {
                let keys: Id = msg_send![event, charactersIgnoringModifiers];
                let key = rust_string(keys).to_lowercase();
                if shortcut(host, &key, flags & SHIFT_KEY != 0) {
                    return YES;
                }
            }
            msg_send![super(this, class!(NSView)), performKeyEquivalent: event]
        }
    }

    fn shortcut(host: Id, key: &str, shift: bool) -> bool {
        VIEWS.with_borrow_mut(|views| {
            let Some(view) = views.values_mut().find(|view| view.host == host) else { return false };
            let Some(browser) = view.browser.clone() else { return false };
            let frame = browser.focused_frame().or_else(|| browser.main_frame());
            match (key, frame) {
                ("a", Some(frame)) => frame.select_all(),
                ("c", Some(frame)) => frame.copy(),
                ("v", Some(frame)) => frame.paste(),
                ("x", Some(frame)) => frame.cut(),
                ("z", Some(frame)) if shift => frame.redo(),
                ("z", Some(frame)) => frame.undo(),
                ("r", _) => browser.reload(),
                ("[", _) => browser.go_back(),
                ("]", _) => browser.go_forward(),
                ("l", _) => view.asked = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |t| t.as_millis() as u64),
                _ => return false,
            }
            true
        })
    }

    fn host_class() -> &'static Class {
        static REGISTER: Once = Once::new();
        REGISTER.call_once(|| unsafe {
            let mut host = ClassDecl::new("HerdrWebHost", class!(NSView)).expect("HerdrWebHost");
            host.add_ivar::<BOOL>("pass");
            host.add_method(sel!(hitTest:), host_hit_test as extern "C" fn(&mut Object, Sel, Point) -> Id);
            host.add_method(sel!(performKeyEquivalent:), host_key_equivalent as extern "C" fn(&mut Object, Sel, Id) -> BOOL);
            host.register();
        });
        class!(HerdrWebHost)
    }

    unsafe fn new_host(parent: Id) -> Id {
        unsafe {
            let host: Id = msg_send![host_class(), alloc];
            let host: Id = msg_send![host, initWithFrame: NsRect::default()];
            let _: () = msg_send![host, setWantsLayer: YES];
            let layer: Id = msg_send![host, layer];
            // Square under the header, rounded where the pane's bottom corners are.
            let _: () = msg_send![layer, setCornerRadius: CORNER_RADIUS];
            let _: () = msg_send![layer, setMaskedCorners: BOTTOM_LEFT_AND_RIGHT];
            let _: () = msg_send![layer, setMasksToBounds: YES];
            let _: () = msg_send![parent, addSubview: host];
            host
        }
    }

    fn create_browser(host: Id, url: &str, width: i32, height: i32) -> Option<Browser> {
        let bounds = CefRect { x: 0, y: 0, width, height };
        let info = WindowInfo { runtime_style: RuntimeStyle::ALLOY, ..Default::default() }
            .set_as_child(host as cef::sys::cef_window_handle_t, &bounds);
        let mut client = PageClient::new();
        let browser = cef::browser_host_create_browser_sync(
            Some(&info), Some(&mut client), Some(&CefString::from(url)),
            Some(&BrowserSettings::default()), None, None,
        )?;
        if let Some(browser_host) = browser.host() {
            let view = browser_host.window_handle() as Id;
            unsafe {
                let _: () = msg_send![view, setAutoresizingMask: WIDTH_AND_HEIGHT];
            }
        }
        Some(browser)
    }

    fn apply_zoom(view: &View) {
        if let Some(host) = view.browser.as_ref().and_then(|browser| browser.host()) {
            host.set_zoom_level(zoom_level(view.zoom));
        }
    }

    // A closing page takes the window it's in along with it, so it leaves the app's window first. It
    // only finishes closing once its own view is released, which leaving the host does.
    fn close_page(view: &View) {
        unsafe {
            let _: () = msg_send![view.host, removeFromSuperview];
        }
        if let Some(host) = view.browser.as_ref().and_then(|browser| browser.host()) {
            host.close_browser(1);
        }
        unsafe {
            // Reading the list holds its views until this pool ends, and at quit nothing else would.
            let pool: Id = msg_send![class!(NSAutoreleasePool), new];
            let pages: Id = msg_send![view.host, subviews];
            let count: usize = msg_send![pages, count];
            for i in 0..count {
                let page: Id = msg_send![pages, objectAtIndex: i];
                let _: () = msg_send![page, removeFromSuperview];
            }
            let _: () = msg_send![pool, drain];
        }
    }

    // Closes every page and lets Chromium write what it holds, so a sign-in just before quitting stays.
    extern "C" fn finish() {
        STARTED.set(false);
        VIEWS.take().values().for_each(close_page);
        cef::shutdown();
    }

    pub fn place(window: &Window, id: &str, bounds: Bounds<Pixels>, cx: &mut GpuiApp) {
        if !STARTED.get() {
            return;
        }
        pump(cx);
        let Ok(handle) = HasWindowHandle::window_handle(window) else { return };
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else { return };
        let gpui_view = handle.ns_view.as_ptr() as Id;
        let (left, top) = (f64::from(f32::from(bounds.origin.x)), f64::from(f32::from(bounds.origin.y)));
        let (width, height) = (f64::from(f32::from(bounds.size.width)), f64::from(f32::from(bounds.size.height)));
        let (space, parent) = unsafe {
            let parent: Id = msg_send![gpui_view, superview];
            let space: NsRect = msg_send![parent, bounds];
            (space, parent)
        };
        let created = VIEWS.with_borrow_mut(|views| {
            let view = views.entry(id.to_owned()).or_insert_with(|| View {
                host: unsafe { new_host(parent) },
                browser: None,
                url: String::new(),
                title: String::new(),
                zoom: DEFAULT_ZOOM,
                shown: true,
                top: 0.0,
                left: 0.0,
                asked: 0,
            });
            view.top = top;
            view.left = left;
            let frame = NsRect { origin: Point { x: left, y: space.size.h - top - height }, size: Size { w: width, h: height } };
            unsafe {
                let _: () = msg_send![view.host, setFrame: frame];
            }
            if view.browser.is_none() && width > 0.0 && height > 0.0 { Some(view.host) } else { None }
        });
        // Creating a browser runs the message loop, which reaches back into VIEWS.
        if let Some(host) = created {
            let url = WAITING.with_borrow_mut(|waiting| waiting.remove(id)).unwrap_or_else(|| "about:blank".to_owned());
            let browser = create_browser(host, &url, width as i32, height as i32);
            VIEWS.with_borrow_mut(|views| {
                if let Some(view) = views.get_mut(id) {
                    view.browser = browser;
                    apply_zoom(view);
                }
            });
        }
        apply_visibility();
    }

    fn apply_visibility() {
        let suspended = SUSPENDED.get();
        VIEWS.with_borrow(|views| {
            for view in views.values() {
                let hidden: BOOL = if view.shown && !suspended { NO } else { YES };
                unsafe {
                    let _: () = msg_send![view.host, setHidden: hidden];
                }
            }
        });
    }

    /// Dialogs dim the whole window from beneath this view, so pages step aside for them.
    pub fn set_suspended(suspended: bool) {
        if SUSPENDED.replace(suspended) != suspended {
            apply_visibility();
        }
    }

    pub fn on_click(callback: impl Fn(String) + 'static) {
        CLICK.with_borrow_mut(|click| *click = Some(Box::new(callback)));
    }

    // A browser's own record of what's on show. Its display callbacks don't arrive for a page in a
    // view parented like this. Zoom goes with the site, so a new address gets it again.
    fn refresh(view: &mut View) {
        let Some(entry) = view.browser.as_ref().and_then(|browser| browser.host()).and_then(|host| host.visible_navigation_entry()) else { return };
        let url = CefString::from(&entry.url()).to_string();
        view.title = CefString::from(&entry.title()).to_string();
        if url != view.url {
            view.url = url;
            apply_zoom(view);
        }
    }

    /// Each page's id, address, title, when it last asked for the address bar, and if it's on show.
    pub fn pages() -> Vec<(String, String, String, u64, bool)> {
        let suspended = SUSPENDED.get();
        VIEWS.with_borrow_mut(|views| {
            views.iter_mut().map(|(id, view)| {
                refresh(view);
                (id.clone(), view.url.clone(), view.title.clone(), view.asked, view.shown && !suspended)
            }).collect()
        })
    }

    fn normalize(text: &str) -> String {
        let text = text.trim();
        let lower = text.to_lowercase();
        if lower.starts_with("localhost") || lower.starts_with("127.0.0.1") {
            return format!("http://{text}");
        }
        let scheme = text.split_once(':').is_some_and(|(head, _)| {
            head.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
                && head.chars().all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
        });
        if scheme {
            return text.to_owned();
        }
        if !text.contains(' ') && text.contains('.') {
            return format!("https://{text}");
        }
        let query: String = text.bytes().map(|byte| match byte {
            b' ' => "+".to_owned(),
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (byte as char).to_string(),
            _ => format!("%{byte:02X}"),
        }).collect();
        format!("https://www.google.com/search?q={query}")
    }

    fn zoom(view: &mut View, how: &str) {
        let at = ZOOMS.iter().position(|z| (*z - view.zoom).abs() < 0.001).unwrap_or(4);
        view.zoom = match how {
            "in" => ZOOMS[(at + 1).min(ZOOMS.len() - 1)],
            "out" => ZOOMS[at.saturating_sub(1)],
            _ => DEFAULT_ZOOM,
        };
        apply_zoom(view);
    }

    fn clear_data(view: &View) {
        let Some(browser) = view.browser.as_ref() else { return };
        if let Some(host) = browser.host() {
            for method in ["Network.clearBrowserCookies", "Network.clearBrowserCache"] {
                host.execute_dev_tools_method(0, Some(&CefString::from(method)), None);
            }
        }
        browser.reload();
    }

    // Popups draw beneath this view, so while one is open it cuts a hole for it.
    unsafe fn set_hole(view: &View, area: &str) {
        unsafe {
            let parts: Vec<f64> = area.split_whitespace().filter_map(|part| part.parse().ok()).collect();
            let active = parts.len() == 4;
            let host = &mut *view.host;
            host.set_ivar::<BOOL>("pass", if active { YES } else { NO });
            let layer: Id = msg_send![view.host, layer];
            if !active || parts[2] <= 0.0 || parts[3] <= 0.0 {
                let _: () = msg_send![layer, setMask: NIL];
                return;
            }
            let bounds: NsRect = msg_send![view.host, bounds];
            let hole = NsRect {
                origin: Point { x: parts[0] - view.left, y: bounds.size.h - (parts[1] - view.top) - parts[3] },
                size: Size { w: parts[2], h: parts[3] },
            };
            let path = CGPathCreateMutable();
            CGPathAddRect(path, std::ptr::null(), bounds);
            CGPathAddRoundedRect(path, std::ptr::null(), hole, HOLE_RADIUS, HOLE_RADIUS);
            let mask: Id = msg_send![class!(CAShapeLayer), layer];
            let _: () = msg_send![mask, setPath: path];
            let _: () = msg_send![mask, setFillRule: kCAFillRuleEvenOdd];
            let _: () = msg_send![layer, setMask: mask];
            CGPathRelease(path);
        }
    }

    pub fn command(id: &str, op: &str, arg: &str) {
        if op == "remove" {
            if let Some(view) = VIEWS.with_borrow_mut(|views| views.remove(id)) {
                close_page(&view);
            }
            return;
        }
        let exists = VIEWS.with_borrow(|views| views.get(id).is_some_and(|view| view.browser.is_some()));
        if op == "navigate" && !exists {
            WAITING.with_borrow_mut(|waiting| waiting.insert(id.to_owned(), normalize(arg)));
            return;
        }
        let browser = VIEWS.with_borrow_mut(|views| {
            let view = views.get_mut(id)?;
            match op {
                "zoom" => zoom(view, arg),
                "clear-data" => clear_data(view),
                "hole" => unsafe { set_hole(view, arg) },
                "show" => view.shown = true,
                "hide" => view.shown = false,
                _ => {}
            }
            view.browser.clone()
        });
        if let Some(browser) = browser {
            match op {
                "navigate" => {
                    if let Some(frame) = browser.main_frame() {
                        frame.load_url(Some(&CefString::from(normalize(arg).as_str())));
                    }
                }
                "back" => browser.go_back(),
                "forward" => browser.go_forward(),
                "reload" => browser.reload(),
                "zoom" | "clear-data" | "hole" | "show" | "hide" => {}
                other => tracing::warn!("web view: unknown command {other:?}"),
            }
        }
        apply_visibility();
    }
}

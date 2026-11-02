// HERDR: a real web page inside the window. On macOS a WKWebView is added over GPUI's own view and
// placed at the rectangle layout gives this element, so it moves in the same frame as everything
// else. Views are kept by id across renders: a page lives until it's told to go, not until a mod
// stops drawing it. Elsewhere this draws nothing.
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
        bounds: Bounds<Pixels>, _: &mut (), window: &mut Window, _: &mut App,
    ) {
        native::place(window, &self.id, bounds);
    }

    fn paint(
        &mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>,
        _: Bounds<Pixels>, _: &mut (), _: &mut (), _: &mut Window, _: &mut App,
    ) {
    }
}

pub use native::{command, on_click, pages, set_suspended};

#[cfg(not(target_os = "macos"))]
mod native {
    use super::*;
    pub fn place(_: &Window, _: &str, _: Bounds<Pixels>) {}
    pub fn command(_: &str, _: &str, _: &str) {}
    pub fn on_click(_: impl Fn(String) + 'static) {}
    pub fn pages() -> Vec<(String, String, String, u64)> { Vec::new() }
    pub fn set_suspended(_: bool) {}
}

#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)] // The objc macros check a feature of their own.
mod native {
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::ffi::{CStr, CString, c_void};
    use std::sync::Once;
    use std::time::{SystemTime, UNIX_EPOCH};

    use block::ConcreteBlock;
    use gpui::{Bounds, Pixels, Window};
    use objc::declare::ClassDecl;
    use objc::runtime::{BOOL, Class, NO, Object, Sel, YES};
    use objc::{Encode, Encoding, class, msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    type Id = *mut Object;
    const NIL: Id = std::ptr::null_mut();

    #[link(name = "WebKit", kind = "framework")]
    unsafe extern "C" {}
    #[link(name = "QuartzCore", kind = "framework")]
    unsafe extern "C" {
        static kCAFillRuleEvenOdd: Id;
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGPathCreateMutable() -> *mut c_void;
        fn CGPathAddRect(path: *mut c_void, transform: *const c_void, rect: Rect);
        fn CGPathAddRoundedRect(path: *mut c_void, transform: *const c_void, rect: Rect, w: f64, h: f64);
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
    struct Rect { origin: Point, size: Size }

    unsafe impl Encode for Point {
        fn encode() -> Encoding { unsafe { Encoding::from_str("{CGPoint=dd}") } }
    }
    unsafe impl Encode for Size {
        fn encode() -> Encoding { unsafe { Encoding::from_str("{CGSize=dd}") } }
    }
    unsafe impl Encode for Rect {
        fn encode() -> Encoding { unsafe { Encoding::from_str("{CGRect={CGPoint=dd}{CGSize=dd}}") } }
    }

    // Web pages default to 16px text. This brings them to about Herdr's 13 to 14.
    const DEFAULT_ZOOM: f64 = 0.85;
    const ZOOMS: [f64; 11] = [0.5, 0.6, 0.7, 0.75, DEFAULT_ZOOM, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0];
    const USER_AGENT: &str = "Version/26.0 Safari/605.1.15";
    const CORNER_RADIUS: f64 = 9.0;
    const HOLE_RADIUS: f64 = 8.0;
    const LEFT_MOUSE_DOWN: u64 = 1;
    const RIGHT_MOUSE_DOWN: u64 = 3;
    const COMMAND_KEY: u64 = 1 << 20;
    const SHIFT_KEY: u64 = 1 << 17;
    const WIDTH_AND_HEIGHT: u64 = 2 | 16;
    const BOTTOM_LEFT_AND_RIGHT: u64 = 1 | 2;

    struct View {
        host: Id,
        web: Id,
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

    fn key_of(view: Id) -> Option<String> {
        VIEWS.with_borrow(|views| views.iter().find(|(_, v)| v.host == view || v.web == view).map(|(id, _)| id.clone()))
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
    extern "C" fn web_key_equivalent(this: &mut Object, _: Sel, event: Id) -> BOOL {
        unsafe {
            let this_id: Id = this;
            let window: Id = msg_send![this_id, window];
            let responder: Id = msg_send![window, firstResponder];
            let mine: BOOL = if responder.is_null() {
                NO
            } else {
                let is_view: BOOL = msg_send![responder, isKindOfClass: class!(NSView)];
                if is_view == NO { NO } else { msg_send![responder, isDescendantOf: this_id] }
            };
            let flags: u64 = msg_send![event, modifierFlags];
            if mine != NO && flags & COMMAND_KEY != 0 {
                let keys: Id = msg_send![event, charactersIgnoringModifiers];
                let key = rust_string(keys).to_lowercase();
                let edit = match key.as_str() {
                    "a" => Some(sel!(selectAll:)),
                    "c" => Some(sel!(copy:)),
                    "v" => Some(sel!(paste:)),
                    "x" => Some(sel!(cut:)),
                    "z" if flags & SHIFT_KEY != 0 => Some(sel!(redo:)),
                    "z" => Some(sel!(undo:)),
                    _ => None,
                };
                if let Some(action) = edit {
                    let app: Id = msg_send![class!(NSApplication), sharedApplication];
                    let _: BOOL = msg_send![app, sendAction: action to: NIL from: this_id];
                    return YES;
                }
                match key.as_str() {
                    "r" => { let _: Id = msg_send![this_id, reload]; return YES; }
                    "[" => { let _: Id = msg_send![this_id, goBack]; return YES; }
                    "]" => { let _: Id = msg_send![this_id, goForward]; return YES; }
                    "l" => {
                        let asked = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |t| t.as_millis() as u64);
                        VIEWS.with_borrow_mut(|views| {
                            if let Some(view) = views.values_mut().find(|v| v.web == this_id) {
                                view.asked = asked;
                            }
                        });
                        return YES;
                    }
                    _ => {}
                }
            }
            msg_send![super(this, class!(WKWebView)), performKeyEquivalent: event]
        }
    }

    // Links that ask for a new window open in this one.
    extern "C" fn create_web_view(_: &mut Object, _: Sel, web: Id, _: Id, action: Id, _: Id) -> Id {
        unsafe {
            let frame: Id = msg_send![action, targetFrame];
            if frame.is_null() {
                let request: Id = msg_send![action, request];
                let _: Id = msg_send![web, loadRequest: request];
            }
            NIL
        }
    }

    fn classes() -> (&'static Class, &'static Class, &'static Class) {
        static REGISTER: Once = Once::new();
        REGISTER.call_once(|| unsafe {
            let mut host = ClassDecl::new("HerdrWebHost", class!(NSView)).expect("HerdrWebHost");
            host.add_ivar::<BOOL>("pass");
            host.add_method(sel!(hitTest:), host_hit_test as extern "C" fn(&mut Object, Sel, Point) -> Id);
            host.register();

            let mut web = ClassDecl::new("HerdrWebView", class!(WKWebView)).expect("HerdrWebView");
            web.add_method(sel!(performKeyEquivalent:), web_key_equivalent as extern "C" fn(&mut Object, Sel, Id) -> BOOL);
            web.register();

            let mut delegate = ClassDecl::new("HerdrWebDelegate", class!(NSObject)).expect("HerdrWebDelegate");
            delegate.add_method(
                sel!(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:),
                create_web_view as extern "C" fn(&mut Object, Sel, Id, Id, Id, Id) -> Id,
            );
            delegate.register();
        });
        (class!(HerdrWebHost), class!(HerdrWebView), class!(HerdrWebDelegate))
    }

    unsafe fn create(parent: Id) -> View {
        unsafe {
            let (host_class, web_class, delegate_class) = classes();
            let frame = Rect::default();
            let host: Id = msg_send![host_class, alloc];
            let host: Id = msg_send![host, initWithFrame: frame];
            let _: () = msg_send![host, setWantsLayer: YES];
            let layer: Id = msg_send![host, layer];
            // Square under the header, rounded where the pane's bottom corners are.
            let _: () = msg_send![layer, setCornerRadius: CORNER_RADIUS];
            let _: () = msg_send![layer, setMaskedCorners: BOTTOM_LEFT_AND_RIGHT];
            let _: () = msg_send![layer, setMasksToBounds: YES];

            let configuration: Id = msg_send![class!(WKWebViewConfiguration), new];
            let _: () = msg_send![configuration, setApplicationNameForUserAgent: ns_string(USER_AGENT)];
            let web: Id = msg_send![web_class, alloc];
            let web: Id = msg_send![web, initWithFrame: frame configuration: configuration];
            let delegate: Id = msg_send![delegate_class, new];
            let _: () = msg_send![web, setUIDelegate: delegate];
            let _: () = msg_send![web, setAutoresizingMask: WIDTH_AND_HEIGHT];
            let _: () = msg_send![web, setAllowsBackForwardNavigationGestures: YES];
            let _: () = msg_send![web, setPageZoom: DEFAULT_ZOOM];
            let _: () = msg_send![host, addSubview: web];
            let _: () = msg_send![parent, addSubview: host];
            View { host, web, shown: true, top: 0.0, left: 0.0, asked: 0 }
        }
    }

    pub fn place(window: &Window, id: &str, bounds: Bounds<Pixels>) {
        let Ok(handle) = HasWindowHandle::window_handle(window) else { return };
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else { return };
        let gpui_view = handle.ns_view.as_ptr() as Id;
        let (left, top) = (f64::from(f32::from(bounds.origin.x)), f64::from(f32::from(bounds.origin.y)));
        let (width, height) = (f64::from(f32::from(bounds.size.width)), f64::from(f32::from(bounds.size.height)));
        unsafe {
            let parent: Id = msg_send![gpui_view, superview];
            let space: Rect = msg_send![parent, bounds];
            VIEWS.with_borrow_mut(|views| {
                let view = views.entry(id.to_owned()).or_insert_with(|| create(parent));
                view.top = top;
                view.left = left;
                let frame = Rect { origin: Point { x: left, y: space.size.h - top - height }, size: Size { w: width, h: height } };
                let _: () = msg_send![view.host, setFrame: frame];
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

    /// Each page's id, address, title and when it last asked for the address bar.
    pub fn pages() -> Vec<(String, String, String, u64)> {
        VIEWS.with_borrow(|views| {
            views.iter().map(|(id, view)| unsafe {
                let url: Id = msg_send![view.web, URL];
                let text: Id = if url.is_null() { NIL } else { msg_send![url, absoluteString] };
                let title: Id = msg_send![view.web, title];
                (id.clone(), rust_string(text), rust_string(title), view.asked)
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

    unsafe fn navigate(web: Id, text: &str) {
        unsafe {
            let url: Id = msg_send![class!(NSURL), URLWithString: ns_string(&normalize(text))];
            if !url.is_null() {
                let request: Id = msg_send![class!(NSURLRequest), requestWithURL: url];
                let _: Id = msg_send![web, loadRequest: request];
            }
        }
    }

    unsafe fn zoom(web: Id, how: &str) {
        unsafe {
            let current: f64 = msg_send![web, pageZoom];
            let at = ZOOMS.iter().position(|z| (*z - current).abs() < 0.001).unwrap_or(4);
            let next = match how {
                "in" => ZOOMS[(at + 1).min(ZOOMS.len() - 1)],
                "out" => ZOOMS[at.saturating_sub(1)],
                _ => DEFAULT_ZOOM,
            };
            let _: () = msg_send![web, setPageZoom: next];
        }
    }

    unsafe fn clear_data(web: Id) {
        unsafe {
            let store: Id = msg_send![class!(WKWebsiteDataStore), defaultDataStore];
            let kinds: Id = msg_send![class!(WKWebsiteDataStore), allWebsiteDataTypes];
            let since: Id = msg_send![class!(NSDate), distantPast];
            let done = ConcreteBlock::new(move || {
                let _: Id = msg_send![web, reload];
            }).copy();
            let _: () = msg_send![store, removeDataOfTypes: kinds modifiedSince: since completionHandler: &*done];
        }
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
            let bounds: Rect = msg_send![view.host, bounds];
            let hole = Rect {
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
                unsafe {
                    let _: () = msg_send![view.host, removeFromSuperview];
                }
            }
            return;
        }
        VIEWS.with_borrow_mut(|views| {
            let Some(view) = views.get_mut(id) else { return };
            unsafe {
                match op {
                    "navigate" => navigate(view.web, arg),
                    "back" => { let _: Id = msg_send![view.web, goBack]; }
                    "forward" => { let _: Id = msg_send![view.web, goForward]; }
                    "reload" => { let _: Id = msg_send![view.web, reload]; }
                    "zoom" => zoom(view.web, arg),
                    "clear-data" => clear_data(view.web),
                    "hole" => set_hole(view, arg),
                    "show" => view.shown = true,
                    "hide" => view.shown = false,
                    other => tracing::warn!("web view: unknown command {other:?}"),
                }
            }
        });
        apply_visibility();
    }
}

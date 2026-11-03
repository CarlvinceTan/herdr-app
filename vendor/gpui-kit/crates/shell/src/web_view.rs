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

pub use native::{begin_covers, command, cover, google_account, holes, ids, is_private, order_back, normal_url, on_click, pages, preload, set_background, set_suspended, set_undrawn, start};

#[cfg(not(target_os = "macos"))]
mod native {
    use super::*;
    pub fn place(_: &Window, _: &str, _: Bounds<Pixels>, _: &mut App) {}
    pub fn command(_: &str, _: &str, _: &str) {}
    pub fn on_click(_: impl Fn(String) + 'static) {}
    pub fn start(_: i32) -> bool { false }
    pub fn pages() -> Vec<(String, String, String, u64, bool)> { Vec::new() }
    pub fn ids() -> Vec<String> { Vec::new() }
    pub fn order_back(_: &Window) {}
    pub fn set_suspended(_: bool) {}
    pub fn set_undrawn(_: std::collections::HashSet<String>) {}
    pub fn begin_covers(_: bool) {}
    pub fn cover(_: Bounds<Pixels>, _: bool) {}
    pub fn holes(_: &str) -> usize { 0 }
    pub fn is_private(_: &str) -> bool { false }
    pub fn normal_url(_: &str) -> Option<String> { None }
    pub fn google_account() -> serde_json::Value { serde_json::Value::Null }
    pub fn preload(_: &Window, _: &str, _: &str) -> bool { false }
    pub fn set_background(_: u32) {}
}

#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)] // The objc macros check a feature of their own.
mod native {
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::ffi::{CStr, CString, c_void};
    use std::path::PathBuf;
    use std::sync::Once;
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use cef::{
        App, Browser, BrowserProcessHandler, BrowserSettings, CefString, Client, CommandLine, DictionaryValue,
        Frame, ImplApp, ImplBrowser, ImplBrowserHost, ImplBrowserProcessHandler, ImplClient, ImplCommandLine, ImplCookieManager, ImplFrame, ImplNavigationEntry,
        ImplLifeSpanHandler, ImplMediaAccessCallback, ImplPermissionHandler, ImplPostData, ImplPostDataElement, ImplRequest, ImplUrlrequestClient,
        LifeSpanHandler, MediaAccessCallback, PermissionHandler, PopupFeatures, Rect as CefRect,
        RequestContext, RequestContextSettings, RuntimeStyle, Settings, Urlrequest, UrlrequestClient,
        WindowInfo, WindowOpenDisposition, WrapApp, WrapBrowserProcessHandler, WrapClient, WrapLifeSpanHandler, WrapPermissionHandler,
        WrapUrlrequestClient,
        rc::Rc as _, wrap_app, wrap_browser_process_handler, wrap_client, wrap_life_span_handler, wrap_permission_handler, wrap_urlrequest_client,
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
    const EXTENSIONS_SETTLE: Duration = Duration::from_millis(5000);
    // cef_urlrequest_flags_t.
    const SKIP_CACHE: i32 = 1 << 0;
    const ALLOW_STORED_CREDENTIALS: i32 = 1 << 3;
    const SCRIPTS_REACH_PAGES: Duration = Duration::from_millis(150);
    const LEFT_MOUSE_DOWN: u64 = 1;
    const RIGHT_MOUSE_DOWN: u64 = 3;
    const COMMAND_KEY: u64 = 1 << 20;
    const SHIFT_KEY: u64 = 1 << 17;
    const WIDTH_AND_HEIGHT: u64 = 2 | 16;
    const BOTTOM_LEFT_AND_RIGHT: u64 = 1 | 2;
    const PUMP_AT_LEAST_EVERY: Duration = Duration::from_millis(250);
    const SEAL_EVERY: Duration = Duration::from_secs(1);
    const CHROME_WINDOWS_CLOSE: Duration = Duration::from_millis(1000);
    const DEBUG_PORT_WAIT: Duration = Duration::from_millis(500);
    const LEAN_FEATURES_OFF: &str = "NativeNotifications,SystemNotifications,SpareRendererForSitePerProcess,MediaRouter,\
        DialMediaRouteProvider,OptimizationHints,OptimizationGuideModelDownloading,Translate";
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
        // A mod's popup over the page (its address, its menu): x, y, width, height in the window.
        hole: Option<[f64; 4]>,
        // While the pane is private: its normal page, parked and hidden, to come back as it was.
        normal: Option<Browser>,
    }

    /// Herdr's own popups and toasts over pages, in window coordinates: the page shows a hole
    /// where each one is. While a popup is open every page lets presses through, so a press on a
    /// page closes the popup as it would anywhere else; a toast lets through only its own.
    #[derive(Clone, Copy, PartialEq)]
    struct Cover {
        area: [f64; 4],
        pass_all: bool,
    }

    thread_local! {
        static VIEWS: RefCell<HashMap<String, View>> = RefCell::new(HashMap::new());
        static CLICK: RefCell<Option<Box<dyn Fn(String)>>> = const { RefCell::new(None) };
        static SUSPENDED: Cell<bool> = const { Cell::new(false) };
        static LAST_CLICK: Cell<f64> = const { Cell::new(0.0) };
        static LIBRARY: RefCell<Option<cef::library_loader::LibraryLoader>> = const { RefCell::new(None) };
        static STARTED: Cell<bool> = const { Cell::new(false) };
        static STARTED_AT: Cell<Option<Instant>> = const { Cell::new(None) };
        // Private panes' pages (by id), and the in-memory session they share while any is open.
        static PRIVATE_IDS: RefCell<std::collections::HashSet<String>> = RefCell::new(std::collections::HashSet::new());
        static PRIVATE_CONTEXT: RefCell<Option<RequestContext>> = const { RefCell::new(None) };
        // Who the profile is signed in to Google as ({signedIn, email, name}), and the answer coming in.
        static GOOGLE: RefCell<serde_json::Value> = const { RefCell::new(serde_json::Value::Null) };
        static GOOGLE_BODY: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
        static GOOGLE_REQUEST: RefCell<Option<Urlrequest>> = const { RefCell::new(None) };
        static STARTED_WALL: Cell<Option<SystemTime>> = const { Cell::new(None) };
        // When the autofill extension said extensions are loaded (its helper touches a file).
        static EXTENSIONS_READY_AT: Cell<Option<Instant>> = const { Cell::new(None) };
        // Pages asked for while extensions are still getting ready (page id, address).
        static SETTLING: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
        static IN_CEF: Cell<bool> = const { Cell::new(false) };
        static UNDRAWN: RefCell<std::collections::HashSet<String>> = RefCell::new(std::collections::HashSet::new());
        static COVERS: RefCell<Vec<Cover>> = const { RefCell::new(Vec::new()) };
        static APPLIED_COVERS: RefCell<Vec<Cover>> = const { RefCell::new(Vec::new()) };
        // Each host's holes, in its own coordinates, for presses to pass through (by host pointer).
        static HOST_HOLES: RefCell<HashMap<usize, Vec<NsRect>>> = RefCell::new(HashMap::new());
        // How many holes each host's mask has now (by host pointer), for `web.report`.
        static MASK_HOLES: RefCell<HashMap<usize, usize>> = RefCell::new(HashMap::new());
        // Popup windows open now (a site's sign-in, usually).
        static POPUPS: RefCell<Vec<Browser>> = const { RefCell::new(Vec::new()) };
        static GPUI_VIEW: Cell<Id> = const { Cell::new(std::ptr::null_mut()) };
        // The page that had the keyboard when a popup or dialog took it, to get it back after.
        static HELD_KEYS: RefCell<Option<String>> = const { RefCell::new(None) };
        static LAST_SEAL: Cell<Option<Instant>> = const { Cell::new(None) };
        static PUMP_AGAIN: Cell<bool> = const { Cell::new(false) };
        // Addresses sent before their page's view has drawn.
        static WAITING: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    }

    /// What a page shows before it paints: the app's theme background, so opening one doesn't flash white.
    static BACKGROUND: AtomicU32 = AtomicU32::new(0xFF1F2328);

    pub fn set_background(color: u32) {
        BACKGROUND.store(color, Ordering::Relaxed);
    }

    // One profile for every copy of the app, so a sign-in holds everywhere. Tests point elsewhere.
    fn state_dir() -> PathBuf {
        std::env::var("HERDR_BROWSER_STATE").map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/state/herdr-browser"))
    }

    // New sites open at the app's zoom, not at 100% until a page is told otherwise, which showed as
    // a jump on each new site. Chromium won't take this preference at runtime, so it goes into the
    // profile before start. "x" is its key for the profile's default partition.
    fn seed_default_zoom(cache: &std::path::Path) {
        let path = cache.join("Default/Preferences");
        let mut prefs: serde_json::Value = std::fs::read_to_string(&path).ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_else(|| serde_json::json!({}));
        let level = zoom_level(DEFAULT_ZOOM);
        if prefs.pointer("/partition/default_zoom_level/x").and_then(|v| v.as_f64()) == Some(level) {
            return;
        }
        let Some(root) = prefs.as_object_mut() else { return };
        let partition = root.entry("partition").or_insert_with(|| serde_json::json!({}));
        if let Some(partition) = partition.as_object_mut() {
            partition.insert("default_zoom_level".into(), serde_json::json!({ "x": level }));
        }
        let _ = std::fs::create_dir_all(cache.join("Default"));
        if let Err(e) = std::fs::write(&path, prefs.to_string()) {
            tracing::warn!("web view: default zoom not written: {e}");
        }
    }

    // Herdr's own extensions, written out at start beside uBlock. The extensions page is one, since
    // only an extension may manage the others (Chrome's page doesn't open in a pane). Autofill is the
    // other: it fills from the user's vault through a native messaging host in the app.
    const OWN_EXTENSIONS: [(&str, &[(&str, &str)]); 3] = [
        ("extensions-page", &[
            ("manifest.json", include_str!("extensions-page/manifest.json")),
            ("manage.html", include_str!("extensions-page/manage.html")),
            ("manage.css", include_str!("extensions-page/manage.css")),
            ("manage.js", include_str!("extensions-page/manage.js")),
        ]),
        ("autofill", &[
            ("manifest.json", include_str!("autofill/manifest.json")),
            ("background.js", include_str!("autofill/background.js")),
            ("content.js", include_str!("autofill/content.js")),
            ("webauthn.js", include_str!("autofill/webauthn.js")),
        ]),
        ("minimal-google", &[
            ("manifest.json", include_str!("minimal-google/manifest.json")),
            ("minimal-google.css", include_str!("minimal-google/minimal-google.css")),
        ]),
    ];
    const AUTOFILL_HOST: &str = "dev.herdr.autofill";

    // Each version of an extension of Herdr's own gets a folder of its own. Chromium names an unpacked
    // extension after its folder, so a new version arrives as a fresh install. Updating one in place
    // made Chromium reload it at start, and a page asking its worker during that reload left the
    // worker refused ("disallowed") for the whole session.
    fn own_extension_folder(name: &str) -> String {
        use sha2::{Digest, Sha256};
        let Some((_, files)) = OWN_EXTENSIONS.iter().find(|(folder, _)| *folder == name) else { return name.to_owned() };
        let mut hash = Sha256::new();
        files.iter().for_each(|(file, text)| {
            hash.update(file.as_bytes());
            hash.update(text.as_bytes());
        });
        let hash = hash.finalize();
        format!("{name}-{:02x}{:02x}{:02x}{:02x}", hash[0], hash[1], hash[2], hash[3])
    }

    fn write_own_extensions(cache: &std::path::Path) {
        for (name, files) in OWN_EXTENSIONS {
            let current = own_extension_folder(name);
            // Earlier versions' folders go: Chromium only loads the folders it's given.
            if let Ok(entries) = std::fs::read_dir(state_dir()) {
                for entry in entries.flatten() {
                    let folder = entry.file_name().to_string_lossy().into_owned();
                    let earlier = folder == *name || folder.strip_prefix(&format!("{name}-")).is_some_and(|rest| rest.len() == 8);
                    if earlier && folder != current {
                        let _ = std::fs::remove_dir_all(entry.path());
                    }
                }
            }
            let folder = state_dir().join(&current);
            let _ = std::fs::create_dir_all(&folder);
            for (file, text) in files.iter() {
                let path = folder.join(file);
                if std::fs::read_to_string(&path).ok().as_deref() != Some(*text) {
                    let _ = std::fs::write(&path, text);
                }
            }
        }
        // Chromium starts the host beside the app's own executable, and only for the autofill extension.
        let (Ok(exe), Some(id)) = (std::env::current_exe(), extension_id(&own_extension_folder("autofill"))) else { return };
        let manifest = serde_json::json!({
            "name": AUTOFILL_HOST,
            "description": "Herdr Autofill",
            "path": exe.with_file_name("herdr-autofill-host"),
            "type": "stdio",
            "allowed_origins": [format!("chrome-extension://{id}/")],
        });
        let hosts = cache.join("NativeMessagingHosts");
        let _ = std::fs::create_dir_all(&hosts);
        let _ = std::fs::write(hosts.join(format!("{AUTOFILL_HOST}.json")), manifest.to_string());
    }

    // Chromium names an extension loaded from a folder after that folder's real path.
    fn extension_id(folder: &str) -> Option<String> {
        use sha2::{Digest, Sha256};
        let path = std::fs::canonicalize(state_dir().join(folder)).ok()?;
        let hash = Sha256::digest(path.to_string_lossy().as_bytes());
        Some(hash[..16].iter().flat_map(|byte| [byte >> 4, byte & 15]).map(|n| (b'a' + n) as char).collect())
    }

    fn zoom_level(factor: f64) -> f64 {
        factor.ln() / ZOOM_BASE.ln()
    }

    wrap_app! {
        struct HerdrCefApp;

        impl App {
            fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
                Some(PumpScheduler::new())
            }

            // Extensions come in as they would for Chromium itself.
            fn on_before_command_line_processing(&self, process_type: Option<&CefString>, command_line: Option<&mut CommandLine>) {
                let is_browser = process_type.is_none_or(|kind| kind.to_string().is_empty());
                let (true, Some(line)) = (is_browser, command_line) else { return };
                let switch = |name: &str, value: Option<&str>| match value {
                    Some(value) => line.append_switch_with_value(Some(&CefString::from(name)), Some(&CefString::from(value))),
                    None => line.append_switch(Some(&CefString::from(name))),
                };
                // Kept lean: pages of one site share a renderer, no spare one waits for the next
                // site, and nothing fetches or casts in the background. Pages can't post to
                // macOS, so the notifications helper would only sit idle.
                switch("process-per-site", None);
                switch("disable-background-networking", None);
                switch("disable-notifications", None);
                // Chromium opens no window of its own, not even to offer "Restore pages?" after a crash.
                switch("no-startup-window", None);
                switch("hide-crash-restore-bubble", None);
                switch("disable-features", Some(LEAN_FEATURES_OFF));
                // Web pages in panes can use the microphone and camera (voice search, calls).
                switch("enable-media-stream", None);
                // HERDR_CHROMIUM_LOG=<vmodule spec> turns on Chromium's own logging, to stderr, for debugging.
                if let Ok(modules) = std::env::var("HERDR_CHROMIUM_LOG") {
                    switch("enable-logging", Some("stderr"));
                    switch("vmodule", Some(&modules));
                }
                // HERDR_KEEP_PAGES_AWAKE keeps pages live while the window is covered, for tests run
                // in a copy behind another window (a covered page is hidden and takes no input).
                // A background copy (HERDR_BACKGROUND) sits behind other windows, so it does too.
                if std::env::var_os("HERDR_KEEP_PAGES_AWAKE").is_some() || std::env::var_os("HERDR_BACKGROUND").is_some() {
                    switch("disable-backgrounding-occluded-windows", None);
                }
                let folders: Vec<String> = [
                    "ublock".to_owned(), own_extension_folder("extensions-page"),
                    own_extension_folder("autofill"), own_extension_folder("minimal-google"),
                ]
                    .iter().map(|folder| state_dir().join(folder))
                    .filter(|folder| folder.join("manifest.json").exists())
                    .map(|folder| folder.to_string_lossy().into_owned()).collect();
                if !folders.is_empty() {
                    switch("load-extension", Some(&folders.join(",")));
                }
            }
        }
    }

    wrap_browser_process_handler! {
        struct PumpScheduler;

        impl BrowserProcessHandler {
            fn on_schedule_message_pump_work(&self, delay_ms: i64) {
                schedule(delay_ms);
            }

            // Another launch on this profile would otherwise open a Chrome window here.
            fn on_already_running_app_relaunch(&self, _command_line: Option<&mut CommandLine>, _current_directory: Option<&CefString>) -> ::std::os::raw::c_int {
                1
            }
        }
    }

    wrap_client! {
        struct PageClient;

        impl Client {
            fn life_span_handler(&self) -> Option<LifeSpanHandler> {
                Some(PagePopups::new())
            }

            fn permission_handler(&self) -> Option<PermissionHandler> {
                Some(PagePermissions::new())
            }
        }
    }

    // A page that asks for the microphone or camera (voice search, a call) gets them: these are
    // Herdr's own panes, and macOS still asks once for Herdr itself.
    wrap_permission_handler! {
        struct PagePermissions;

        impl PermissionHandler {
            fn on_request_media_access_permission(
                &self,
                _browser: Option<&mut Browser>,
                _frame: Option<&mut Frame>,
                _origin: Option<&CefString>,
                requested: u32,
                callback: Option<&mut MediaAccessCallback>,
            ) -> ::std::os::raw::c_int {
                if let Some(callback) = callback {
                    callback.cont(requested);
                }
                1
            }
        }
    }

    wrap_life_span_handler! {
        struct PagePopups;

        impl LifeSpanHandler {
            // A script's popup (a sign-in, usually), or one opened blank to be filled in later, gets a
            // window of its own and keeps its tie to the page that opened it, which waits to hear back
            // from it. Links that only ask for a new tab open in this pane.
            #[allow(clippy::too_many_arguments)]
            fn on_before_popup(
                &self,
                browser: Option<&mut Browser>,
                _frame: Option<&mut Frame>,
                _popup_id: ::std::os::raw::c_int,
                target_url: Option<&CefString>,
                _target_frame_name: Option<&CefString>,
                target_disposition: WindowOpenDisposition,
                _user_gesture: ::std::os::raw::c_int,
                _popup_features: Option<&PopupFeatures>,
                window_info: Option<&mut WindowInfo>,
                _client: Option<&mut Option<Client>>,
                _settings: Option<&mut BrowserSettings>,
                _extra_info: Option<&mut Option<DictionaryValue>>,
                _no_javascript_access: Option<&mut ::std::os::raw::c_int>,
            ) -> ::std::os::raw::c_int {
                let url = target_url.map(CefString::to_string).unwrap_or_default();
                let blank = url.is_empty() || url == "about:blank";
                if blank || target_disposition.as_ref() == WindowOpenDisposition::NEW_POPUP.as_ref() {
                    if let Some(info) = window_info {
                        info.runtime_style = RuntimeStyle::ALLOY;
                    }
                    return 0;
                }
                if let Some(frame) = browser.and_then(|browser| browser.main_frame()) {
                    frame.load_url(target_url);
                }
                1
            }

            fn on_after_created(&self, browser: Option<&mut Browser>) {
                if let Some(browser) = browser.filter(|browser| browser.is_popup() != 0) {
                    POPUPS.with_borrow_mut(|popups| popups.push(browser.clone()));
                    if let Some(host) = browser.host() {
                        center_on_herdr(host.window_handle() as Id);
                    }
                }
            }

            fn on_before_close(&self, browser: Option<&mut Browser>) {
                if let Some(browser) = browser {
                    let id = browser.identifier();
                    POPUPS.with_borrow_mut(|popups| popups.retain(|popup| popup.identifier() != id));
                }
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
            seed_default_zoom(&cache);
            write_own_extensions(&cache);
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
            STARTED_AT.set(Some(Instant::now()));
            STARTED_WALL.set(Some(SystemTime::now()));
            // The debugging port opens on a thread of its own a moment later. It's sealed before
            // the app goes on to start any children, the core among them.
            if debug_port > 0 {
                let port = std::net::SocketAddr::from(([127, 0, 0, 1], debug_port as u16));
                let deadline = Instant::now() + DEBUG_PORT_WAIT;
                while Instant::now() < deadline && std::net::TcpStream::connect_timeout(&port, Duration::from_millis(20)).is_err() {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            seal_fds();
            unsafe {
                let app: Id = msg_send![class!(NSApplication), sharedApplication];
                let _: () = msg_send![app, setBeforeTerminate: finish as extern "C" fn() as *mut c_void];
            }
            true
        }
    }

    // CEF's work runs on the main queue when CEF says it's due, from any thread, or at worst every
    // PUMP_AT_LEAST_EVERY. A newer timed request replaces the one waiting; one due now always runs.
    unsafe extern "C" {
        static _dispatch_main_q: u8;
        fn dispatch_async_f(queue: *const u8, context: *mut c_void, work: extern "C" fn(*mut c_void));
        fn dispatch_after_f(when: u64, queue: *const u8, context: *mut c_void, work: extern "C" fn(*mut c_void));
        fn dispatch_time(when: u64, delta: i64) -> u64;
    }

    // Chromium opens files and sockets without close-on-exec, since its own launcher closes them
    // in its children. The app's children would inherit them: a core started by the app held the
    // debugging port long after the app quit. So they're sealed after start and then now and then.
    fn seal_fds() {
        let Ok(dir) = std::fs::read_dir("/dev/fd") else { return };
        let fds: Vec<i32> = dir.filter_map(|entry| entry.ok()?.file_name().to_str()?.parse().ok()).collect();
        for fd in fds.into_iter().filter(|fd| *fd > 2) {
            unsafe {
                let flags = libc::fcntl(fd, libc::F_GETFD);
                if flags >= 0 && flags & libc::FD_CLOEXEC == 0 {
                    libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
                }
            }
        }
    }

    static PUMP_TICKET: AtomicU64 = AtomicU64::new(0);
    static PUMP_WAITING: AtomicBool = AtomicBool::new(false);

    fn schedule(delay_ms: i64) {
        let main = unsafe { &raw const _dispatch_main_q };
        if delay_ms <= 0 {
            unsafe { dispatch_async_f(main, std::ptr::null_mut(), pump_now) };
            return;
        }
        let ticket = PUMP_TICKET.fetch_add(1, Ordering::SeqCst) + 1;
        PUMP_WAITING.store(true, Ordering::SeqCst);
        let delay = Duration::from_millis(delay_ms as u64).min(PUMP_AT_LEAST_EVERY);
        unsafe { dispatch_after_f(dispatch_time(0, delay.as_nanos() as i64), main, ticket as *mut c_void, pump_later) };
    }

    extern "C" fn pump_later(ticket: *mut c_void) {
        if ticket as u64 == PUMP_TICKET.load(Ordering::SeqCst) {
            PUMP_WAITING.store(false, Ordering::SeqCst);
            pump_now(std::ptr::null_mut());
        }
    }

    /// How long after Chromium starts its extensions' page scripts (uBlock's, autofill's) are
    /// ready. A page loaded sooner never gets them, and one like YouTube never reloads to.
    fn extensions_settle() -> Duration {
        std::env::var("HERDR_PAGE_SETTLE_MS").ok().and_then(|ms| ms.parse().ok()).map_or(EXTENSIONS_SETTLE, Duration::from_millis)
    }

    /// Held pages wait for the extensions to say they're loaded, plus a moment for their scripts to
    /// reach new pages, as Chromium holds a navigation until they're ready; at most the settle time.
    fn settling() -> bool {
        let Some(started) = STARTED_AT.get() else { return false };
        if started.elapsed() >= extensions_settle() {
            return false;
        }
        if EXTENSIONS_READY_AT.get().is_none() {
            let touched = std::fs::metadata(state_dir().join("extensions-ready")).and_then(|m| m.modified()).ok();
            if touched.is_some_and(|at| STARTED_WALL.get().is_some_and(|start| at >= start)) {
                EXTENSIONS_READY_AT.set(Some(Instant::now()));
            }
        }
        EXTENSIONS_READY_AT.get().is_none_or(|ready| ready.elapsed() < SCRIPTS_REACH_PAGES)
    }

    /// Holds a page's address until extensions are ready; true when it was held.
    fn hold_while_settling(id: &str, url: &str) -> bool {
        if !settling() || url.is_empty() || url == "about:blank" {
            return false;
        }
        SETTLING.with_borrow_mut(|held| {
            held.retain(|(held_id, _)| held_id != id);
            held.push((id.to_owned(), url.to_owned()));
        });
        true
    }

    fn load_settled() {
        if settling() || SETTLING.with_borrow(Vec::is_empty) {
            return;
        }
        for (id, url) in SETTLING.take() {
            let browser = VIEWS.with(|views| views.try_borrow().ok().and_then(|views| views.get(&id).and_then(|view| view.browser.clone())));
            if let Some(frame) = browser.and_then(|browser| browser.main_frame()) {
                frame.load_url(Some(&CefString::from(url.as_str())));
            }
        }
    }

    extern "C" fn pump_now(_: *mut c_void) {
        if !STARTED.get() {
            return;
        }
        settle_chrome_dialogs();
        if LAST_SEAL.get().is_none_or(|at| at.elapsed() >= SEAL_EVERY) {
            LAST_SEAL.set(Some(Instant::now()));
            seal_fds();
        }
        // CEF can turn the run loop while it works, which can land back here.
        if IN_CEF.replace(true) {
            PUMP_AGAIN.set(true);
            return;
        }
        load_settled();
        cef::do_message_loop_work();
        IN_CEF.set(false);
        if PUMP_AGAIN.take() {
            schedule(0);
        } else if !PUMP_WAITING.load(Ordering::SeqCst) {
            schedule(PUMP_AT_LEAST_EVERY.as_millis() as i64);
        }
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
            let superview: Id = msg_send![this as *mut Object, superview];
            let local: Point = msg_send![this as *mut Object, convertPoint: point fromView: superview];
            let key = this as *mut Object as usize;
            let in_hole = HOST_HOLES.with(|holes| holes.try_borrow().is_ok_and(|holes| {
                holes.get(&key).is_some_and(|rects| rects.iter().any(|r| {
                    local.x >= r.origin.x && local.x <= r.origin.x + r.size.w && local.y >= r.origin.y && local.y <= r.origin.y + r.size.h
                }))
            }));
            if in_hole {
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

    fn create_browser(host: Id, url: &str, width: i32, height: i32, private: bool) -> Option<Browser> {
        let bounds = CefRect { x: 0, y: 0, width, height };
        let info = WindowInfo { runtime_style: RuntimeStyle::ALLOY, ..Default::default() }
            .set_as_child(host as cef::sys::cef_window_handle_t, &bounds);
        let mut client = PageClient::new();
        let mut session = private.then(private_context).flatten();
        let settings = BrowserSettings { background_color: BACKGROUND.load(Ordering::Relaxed), ..Default::default() };
        let browser = cef::browser_host_create_browser_sync(
            Some(&info), Some(&mut client), Some(&CefString::from(url)),
            Some(&settings), None, session.as_mut(),
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
        for browser in view.browser.iter().chain(view.normal.iter()) {
            if let Some(host) = browser.host() {
                host.close_browser(1);
            }
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
        if let Some(cookies) = cef::cookie_manager_get_global_manager(None) {
            cookies.flush_store(None);
        }
        // Chromium's own windows (an extension's install prompt) have to go first: its shutdown
        // crashes with one open. They close through the run loop, which nothing turns during a quit.
        for window in chrome_windows() {
            unsafe {
                let _: () = msg_send![window, close];
            }
        }
        VIEWS.take().values().for_each(close_page);
        for host in POPUPS.with_borrow(|popups| popups.iter().filter_map(|popup| popup.host()).collect::<Vec<_>>()) {
            host.close_browser(1);
        }
        let open = || !chrome_windows().is_empty() || POPUPS.with_borrow(|popups| !popups.is_empty());
        let deadline = Instant::now() + CHROME_WINDOWS_CLOSE;
        while open() && Instant::now() < deadline {
            cef::do_message_loop_work();
            unsafe {
                let pool: Id = msg_send![class!(NSAutoreleasePool), new];
                let until: Id = msg_send![class!(NSDate), dateWithTimeIntervalSinceNow: 0.01f64];
                let run_loop: Id = msg_send![class!(NSRunLoop), currentRunLoop];
                let _: BOOL = msg_send![run_loop, runMode: ns_string("kCFRunLoopDefaultMode") beforeDate: until];
                let _: () = msg_send![pool, drain];
            }
        }
        // Better to skip Chromium's shutdown than crash in it; the cookies are already written.
        if !open() {
            cef::shutdown();
        } else {
            tracing::warn!("web view: a Chromium window wouldn't close; quitting without its shutdown");
        }
    }

    // Chromium's own dialogs (an extension's install prompt) come as loose windows whose title bar
    // covers their heading. Each gets a clear title bar and sits centred on Herdr's window, attached
    // to it. Only titled windows: a page's dropdowns and pickers are untitled and stay where they are.
    fn settle_chrome_dialogs() {
        let gpui = GPUI_VIEW.get();
        if gpui.is_null() {
            return;
        }
        const TITLED: u64 = 1;
        const TITLE_HIDDEN: i64 = 1;
        const ABOVE: i64 = 1;
        unsafe {
            let main: Id = msg_send![gpui, window];
            if main.is_null() {
                return;
            }
            for window in chrome_windows() {
                let style: u64 = msg_send![window, styleMask];
                let parent: Id = msg_send![window, parentWindow];
                if style & TITLED == 0 || parent == main {
                    continue;
                }
                let _: () = msg_send![window, setTitlebarAppearsTransparent: YES];
                let _: () = msg_send![window, setTitleVisibility: TITLE_HIDDEN];
                let area: NsRect = msg_send![main, frame];
                let frame: NsRect = msg_send![window, frame];
                let origin = Point {
                    x: area.origin.x + (area.size.w - frame.size.w) / 2.0,
                    y: area.origin.y + (area.size.h - frame.size.h) / 2.0,
                };
                let _: () = msg_send![window, setFrameOrigin: origin];
                let _: () = msg_send![main, addChildWindow: window ordered: ABOVE];
            }
        }
    }

    // The app's windows that Chromium opened itself (views widgets), not Herdr's own.
    // A popup opens over the middle of Herdr's window, not wherever the site asked.
    fn center_on_herdr(view: Id) {
        let gpui = GPUI_VIEW.get();
        if view.is_null() || gpui.is_null() {
            return;
        }
        unsafe {
            let window: Id = msg_send![view, window];
            let main: Id = msg_send![gpui, window];
            if window.is_null() || main.is_null() || window == main {
                return;
            }
            let area: NsRect = msg_send![main, frame];
            let frame: NsRect = msg_send![window, frame];
            let origin = Point {
                x: area.origin.x + (area.size.w - frame.size.w) / 2.0,
                y: area.origin.y + (area.size.h - frame.size.h) / 2.0,
            };
            let _: () = msg_send![window, setFrameOrigin: origin];
        }
    }

    fn chrome_windows() -> Vec<Id> {
        unsafe {
            let app: Id = msg_send![class!(NSApplication), sharedApplication];
            let windows: Id = msg_send![app, windows];
            let count: usize = msg_send![windows, count];
            (0..count).filter_map(|i| {
                let window: Id = msg_send![windows, objectAtIndex: i];
                let visible: BOOL = msg_send![window, isVisible];
                let class: Id = msg_send![window, className];
                (visible == YES && rust_string(class).starts_with("NativeWidget")).then_some(window)
            }).collect()
        }
    }

    /// Puts the window behind every other app's, without touching focus: for a copy of the app
    /// run in the background (tests), which mustn't cover what the person is doing.
    pub fn order_back(window: &Window) {
        let Ok(handle) = HasWindowHandle::window_handle(window) else { return };
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else { return };
        let view = handle.ns_view.as_ptr() as Id;
        unsafe {
            let ns_window: Id = msg_send![view, window];
            if !ns_window.is_null() {
                let _: () = msg_send![ns_window, orderBack: NIL];
            }
        }
    }

    pub fn place(window: &Window, id: &str, bounds: Bounds<Pixels>, _cx: &mut GpuiApp) {
        if !STARTED.get() {
            return;
        }
        let Ok(handle) = HasWindowHandle::window_handle(window) else { return };
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else { return };
        let gpui_view = handle.ns_view.as_ptr() as Id;
        GPUI_VIEW.set(gpui_view);
        watch_presses();
        // A page moves in Core Animation's transaction; the window's drawing has to land in the
        // same one, or for a frame the pane and its page disagree (the flash on zoom).
        gpui_apple::metal_renderer::MetalRenderer::present_with_native_views(true);
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
                hole: None,
                normal: None,
            });
            view.top = top;
            view.left = left;
            let frame = NsRect { origin: Point { x: left, y: space.size.h - top - height }, size: Size { w: width, h: height } };
            unsafe {
                let _: () = msg_send![view.host, setFrame: frame];
            }
            apply_mask(view);
            if view.browser.is_none() && width > 0.0 && height > 0.0 { Some(view.host) } else { None }
        });
        // Creating a browser runs the message loop, which reaches back into VIEWS.
        if let Some(host) = created {
            let mut url = WAITING.with_borrow_mut(|waiting| waiting.remove(id)).unwrap_or_else(|| "about:blank".to_owned());
            if hold_while_settling(id, &url) {
                url = "about:blank".to_owned();
            }
            let nested = IN_CEF.replace(true);
            let private = PRIVATE_IDS.with_borrow(|ids| ids.contains(id));
            let browser = create_browser(host, &url, width as i32, height as i32, private);
            IN_CEF.set(nested);
            if !nested && PUMP_AGAIN.take() {
                schedule(0);
            }
            VIEWS.with_borrow_mut(|views| {
                if let Some(view) = views.get_mut(id) {
                    view.browser = browser;
                    apply_zoom(view);
                }
            });
        }
        apply_visibility();
    }

    /// Opens a page before its pane is drawn (a browser pane in a hidden tab), so
    /// coming back to it shows it already loaded. The element adopts it when drawn.
    pub fn preload(window: &Window, id: &str, url: &str) -> bool {
        if !STARTED.get() || VIEWS.with_borrow(|views| views.get(id).is_some_and(|view| view.browser.is_some())) {
            return false;
        }
        let Ok(handle) = HasWindowHandle::window_handle(window) else { return false };
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else { return false };
        let gpui_view = handle.ns_view.as_ptr() as Id;
        let parent: Id = unsafe { msg_send![gpui_view, superview] };
        if parent.is_null() {
            return false;
        }
        let host = unsafe { new_host(parent) };
        unsafe {
            let frame = NsRect { origin: Point { x: 0.0, y: 0.0 }, size: Size { w: 1.0, h: 1.0 } };
            let _: () = msg_send![host, setFrame: frame];
            let _: () = msg_send![host, setHidden: YES];
        }
        let nested = IN_CEF.replace(true);
        let browser = create_browser(host, url, 1, 1, false);
        IN_CEF.set(nested);
        if !nested && PUMP_AGAIN.take() {
            schedule(0);
        }
        let Some(browser) = browser else {
            unsafe { let _: () = msg_send![host, removeFromSuperview]; }
            return false;
        };
        VIEWS.with_borrow_mut(|views| {
            views.insert(id.to_owned(), View {
                host, browser: Some(browser), url: url.to_owned(), title: String::new(),
                zoom: DEFAULT_ZOOM, shown: false, top: 0.0, left: 0.0, asked: 0, hole: None, normal: None,
            });
        });
        apply_visibility();
        true
    }

    // AppKit sends keys to the view holding first responder, and a page keeps it after Herdr's own
    // focus has moved on. These move it between the app's view and a page.
    fn page_with_keys() -> Option<String> {
        let gpui = GPUI_VIEW.get();
        if gpui.is_null() {
            return None;
        }
        unsafe {
            let window: Id = msg_send![gpui, window];
            let responder: Id = if window.is_null() { NIL } else { msg_send![window, firstResponder] };
            if responder.is_null() {
                return None;
            }
            let is_view: BOOL = msg_send![responder, isKindOfClass: class!(NSView)];
            if is_view == NO {
                return None;
            }
            VIEWS.with_borrow(|views| {
                views.iter().find(|(_, view)| {
                    let inside: BOOL = msg_send![responder, isDescendantOf: view.host];
                    inside == YES
                }).map(|(id, _)| id.clone())
            })
        }
    }

    fn take_keys_back() {
        let Some(id) = page_with_keys() else { return };
        unsafe {
            let window: Id = msg_send![GPUI_VIEW.get(), window];
            let _: BOOL = msg_send![window, makeFirstResponder: GPUI_VIEW.get()];
        }
        if let Some(host) = VIEWS.with_borrow(|views| views.get(&id).and_then(|view| view.browser.clone())).and_then(|browser| browser.host()) {
            host.set_focus(0);
        }
    }

    fn give_keys(id: &str) {
        let page = VIEWS.with_borrow(|views| {
            let view = views.get(id)?;
            let holed: BOOL = unsafe { *(*view.host).get_ivar("pass") };
            (view.shown && holed == NO).then(|| view.browser.clone()).flatten()
        });
        let (Some(host), false, false) = (page.and_then(|browser| browser.host()), GPUI_VIEW.get().is_null(), SUSPENDED.get()) else { return };
        unsafe {
            let window: Id = msg_send![GPUI_VIEW.get(), window];
            let _: BOOL = msg_send![window, makeFirstResponder: host.window_handle() as Id];
        }
        host.set_focus(1);
    }

    // A press anywhere in Herdr outside a page (a tab being renamed, a field in the sidebar or a popup)
    // takes the keyboard back from the page, as moving to another pane does. A press on a page is
    // the page's. Pane focus alone misses these: renaming a tab leaves the browser pane focused.
    fn watch_presses() {
        static WATCHING: Once = Once::new();
        WATCHING.call_once(|| {
            const PRESSES: u64 = (1 << 1) | (1 << 3) | (1 << 25);
            let handler = block::ConcreteBlock::new(|event: Id| -> Id {
                // Never mid-change of the pages (a panic here would end the app).
                if event.is_null() || VIEWS.with(|views| views.try_borrow().is_err()) {
                    return event;
                }
                unsafe {
                    let gpui = GPUI_VIEW.get();
                    let window: Id = msg_send![event, window];
                    let main: Id = if gpui.is_null() { NIL } else { msg_send![gpui, window] };
                    if !window.is_null() && window == main {
                        let content: Id = msg_send![window, contentView];
                        let point: Point = msg_send![event, locationInWindow];
                        let hit: Id = msg_send![content, hitTest: point];
                        let on_page = !hit.is_null() && VIEWS.with_borrow(|views| views.values().any(|view| {
                            let inside: BOOL = msg_send![hit, isDescendantOf: view.host];
                            inside == YES
                        }));
                        if !on_page {
                            take_keys_back();
                        }
                    }
                }
                event
            }).copy();
            unsafe {
                let monitor: Id = msg_send![class!(NSEvent), addLocalMonitorForEventsMatchingMask: PRESSES handler: &*handler];
                if !monitor.is_null() {
                    let _: Id = msg_send![monitor, retain];
                }
            }
            // The monitor holds the block for as long as the app runs.
            std::mem::forget(handler);
        });
    }

    /// Pages whose panes the window isn't drawing now (another tab, or another pane zoomed), so
    /// they hide in the same frame rather than when a mod gets round to it.
    pub fn set_undrawn(ids: std::collections::HashSet<String>) {
        if UNDRAWN.with_borrow(|undrawn| *undrawn == ids) {
            return;
        }
        UNDRAWN.set(ids);
        apply_visibility();
    }

    fn apply_visibility() {
        let suspended = SUSPENDED.get();
        // A hidden page mustn't keep the keyboard.
        let undrawn = |id: &str| UNDRAWN.with_borrow(|undrawn| undrawn.contains(id));
        if let Some(id) = page_with_keys()
            && (suspended || undrawn(&id) || VIEWS.with_borrow(|views| views.get(&id).is_some_and(|view| !view.shown)))
        {
            take_keys_back();
        }
        VIEWS.with_borrow(|views| {
            for (id, view) in views.iter() {
                let hidden: BOOL = if view.shown && !suspended && !undrawn(id) { NO } else { YES };
                unsafe {
                    let _: () = msg_send![view.host, setHidden: hidden];
                }
            }
        });
    }

    /// Dialogs dim the whole window from beneath this view, so pages step aside for them.
    pub fn set_suspended(suspended: bool) {
        if SUSPENDED.get() == suspended {
            return;
        }
        if suspended {
            HELD_KEYS.with_borrow_mut(|held| *held = page_with_keys());
        }
        SUSPENDED.set(suspended);
        apply_visibility();
        if !suspended && let Some(id) = HELD_KEYS.take() {
            give_keys(&id);
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

    /// Every page's id, without asking the pages anything.
    pub fn ids() -> Vec<String> {
        VIEWS.with_borrow(|views| views.keys().cloned().collect())
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
    fn set_hole(view: &mut View, area: &str) {
        let parts: Vec<f64> = area.split_whitespace().filter_map(|part| part.parse().ok()).collect();
        view.hole = (parts.len() == 4).then(|| [parts[0], parts[1], parts[2], parts[3]]);
        apply_mask(view);
    }

    /// Cuts the page's holes (a mod's popup, Herdr's popups and toasts over it) out of its picture,
    /// and says which presses pass through to Herdr.
    fn apply_mask(view: &View) {
        // The last whole frame's covers: this frame's are still being collected while pages place.
        let covers = APPLIED_COVERS.with(|covers| covers.try_borrow().map(|c| c.clone()).unwrap_or_default());
        unsafe {
            let bounds: NsRect = msg_send![view.host, bounds];
            let (width, height) = (bounds.size.w, bounds.size.h);
            let overlaps = |a: &[f64; 4]| a[2] > 0.0 && a[3] > 0.0 && a[0] < view.left + width && a[0] + a[2] > view.left
                && a[1] < view.top + height && a[1] + a[3] > view.top;
            let to_local = |a: &[f64; 4]| NsRect {
                origin: Point { x: a[0] - view.left, y: height - (a[1] - view.top) - a[3] },
                size: Size { w: a[2], h: a[3] },
            };
            let mut holes: Vec<NsRect> = view.hole.iter().filter(|a| overlaps(a)).map(to_local).collect();
            let covered: Vec<NsRect> = covers.iter().filter(|c| overlaps(&c.area)).map(|c| to_local(&c.area)).collect();
            holes.extend(covered.iter().copied());
            // Even-odd filling would close a hole where two meet (a mod's menu that is also a
            // popup reports the same place twice), so holes that touch become one.
            let holes = merge_overlapping(holes);
            MASK_HOLES.with(|map| {
                if let Ok(mut map) = map.try_borrow_mut() {
                    map.insert(view.host as usize, holes.len());
                }
            });
            let pass_all = view.hole.is_some() || covers.iter().any(|c| c.pass_all);
            let host = &mut *view.host;
            host.set_ivar::<BOOL>("pass", if pass_all { YES } else { NO });
            HOST_HOLES.with(|map| {
                if let Ok(mut map) = map.try_borrow_mut() {
                    map.insert(view.host as usize, covered);
                }
            });
            let layer: Id = msg_send![view.host, layer];
            if holes.is_empty() {
                let _: () = msg_send![layer, setMask: NIL];
                return;
            }
            let path = CGPathCreateMutable();
            CGPathAddRect(path, std::ptr::null(), bounds);
            for hole in &holes {
                CGPathAddRoundedRect(path, std::ptr::null(), *hole, HOLE_RADIUS, HOLE_RADIUS);
            }
            let mask: Id = msg_send![class!(CAShapeLayer), layer];
            let _: () = msg_send![mask, setPath: path];
            let _: () = msg_send![mask, setFillRule: kCAFillRuleEvenOdd];
            let _: () = msg_send![layer, setMask: mask];
            CGPathRelease(path);
        }
    }

    fn merge_overlapping(mut rects: Vec<NsRect>) -> Vec<NsRect> {
        let touch = |a: &NsRect, b: &NsRect| a.origin.x <= b.origin.x + b.size.w && b.origin.x <= a.origin.x + a.size.w
            && a.origin.y <= b.origin.y + b.size.h && b.origin.y <= a.origin.y + a.size.h;
        'again: loop {
            for i in 0..rects.len() {
                for j in i + 1..rects.len() {
                    if touch(&rects[i], &rects[j]) {
                        let (a, b) = (rects[i], rects.remove(j));
                        let (left, bottom) = (a.origin.x.min(b.origin.x), a.origin.y.min(b.origin.y));
                        let right = (a.origin.x + a.size.w).max(b.origin.x + b.size.w);
                        let top = (a.origin.y + a.size.h).max(b.origin.y + b.size.h);
                        rects[i] = NsRect { origin: Point { x: left, y: bottom }, size: Size { w: right - left, h: top - bottom } };
                        continue 'again;
                    }
                }
            }
            return rects;
        }
    }

    /// The private panes' session: in memory only (no cache folder), shared while any is open.
    fn private_context() -> Option<RequestContext> {
        PRIVATE_CONTEXT.with_borrow_mut(|context| {
            if context.is_none() {
                let settings = RequestContextSettings { persist_session_cookies: 0, ..Default::default() };
                *context = cef::request_context_create_context(Some(&settings), None);
            }
            context.clone()
        })
    }

    fn set_view_hidden(browser: &Browser, hidden: bool) {
        if let Some(host) = browser.host() {
            let view = host.window_handle() as Id;
            if !view.is_null() {
                unsafe {
                    let _: () = msg_send![view, setHidden: if hidden { YES } else { NO }];
                }
            }
        }
    }

    /// Switching a pane's page to the private session parks its normal page, hidden, and opens the
    /// same address privately on top; switching back closes the private page (and the session, if
    /// it was the last) and shows the normal page again as it was.
    fn switch_private(id: &str, on: bool) {
        if PRIVATE_IDS.with_borrow(|ids| ids.contains(id)) == on {
            return;
        }
        let Some((host, url, width, height)) = VIEWS.with_borrow(|views| views.get(id).map(|view| {
            let bounds: NsRect = unsafe { msg_send![view.host, bounds] };
            (view.host, view.url.clone(), bounds.size.w as i32, bounds.size.h as i32)
        })) else { return };
        let create = |url: &str, private: bool| {
            // Creating a browser runs the message loop, which reaches back into VIEWS.
            let nested = IN_CEF.replace(true);
            let browser = create_browser(host, if url.is_empty() { "about:blank" } else { url }, width, height, private);
            IN_CEF.set(nested);
            if !nested && PUMP_AGAIN.take() {
                schedule(0);
            }
            browser
        };
        if on {
            let parked = VIEWS.with_borrow_mut(|views| views.get_mut(id).and_then(|view| view.browser.take()));
            if let Some(normal) = &parked {
                set_view_hidden(normal, true);
            }
            PRIVATE_IDS.with_borrow_mut(|ids| ids.insert(id.to_owned()));
            let private = create(&url, true);
            VIEWS.with_borrow_mut(|views| {
                if let Some(view) = views.get_mut(id) {
                    view.normal = parked;
                    view.browser = private;
                    apply_zoom(view);
                }
            });
        } else {
            let (private, normal) = VIEWS.with_borrow_mut(|views| {
                views.get_mut(id).map(|view| (view.browser.take(), view.normal.take())).unwrap_or_default()
            });
            // Closed and taken out of the pane, as a closing pane's page is: only then does it go.
            if let Some(host) = private.as_ref().and_then(|private| private.host()) {
                let view = host.window_handle() as Id;
                host.close_browser(1);
                if !view.is_null() {
                    unsafe {
                        let _: () = msg_send![view, removeFromSuperview];
                    }
                }
            }
            PRIVATE_IDS.with_borrow_mut(|ids| ids.remove(id));
            if !VIEWS.with_borrow(|views| views.keys().any(|key| PRIVATE_IDS.with_borrow(|ids| ids.contains(key)))) {
                PRIVATE_CONTEXT.take();
            }
            let normal = match normal {
                Some(normal) => {
                    set_view_hidden(&normal, false);
                    Some(normal)
                }
                None => create("about:blank", false),
            };
            VIEWS.with_borrow_mut(|views| {
                if let Some(view) = views.get_mut(id) {
                    view.browser = normal;
                    view.url.clear();
                    refresh(view);
                }
            });
        }
        apply_visibility();
    }

    /// While a pane is private, the address its parked normal page is at.
    pub fn normal_url(id: &str) -> Option<String> {
        VIEWS.with(|views| views.try_borrow().ok().and_then(|views| {
            let normal = views.get(id)?.normal.as_ref()?;
            let entry = normal.host()?.visible_navigation_entry()?;
            Some(CefString::from(&entry.url()).to_string())
        }))
    }

    pub fn is_private(id: &str) -> bool {
        PRIVATE_IDS.with_borrow(|ids| ids.contains(id))
    }

    /// Who the profile is signed in to Google as, from the last `google` ask.
    pub fn google_account() -> serde_json::Value {
        GOOGLE.with_borrow(|google| google.clone())
    }

    wrap_urlrequest_client! {
        struct GoogleAccounts;

        impl UrlrequestClient {
            fn on_download_data(&self, _request: Option<&mut Urlrequest>, data: *const u8, data_length: usize) {
                if data.is_null() {
                    return;
                }
                let chunk = unsafe { std::slice::from_raw_parts(data, data_length) };
                GOOGLE_BODY.with_borrow_mut(|body| body.get_or_insert_default().extend_from_slice(chunk));
            }

            fn on_request_complete(&self, _request: Option<&mut Urlrequest>) {
                let body = GOOGLE_BODY.take().unwrap_or_default();
                GOOGLE_REQUEST.take();
                GOOGLE.set(google_from(&body));
            }
        }
    }

    /// Chrome's own account list (`ListAccounts`): ["gaia.l.a.r", [["gaia.l.a", 1, name, email, ...], ...]].
    fn google_from(body: &[u8]) -> serde_json::Value {
        let Ok(list) = serde_json::from_slice::<serde_json::Value>(body) else { return serde_json::Value::Null };
        let first = list.get(1).and_then(|accounts| accounts.as_array()).and_then(|accounts| accounts.first());
        match first {
            Some(account) => serde_json::json!({
                "signedIn": true,
                "name": account.get(2).and_then(|v| v.as_str()).unwrap_or_default(),
                "email": account.get(3).and_then(|v| v.as_str()).unwrap_or_default(),
            }),
            None => serde_json::json!({ "signedIn": false }),
        }
    }

    /// Asks Google, with the profile's own cookies, which account it's signed in as.
    fn ask_google() {
        if GOOGLE_REQUEST.with_borrow(|request| request.is_some()) || !STARTED.get() {
            return;
        }
        let Some(request) = cef::request_create() else { return };
        request.set_url(Some(&CefString::from("https://accounts.google.com/ListAccounts?gpsia=1&source=ChromiumBrowser&json=standard")));
        // A POST with a one-space body, as Chromium sends it; a GET is refused (400).
        request.set_method(Some(&CefString::from("POST")));
        if let (Some(mut body), Some(mut element)) = (cef::post_data_create(), cef::post_data_element_create()) {
            element.set_to_bytes(1, b" ".as_ptr());
            body.add_element(Some(&mut element));
            request.set_post_data(Some(&mut body));
        }
        request.set_header_by_name(Some(&CefString::from("Content-Type")), Some(&CefString::from("application/x-www-form-urlencoded")), 1);
        // Stored credentials (the profile's cookies), and never a cached answer.
        request.set_flags(ALLOW_STORED_CREDENTIALS | SKIP_CACHE);
        let mut request = request;
        let mut client = GoogleAccounts::new();
        GOOGLE_BODY.set(Some(Vec::new()));
        let started = cef::urlrequest_create(Some(&mut request), Some(&mut client), None);
        GOOGLE_REQUEST.set(started);
    }

    /// How many holes a page shows now (a mod's popup, Herdr's popups and toasts over it).
    pub fn holes(id: &str) -> usize {
        VIEWS.with(|views| views.try_borrow().ok().and_then(|views| views.get(id).map(|view| {
            MASK_HOLES.with(|holes| holes.borrow().get(&(view.host as usize)).copied().unwrap_or(0))
        })).unwrap_or(0))
    }

    /// The start of a frame's covers; `expected`: whether any popup or toast will report one.
    pub fn begin_covers(expected: bool) {
        COVERS.with(|covers| covers.borrow_mut().clear());
        if !expected {
            apply_covers();
        }
    }

    /// A popup or toast drawn at `bounds` this frame (window coordinates).
    pub fn cover(bounds: Bounds<Pixels>, pass_all: bool) {
        let area = [f64::from(f32::from(bounds.origin.x)), f64::from(f32::from(bounds.origin.y)),
            f64::from(f32::from(bounds.size.width)), f64::from(f32::from(bounds.size.height))];
        COVERS.with(|covers| covers.borrow_mut().push(Cover { area, pass_all }));
        apply_covers();
    }

    fn apply_covers() {
        let now = COVERS.with(|covers| covers.borrow().clone());
        if APPLIED_COVERS.with(|applied| *applied.borrow() == now) {
            return;
        }
        APPLIED_COVERS.set(now);
        VIEWS.with(|views| {
            if let Ok(views) = views.try_borrow() {
                views.values().for_each(apply_mask);
            }
        });
    }

    pub fn command(id: &str, op: &str, arg: &str) {
        match op {
            "blur" => return take_keys_back(),
            "focus" => return give_keys(id),
            // A popup over the page (its address, its menu) takes the keyboard while it's open.
            "hole" if arg.split_whitespace().count() == 4 => {
                if let Some(page) = page_with_keys() {
                    HELD_KEYS.with_borrow_mut(|held| *held = Some(page));
                    take_keys_back();
                }
            }
            _ => {}
        }
        if op == "remove" {
            if let Some(view) = VIEWS.with_borrow_mut(|views| views.remove(id)) {
                close_page(&view);
            }
            // The last private pane gone, its session goes with it: nothing of it is kept.
            PRIVATE_IDS.with_borrow_mut(|ids| ids.remove(id));
            let any_private = VIEWS.with_borrow(|views| views.keys().any(|key| PRIVATE_IDS.with_borrow(|ids| ids.contains(key))));
            if !any_private {
                PRIVATE_CONTEXT.take();
            }
            return;
        }
        // "on" or "off": the pane's page switches to the private session or back, in place. Before the
        // page exists it only says which it opens in.
        if op == "private" {
            let on = arg.trim() != "off";
            if VIEWS.with_borrow(|views| views.get(id).is_some_and(|view| view.browser.is_some())) {
                return switch_private(id, on);
            }
            PRIVATE_IDS.with_borrow_mut(|ids| if on { ids.insert(id.to_owned()) } else { ids.remove(id) });
            return;
        }
        if op == "google" {
            return ask_google();
        }
        let exists = VIEWS.with_borrow(|views| views.get(id).is_some_and(|view| view.browser.is_some()));
        if op == "navigate" && !exists {
            WAITING.with_borrow_mut(|waiting| waiting.insert(id.to_owned(), normalize(arg)));
            return;
        }
        // A preloaded page is already where its pane would send it: don't load it again.
        if op == "navigate" && VIEWS.with_borrow(|views| views.get(id).is_some_and(|view| view.url == normalize(arg))) {
            return;
        }
        let browser = VIEWS.with_borrow_mut(|views| {
            let view = views.get_mut(id)?;
            match op {
                "zoom" => zoom(view, arg),
                "clear-data" => clear_data(view),
                "hole" => set_hole(view, arg),
                "show" => view.shown = true,
                "hide" => view.shown = false,
                _ => {}
            }
            view.browser.clone()
        });
        if let Some(browser) = browser {
            match op {
                "navigate" if hold_while_settling(id, &normalize(arg)) => {}
                "navigate" => {
                    if let Some(frame) = browser.main_frame() {
                        frame.load_url(Some(&CefString::from(normalize(arg).as_str())));
                    }
                }
                "back" => browser.go_back(),
                "forward" => browser.go_forward(),
                "reload" => browser.reload(),
                "ublock" | "extensions" => {
                    let (folder, page) = if op == "ublock" { ("ublock".to_owned(), "dashboard.html") } else { (own_extension_folder("extensions-page"), "manage.html") };
                    if let (Some(id), Some(frame)) = (extension_id(&folder), browser.main_frame()) {
                        frame.load_url(Some(&CefString::from(format!("chrome-extension://{id}/{page}").as_str())));
                    }
                }
                "hole" => {
                    if arg.trim().is_empty() && HELD_KEYS.with_borrow(|held| held.as_deref() == Some(id)) {
                        HELD_KEYS.take();
                        give_keys(id);
                    }
                }
                "zoom" | "clear-data" | "show" | "hide" => {}
                other => tracing::warn!("web view: unknown command {other:?}"),
            }
        }
        apply_visibility();
    }
}

//! CEF's helper process: the renderer, GPU and utility processes behind every web page. It's packaged
//! into the app as "Herdr Helper" and its variants.

#[cfg(target_os = "macos")]
fn main() {
    let args = cef::args::Args::new();
    let exe = std::env::current_exe().expect("own path");
    let loader = cef::library_loader::LibraryLoader::new(&exe, true);
    assert!(loader.load(), "Chromium Embedded Framework not found");
    let _ = cef::api_hash(cef::sys::CEF_API_VERSION_LAST, 0);
    gpui_shell::web_view::helper_main();
}

#[cfg(not(target_os = "macos"))]
fn main() {}

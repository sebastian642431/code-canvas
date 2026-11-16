//! Browser views for extensions' webviews.
//!
//! A webview is an extension's own HTML interface (the Claude Code panel,
//! for one). The extension host owns the object the extension talks to
//! (`exthost/host.js`); this module shows the page: a child browser view
//! (WebView2 on Windows, through `wry`) placed over a panel of the window.
//!
//! * The page and its files come from `https://ccres.localhost`, a protocol
//!   served here: `/view/<handle>` is the HTML the extension set,
//!   `/file/<path>` a file from an allowed folder (extensions, open folders).
//! * A script added to every page provides `acquireVsCodeApi()` and the
//!   `--vscode-*` theme colours pages are styled with.
//! * Messages go both ways as JSON: page → `ipc` → [`WebViews::poll`];
//!   extension → [`WebViews::post`] → `window.postMessage` in the page.

use serde_json::Value;

/// What a page did: `(handle, message)`.
pub type PageMessage = (String, Value);

/// Runs in every page before its own scripts.
#[cfg(windows)]
const INIT_JS: &str = r#"
(function () {
  if (window.__codeCanvas) return;
  window.__codeCanvas = true;
  let state;
  const api = {
    postMessage: (message) => window.ipc.postMessage(JSON.stringify({ message })),
    getState: () => state,
    setState: (value) => (state = value),
  };
  window.acquireVsCodeApi = () => api;

  // Colours VS Code gives every webview as CSS variables. Pages use hundreds
  // of them; the ones this app defines are exact, the rest are guessed from
  // their name so that nothing is left transparent or black on black.
  const exact = {
    'font-family': '"Segoe UI", system-ui, sans-serif', 'font-size': '13px', 'font-weight': 'normal',
    'editor-font-family': 'Consolas, "Cascadia Mono", monospace', 'editor-font-size': '13px', 'editor-font-weight': 'normal',
    'foreground': '#e6e7ec', 'descriptionForeground': '#8a8e99', 'disabledForeground': '#5c606c', 'errorForeground': '#f7768e',
    'focusBorder': '#5a96ff', 'editor-background': '#101115', 'editor-foreground': '#e6e7ec',
    'sideBar-background': '#101115', 'sideBar-foreground': '#e6e7ec', 'panel-background': '#101115', 'panel-border': '#22252e',
    'input-background': '#0b0c0f', 'input-foreground': '#e6e7ec', 'input-border': '#22252e', 'input-placeholderForeground': '#5c606c',
    'button-background': '#5a96ff', 'button-foreground': '#080a10', 'button-hoverBackground': '#74a7ff',
    'button-secondaryBackground': '#1b1d24', 'button-secondaryForeground': '#e6e7ec', 'button-secondaryHoverBackground': '#22252e',
    'textLink-foreground': '#5a96ff', 'textLink-activeForeground': '#74a7ff', 'textCodeBlock-background': '#0b0c0f',
    'textPreformat-foreground': '#e0af68', 'textBlockQuote-background': '#0b0c0f', 'textBlockQuote-border': '#22252e',
    'list-hoverBackground': '#1b1d24', 'list-activeSelectionBackground': '#22252e', 'list-activeSelectionForeground': '#e6e7ec',
    'badge-background': '#22252e', 'badge-foreground': '#e6e7ec', 'scrollbarSlider-background': '#ffffff1f',
    'scrollbarSlider-hoverBackground': '#ffffff33', 'scrollbarSlider-activeBackground': '#ffffff4d', 'widget-shadow': '#00000080',
    'dropdown-background': '#16171c', 'dropdown-foreground': '#e6e7ec', 'dropdown-border': '#22252e',
    'menu-background': '#16171c', 'menu-foreground': '#e6e7ec', 'menu-selectionBackground': '#22252e',
    'editorWidget-background': '#16171c', 'editorWidget-border': '#22252e', 'toolbar-hoverBackground': '#ffffff14',
    'charts-red': '#f7768e', 'charts-green': '#3ddc84', 'charts-yellow': '#e0af68', 'charts-blue': '#5a96ff',
    'charts-orange': '#ff9e64', 'charts-purple': '#bb9af7', 'icon-foreground': '#c5c8d3',
    'gitDecoration-addedResourceForeground': '#3ddc84', 'gitDecoration-modifiedResourceForeground': '#5a96ff',
    'gitDecoration-deletedResourceForeground': '#f7768e',
  };
  function guess(name) {
    const n = name.toLowerCase();
    const tone = /error|deleted|removed/.test(n) ? '#f7768e' : /warning|modified/.test(n) ? '#e0af68' : /added|inserted|success/.test(n) ? '#3ddc84' : /info|link|focus|active|selection|highlight/.test(n) ? '#5a96ff' : null;
    if (/shadow/.test(n)) return '#00000080';
    if (/border|outline|separator/.test(n)) return tone || '#22252e';
    if (/background/.test(n)) return tone ? tone + '26' : /hover/.test(n) ? '#1b1d24' : '#101115';
    if (/foreground|color/.test(n)) return tone || '#e6e7ec';
    return null;
  }
  function applyTheme() {
    const root = document.documentElement;
    for (const [k, v] of Object.entries(exact)) root.style.setProperty('--vscode-' + k, v);
    const wanted = new Set();
    for (const sheet of document.styleSheets) {
      let rules;
      try { rules = sheet.cssRules; } catch { continue; }
      for (const rule of rules) for (const m of (rule.cssText || '').matchAll(/var\(--vscode-([A-Za-z0-9-]+)/g)) wanted.add(m[1]);
    }
    for (const name of wanted) {
      if (name in exact) continue;
      const value = guess(name);
      if (value) root.style.setProperty('--vscode-' + name, value);
    }
    if (document.body) {
      document.body.classList.add('vscode-dark');
      document.body.setAttribute('data-vscode-theme-kind', 'vscode-dark');
    }
  }
  document.addEventListener('DOMContentLoaded', applyTheme);
  window.addEventListener('load', () => {
    applyTheme();
    window.ipc.postMessage(JSON.stringify({ ready: true }));
  });
})();
"#;

#[cfg(windows)]
mod platform {
    use std::borrow::Cow;
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::sync::{Arc, Mutex};

    use canvas_core::Rect;
    use serde_json::Value;
    use wry::dpi::{PhysicalPosition, PhysicalSize};
    use wry::http::{header::CONTENT_TYPE, Request, Response};
    use wry::{WebView, WebViewBuilder, WebViewBuilderExtWindows};

    use super::{PageMessage, INIT_JS};

    const ORIGIN: &str = "https://ccres.localhost";

    /// State the protocol handler reads from the browser's thread.
    #[derive(Default)]
    struct Shared {
        /// Handle → the HTML the extension set.
        html: HashMap<String, String>,
        /// Folders files may be served from.
        roots: Vec<PathBuf>,
    }

    struct View {
        web: WebView,
        /// The page finished loading: messages can be delivered.
        ready: bool,
        /// Messages posted before that, in order.
        queued: Vec<String>,
    }

    pub struct WebViews {
        shared: Arc<Mutex<Shared>>,
        views: HashMap<String, View>,
        tx: Sender<(String, String)>,
        rx: Receiver<(String, String)>,
        shown: Option<String>,
        /// Where the shown view was last placed.
        placed: Option<Rect>,
    }

    fn mime(path: &Path) -> &'static str {
        match path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
            "html" | "htm" => "text/html; charset=utf-8",
            "js" | "mjs" => "text/javascript; charset=utf-8",
            "css" => "text/css; charset=utf-8",
            "json" | "map" => "application/json",
            "svg" => "image/svg+xml",
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "ico" => "image/x-icon",
            "woff" => "font/woff",
            "woff2" => "font/woff2",
            "ttf" => "font/ttf",
            "otf" => "font/otf",
            "wasm" => "application/wasm",
            "mp3" => "audio/mpeg",
            "wav" => "audio/wav",
            _ => "application/octet-stream",
        }
    }

    fn decode(path: &str) -> String {
        let bytes = path.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            let hex = (bytes[i] == b'%')
                .then(|| path.get(i + 1..i + 3))
                .flatten()
                .and_then(|h| u8::from_str_radix(h, 16).ok());
            match hex {
                Some(b) => {
                    out.push(b);
                    i += 3;
                }
                None => {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    fn respond(status: u16, kind: &str, body: Vec<u8>) -> Response<Cow<'static, [u8]>> {
        Response::builder()
            .status(status)
            .header(CONTENT_TYPE, kind)
            .header("Access-Control-Allow-Origin", "*")
            .body(Cow::Owned(body))
            .unwrap_or_else(|_| Response::new(Cow::Borrowed(&[][..])))
    }

    /// Answer one request of a page: its HTML, or a file from an allowed folder.
    fn serve(shared: &Mutex<Shared>, request: &Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
        let path = decode(request.uri().path());
        if let Some(handle) = path.strip_prefix("/view/") {
            return match shared.lock().unwrap().html.get(handle) {
                Some(html) => respond(200, "text/html; charset=utf-8", html.clone().into_bytes()),
                None => respond(404, "text/plain", b"no such view".to_vec()),
            };
        }
        let Some(file) = path.strip_prefix("/file/") else { return respond(404, "text/plain", Vec::new()) };
        let file = PathBuf::from(file);
        // Only what the extensions and the open folders contain; `..` cannot
        // leave them because the resolved path is what gets compared.
        let Ok(real) = file.canonicalize() else { return respond(404, "text/plain", Vec::new()) };
        let key = |p: &Path| crate::ext::path_key(&canvas_core::scan::clean_path(p.to_path_buf()).to_string_lossy());
        let real_key = key(&real);
        let allowed = shared.lock().unwrap().roots.iter().any(|root| {
            let mut prefix = key(root);
            prefix.push('/');
            real_key.starts_with(&prefix)
        });
        if !allowed {
            return respond(403, "text/plain", b"outside the allowed folders".to_vec());
        }
        match std::fs::read(&real) {
            Ok(bytes) => respond(200, mime(&real), bytes),
            Err(_) => respond(404, "text/plain", Vec::new()),
        }
    }

    fn bounds(r: Rect) -> wry::Rect {
        wry::Rect {
            position: PhysicalPosition::new(r.x.round() as i32, r.y.round() as i32).into(),
            size: PhysicalSize::new(r.w.round().max(1.0) as u32, r.h.round().max(1.0) as u32).into(),
        }
    }

    impl WebViews {
        pub fn new() -> Self {
            let (tx, rx) = mpsc::channel();
            WebViews { shared: Default::default(), views: HashMap::new(), tx, rx, shown: None, placed: None }
        }

        pub fn set_roots(&mut self, roots: Vec<PathBuf>) {
            self.shared.lock().unwrap().roots = roots.into_iter().map(|r| r.canonicalize().unwrap_or(r)).collect();
        }

        /// The extension set (or replaced) the page of `handle`.
        pub fn set_html(&mut self, handle: &str, html: String) {
            self.shared.lock().unwrap().html.insert(handle.to_string(), html);
            if let Some(view) = self.views.get_mut(handle) {
                view.ready = false;
                view.queued.clear();
                let _ = view.web.load_url(&format!("{ORIGIN}/view/{handle}"));
            }
        }

        pub fn has_html(&self, handle: &str) -> bool {
            self.shared.lock().unwrap().html.contains_key(handle)
        }

        /// The extension sent `message` to the page.
        pub fn post(&mut self, handle: &str, message: &Value) {
            let script = format!("window.postMessage({message}, '*')");
            match self.views.get_mut(handle) {
                Some(view) if view.ready => {
                    let _ = view.web.evaluate_script(&script);
                }
                Some(view) => view.queued.push(script),
                // Not on screen yet: nothing listens, and VS Code drops these too.
                None => {}
            }
        }

        pub fn close(&mut self, handle: &str) {
            self.views.remove(handle);
            self.shared.lock().unwrap().html.remove(handle);
            if self.shown.as_deref() == Some(handle) {
                self.shown = None;
            }
        }

        /// Show `handle` over `area` (physical pixels of the window), hiding
        /// whichever view was shown before; `None` hides them all.
        pub fn show(&mut self, window: &winit::window::Window, target: Option<(&str, Rect)>) {
            let wanted = target.map(|(h, _)| h.to_string());
            if self.shown != wanted {
                if let Some(old) = self.shown.take().and_then(|h| self.views.get(&h)) {
                    let _ = old.web.set_visible(false);
                }
            }
            let Some((handle, area)) = target else { return };
            if !self.views.contains_key(handle) {
                if !self.has_html(handle) {
                    return; // the extension has not produced the page yet
                }
                match self.create(window, handle, area) {
                    Ok(web) => {
                        self.views.insert(handle.to_string(), View { web, ready: false, queued: Vec::new() });
                    }
                    Err(e) => {
                        log::error!("webview {handle} could not be created: {e}");
                        return;
                    }
                }
            }
            let same = self.shown.as_deref() == Some(handle);
            if let Some(view) = self.views.get(handle) {
                if !same || self.placed != Some(area) {
                    let _ = view.web.set_bounds(bounds(area));
                }
                if !same {
                    let _ = view.web.set_visible(true);
                }
            }
            self.placed = Some(area);
            self.shown = Some(handle.to_string());
        }

        fn create(&self, window: &winit::window::Window, handle: &str, area: Rect) -> wry::Result<WebView> {
            let shared = self.shared.clone();
            let (tx, owner) = (self.tx.clone(), handle.to_string());
            WebViewBuilder::new()
                .with_bounds(bounds(area))
                .with_https_scheme(true)
                .with_custom_protocol("ccres".into(), move |_id, request| serve(&shared, &request))
                .with_initialization_script(INIT_JS)
                .with_ipc_handler(move |request: Request<String>| {
                    let _ = tx.send((owner.clone(), request.body().clone()));
                    crate::ext::wake_ui();
                })
                .with_background_color((16, 17, 21, 255))
                .with_url(format!("{ORIGIN}/view/{handle}"))
                .build_as_child(window)
        }

        /// What the pages sent since the last call.
        pub fn poll(&mut self) -> Vec<PageMessage> {
            let mut out = Vec::new();
            while let Ok((handle, body)) = self.rx.try_recv() {
                let Ok(v) = serde_json::from_str::<Value>(&body) else { continue };
                if v.get("ready").is_some() {
                    if let Some(view) = self.views.get_mut(&handle) {
                        view.ready = true;
                        for script in view.queued.drain(..) {
                            let _ = view.web.evaluate_script(&script);
                        }
                    }
                } else if let Some(message) = v.get("message") {
                    out.push((handle, message.clone()));
                }
            }
            out
        }
    }
}

/// Where there is no browser view to use yet, webviews stay empty.
#[cfg(not(windows))]
mod platform {
    use std::path::PathBuf;

    use canvas_core::Rect;
    use serde_json::Value;

    use super::PageMessage;

    pub struct WebViews;

    impl WebViews {
        pub fn new() -> Self {
            WebViews
        }
        pub fn set_roots(&mut self, _roots: Vec<PathBuf>) {}
        pub fn set_html(&mut self, _handle: &str, _html: String) {}
        pub fn post(&mut self, _handle: &str, _message: &Value) {}
        pub fn close(&mut self, _handle: &str) {}
        pub fn show(&mut self, _window: &winit::window::Window, _target: Option<(&str, Rect)>) {}
        pub fn poll(&mut self) -> Vec<PageMessage> {
            Vec::new()
        }
    }
}

pub use platform::WebViews;

/// Can this build show webviews?
pub const AVAILABLE: bool = cfg!(windows);

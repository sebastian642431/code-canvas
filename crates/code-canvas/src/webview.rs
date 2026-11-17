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

/// A key chord pressed while a page had the keyboard. Pages hand these over
/// (see `INIT_JS`) so that the app's shortcuts work wherever the caret is.
#[derive(Clone, Debug, PartialEq)]
pub struct PageKey {
    /// Lower case, as browsers name keys: `"p"`, `"f1"`, `"tab"`.
    pub key: String,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

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
  // of them; the ones below are taken from the app's theme (`T`, by the keys
  // of `canvas_render::theme`), the rest are guessed from their name so that
  // nothing is left transparent or black on black.
  let T = window.__ccTheme || {};
  const exact = () => ({
    'font-family': '"Segoe UI", system-ui, sans-serif', 'font-size': '13px', 'font-weight': 'normal',
    'editor-font-family': 'Consolas, "Cascadia Mono", monospace', 'editor-font-size': '13px', 'editor-font-weight': 'normal',
    'foreground': T.text, 'descriptionForeground': T.text_dim, 'disabledForeground': T.text_faint, 'errorForeground': T.problem_error,
    'focusBorder': T.accent, 'editor-background': T.card, 'editor-foreground': T.text,
    'sideBar-background': T.card, 'sideBar-foreground': T.text, 'panel-background': T.card, 'panel-border': T.border,
    'input-background': T.well, 'input-foreground': T.text, 'input-border': T.border, 'input-placeholderForeground': T.text_faint,
    'button-background': T.accent, 'button-foreground': T.on_accent, 'button-hoverBackground': T.accent_hover,
    'button-secondaryBackground': T.raised, 'button-secondaryForeground': T.text, 'button-secondaryHoverBackground': T.hover,
    'textLink-foreground': T.accent, 'textLink-activeForeground': T.accent_hover, 'textCodeBlock-background': T.well,
    'textPreformat-foreground': T.problem_warning, 'textBlockQuote-background': T.well, 'textBlockQuote-border': T.border,
    'list-hoverBackground': T.raised, 'list-activeSelectionBackground': T.hover, 'list-activeSelectionForeground': T.text,
    'badge-background': T.hover, 'badge-foreground': T.text, 'scrollbarSlider-background': '#ffffff1f',
    'scrollbarSlider-hoverBackground': '#ffffff33', 'scrollbarSlider-activeBackground': '#ffffff4d', 'widget-shadow': '#00000080',
    'dropdown-background': T.row, 'dropdown-foreground': T.text, 'dropdown-border': T.border,
    'menu-background': T.row, 'menu-foreground': T.text, 'menu-selectionBackground': T.hover,
    'editorWidget-background': T.row, 'editorWidget-border': T.border, 'toolbar-hoverBackground': '#ffffff14',
    'charts-red': T.problem_error, 'charts-green': T.git_added, 'charts-yellow': T.problem_warning, 'charts-blue': T.accent,
    'charts-orange': T.tok_number, 'charts-purple': T.tok_keyword, 'icon-foreground': T.icon,
    'gitDecoration-addedResourceForeground': T.git_added, 'gitDecoration-modifiedResourceForeground': T.git_modified,
    'gitDecoration-deletedResourceForeground': T.git_removed,
  });
  function guess(name) {
    const n = name.toLowerCase();
    const tone = /error|deleted|removed/.test(n) ? T.problem_error : /warning|modified/.test(n) ? T.problem_warning : /added|inserted|success/.test(n) ? T.git_added : /info|link|focus|active|selection|highlight/.test(n) ? T.accent : null;
    if (/shadow/.test(n)) return '#00000080';
    if (/border|outline|separator/.test(n)) return tone || T.border;
    // A tint of the tone: its `#rrggbb` with a low alpha.
    if (/background/.test(n)) return tone ? tone.slice(0, 7) + '26' : /hover/.test(n) ? T.raised : T.card;
    if (/foreground|color/.test(n)) return tone || T.text;
    return null;
  }
  function applyTheme() {
    const root = document.documentElement;
    const known = exact();
    for (const [k, v] of Object.entries(known)) root.style.setProperty('--vscode-' + k, v);
    const wanted = new Set();
    for (const sheet of document.styleSheets) {
      let rules;
      try { rules = sheet.cssRules; } catch { continue; }
      for (const rule of rules) for (const m of (rule.cssText || '').matchAll(/var\(--vscode-([A-Za-z0-9-]+)/g)) wanted.add(m[1]);
    }
    for (const name of wanted) {
      if (name in known) continue;
      const value = guess(name);
      if (value) root.style.setProperty('--vscode-' + name, value);
    }
    if (document.body) {
      document.body.classList.add('vscode-dark');
      document.body.setAttribute('data-vscode-theme-kind', 'vscode-dark');
    }
  }
  // VS Code also gives every webview a default style sheet, the scroll bars
  // among it; without one, pages show the browser's own grey bar with arrows.
  // The second half is this app's taste for the Claude Code panel: text that
  // does not fit is cut at a whole line instead of fading out. Its classes
  // end in a hash that changes with every version, hence `[class*=...]`.
  const css = `
    ::-webkit-scrollbar { width: 10px; height: 10px; }
    ::-webkit-scrollbar-track, ::-webkit-scrollbar-corner { background: transparent; }
    ::-webkit-scrollbar-button { display: none; }
    ::-webkit-scrollbar-thumb {
      background-color: var(--vscode-scrollbarSlider-background);
      background-clip: content-box;
      border: 3px solid transparent;
      border-radius: 5px;
    }
    ::-webkit-scrollbar-thumb:vertical { min-height: 32px; }
    ::-webkit-scrollbar-thumb:horizontal { min-width: 32px; }
    ::-webkit-scrollbar-thumb:hover { background-color: var(--vscode-scrollbarSlider-hoverBackground); border-width: 2px; }
    ::-webkit-scrollbar-thumb:active { background-color: var(--vscode-scrollbarSlider-activeBackground); border-width: 2px; }
    /* The standard properties switch the rules above off where a page sets them. */
    [class*="userMessageAttachments_"], [class*="attachedFilesContainerAbove_"] { scrollbar-width: auto !important; }
    [class*="root_"] { scrollbar-color: auto !important; }

    /* A long message of the user, collapsed: three whole lines, no veil. */
    [class*="truncationGradient_"] { display: none !important; }
    [class*="userMessage_"] [class*="content_"][class*="collapsed_"] { max-height: 3lh !important; }
    /* What a tool printed, collapsed: three whole lines of its smaller type
       (0.85em at 1.5). The 4px of padding below still show what overflows,
       so they come off the height. */
    [class*="toolBodyRowContent_"]:not([class*="toolBodyRowContent_disableClipping_"]) {
      -webkit-mask-image: none !important;
      mask-image: none !important;
      max-height: calc(3 * 1.5 * 0.85em - 4px) !important;
    }
    /* The message pinned at the top: a solid edge over what scrolls under it. */
    [class*="stickyHeader_"] { background-image: none !important; background-color: var(--sticky-bg) !important; }
  `;
  function addStyles() {
    if (document.getElementById('_defaultStyles')) return;
    const style = document.createElement('style');
    style.id = '_defaultStyles';
    style.textContent = css;
    // Last in the head: after the page's own sheets, so equal rules are ours.
    (document.head || document.documentElement).appendChild(style);
  }

  // The app calls this when its theme changes while the page is open.
  window.__codeCanvasTheme = (theme) => {
    T = theme;
    applyTheme();
  };

  // Key chords belong to the app (Ctrl+P, Ctrl+Shift+P...), as in VS Code,
  // where a webview hands them to the workbench. The ones text editing needs
  // stay here, and so does AltGr, which arrives as Ctrl+Alt.
  const editing = new Set(['a', 'c', 'v', 'x', 'z', 'y', 'insert', 'backspace', 'delete', 'enter', 'home', 'end',
    'pageup', 'pagedown', 'arrowleft', 'arrowright', 'arrowup', 'arrowdown', 'control', 'shift', 'alt', 'meta']);
  window.addEventListener('keydown', (e) => {
    const key = (e.key || '').toLowerCase();
    const ctrl = e.ctrlKey || e.metaKey;
    const chord = ctrl && !e.altKey && !editing.has(key);
    if (!chord && !/^f([1-9]|1[0-2])$/.test(key)) return;
    // The page's own handlers still run; only the browser's default does not.
    if (chord) e.preventDefault();
    window.ipc.postMessage(JSON.stringify({ key: { key, ctrl, shift: e.shiftKey, alt: e.altKey } }));
  }, true);

  document.addEventListener('DOMContentLoaded', () => {
    addStyles();
    applyTheme();
  });
  window.addEventListener('load', () => {
    addStyles();
    applyTheme();
    window.ipc.postMessage(JSON.stringify({ ready: true }));
  });
})();
"#;

/// The theme in use as the object `INIT_JS` reads: every colour of
/// `canvas_render::theme` by its key, as `#rrggbb` or `#rrggbbaa`, plus two
/// that pages need and are derived from those.
#[cfg(windows)]
fn theme_json() -> String {
    use canvas_render::theme;
    let t = theme::get();
    let mut colours = serde_json::Map::new();
    for field in theme::FIELDS {
        colours.insert(field.key.to_string(), Value::String(theme::to_hex((field.get)(&t))));
    }
    let derived =
        [("accent_hover", theme::mix(t.accent, 0xffffffff, 0.16)), ("icon", theme::mix(t.text, t.text_dim, 0.35))];
    for (key, colour) in derived {
        colours.insert(key.to_string(), Value::String(theme::to_hex(colour)));
    }
    Value::Object(colours).to_string()
}

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

    use super::{theme_json, PageKey, PageMessage, INIT_JS};

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
        /// The theme (`theme::revision`) its page was last given.
        theme: u64,
    }

    pub struct WebViews {
        shared: Arc<Mutex<Shared>>,
        views: HashMap<String, View>,
        tx: Sender<(String, String)>,
        rx: Receiver<(String, String)>,
        shown: Option<String>,
        /// Where the shown view was last placed.
        placed: Option<Rect>,
        /// Key chords the pages handed over since [`WebViews::take_keys`].
        keys: Vec<PageKey>,
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
            WebViews {
                shared: Default::default(),
                views: HashMap::new(),
                tx,
                rx,
                shown: None,
                placed: None,
                keys: Vec::new(),
            }
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
                        let theme = canvas_render::theme::revision();
                        self.views.insert(handle.to_string(), View { web, ready: false, queued: Vec::new(), theme });
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
                // The theme first: the script reads it while it sets itself up.
                .with_initialization_script(format!("window.__ccTheme = {};\n{INIT_JS}", theme_json()))
                // The browser's own shortcuts (print, find, reload) are not ours.
                .with_browser_accelerator_keys(false)
                .with_ipc_handler(move |request: Request<String>| {
                    let _ = tx.send((owner.clone(), request.body().clone()));
                    crate::ext::wake_ui();
                })
                .with_background_color({
                    // What shows until the page paints: the panel's own colour.
                    let [r, g, b, _] = canvas_render::theme::unpack(canvas_render::theme::get().card);
                    (r, g, b, 255)
                })
                .with_url(format!("{ORIGIN}/view/{handle}"))
                .build_as_child(window)
        }

        /// Give every open page the theme in use, if it changed since the
        /// page got one. A page that is still loading gets it when it is
        /// ready: call this again then (it is cheap when nothing is due).
        pub fn retheme(&mut self) {
            let revision = canvas_render::theme::revision();
            if self.views.values().all(|v| v.theme == revision) {
                return;
            }
            let script = format!("window.__codeCanvasTheme && window.__codeCanvasTheme({})", theme_json());
            for view in self.views.values_mut().filter(|v| v.ready && v.theme != revision) {
                view.theme = revision;
                let _ = view.web.evaluate_script(&script);
            }
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
                } else if let Some(k) = v.get("key") {
                    let flag = |name: &str| k.get(name).and_then(Value::as_bool).unwrap_or(false);
                    if let Some(key) = k.get("key").and_then(Value::as_str) {
                        self.keys.push(PageKey {
                            key: key.to_string(),
                            ctrl: flag("ctrl"),
                            shift: flag("shift"),
                            alt: flag("alt"),
                        });
                    }
                }
            }
            out
        }

        /// The key chords pages handed over; [`WebViews::poll`] collects them.
        pub fn take_keys(&mut self) -> Vec<PageKey> {
            std::mem::take(&mut self.keys)
        }

        /// Give the keyboard back to the window, so that what is typed next
        /// reaches the app and not the page.
        pub fn focus_app(&self) {
            if let Some(view) = self.shown.as_ref().and_then(|h| self.views.get(h)) {
                let _ = view.web.focus_parent();
            }
        }
    }
}

/// Where there is no browser view to use yet, webviews stay empty.
#[cfg(not(windows))]
mod platform {
    use std::path::PathBuf;

    use canvas_core::Rect;
    use serde_json::Value;

    use super::{PageKey, PageMessage};

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
        pub fn take_keys(&mut self) -> Vec<PageKey> {
            Vec::new()
        }
        pub fn focus_app(&self) {}
        pub fn retheme(&mut self) {}
    }
}

pub use platform::WebViews;

/// Can this build show webviews?
pub const AVAILABLE: bool = cfg!(windows);

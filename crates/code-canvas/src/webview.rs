use serde_json::Value;

pub type PageMessage = (String, Value);

#[derive(Clone, Debug, PartialEq)]
pub struct PageKey {
    pub key: String,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

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
    [class*="userMessageAttachments_"], [class*="attachedFilesContainerAbove_"] { scrollbar-width: auto !important; }
    [class*="root_"] { scrollbar-color: auto !important; }

    [class*="truncationGradient_"] { display: none !important; }
    [class*="userMessage_"] [class*="content_"][class*="collapsed_"] { max-height: 3lh !important; }
    [class*="toolBodyRowContent_"]:not([class*="toolBodyRowContent_disableClipping_"]) {
      -webkit-mask-image: none !important;
      mask-image: none !important;
      max-height: calc(3 * 1.5 * 0.85em - 4px) !important;
    }
    [class*="stickyHeader_"] { background-image: none !important; background-color: var(--sticky-bg) !important; }
  `;
  function addStyles() {
    if (document.getElementById('_defaultStyles')) return;
    const style = document.createElement('style');
    style.id = '_defaultStyles';
    style.textContent = css;
    (document.head || document.documentElement).appendChild(style);
  }

  window.__codeCanvasTheme = (theme) => {
    T = theme;
    applyTheme();
  };

  let follow = window.__ccFollow !== false;
  window.__codeCanvasFollow = (on) => {
    follow = !!on;
  };
  let lastGesture = 0;
  const gesture = () => {
    lastGesture = Date.now();
  };
  for (const kind of ['wheel', 'touchmove', 'pointerdown', 'keydown']) {
    window.addEventListener(kind, gesture, { capture: true, passive: true });
  }
  const followed = new WeakSet();
  function followScroll(box) {
    if (followed.has(box)) return;
    followed.add(box);
    let held = true;
    const gap = () => box.scrollHeight - box.scrollTop - box.clientHeight;
    box.addEventListener(
      'scroll',
      () => {
        if (gap() < 48) held = true;
        else if (Date.now() - lastGesture < 1500) held = false;
      },
      { passive: true },
    );
    const toEnd = () => {
      if (follow && held && gap() > 1) box.scrollTop = box.scrollHeight;
    };
    new MutationObserver(toEnd).observe(box, { childList: true, subtree: true, characterData: true });
    new ResizeObserver(toEnd).observe(box);
  }
  function findConversations() {
    document.querySelectorAll('[class*="messagesContainer_"]').forEach(followScroll);
  }
  document.addEventListener('DOMContentLoaded', () => {
    findConversations();
    new MutationObserver(findConversations).observe(document.documentElement, { childList: true, subtree: true });
  });

  const editing = new Set(['a', 'c', 'v', 'x', 'z', 'y', 'insert', 'backspace', 'delete', 'enter', 'home', 'end',
    'pageup', 'pagedown', 'arrowleft', 'arrowright', 'arrowup', 'arrowdown', 'control', 'shift', 'alt', 'meta']);
  window.addEventListener('keydown', (e) => {
    const key = (e.key || '').toLowerCase();
    const ctrl = e.ctrlKey || e.metaKey;
    const chord = ctrl && !e.altKey && !editing.has(key);
    if (!chord && !/^f([1-9]|1[0-2])$/.test(key)) return;
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

    #[derive(Default)]
    struct Shared {
        html: HashMap<String, String>,
        roots: Vec<PathBuf>,
        files: Vec<PathBuf>,
    }

    struct View {
        web: WebView,
        ready: bool,
        queued: Vec<String>,
        theme: u64,
    }

    pub struct WebViews {
        shared: Arc<Mutex<Shared>>,
        views: HashMap<String, View>,
        tx: Sender<(String, String)>,
        rx: Receiver<(String, String)>,
        shown: Vec<(String, Rect)>,
        keys: Vec<PageKey>,
        links: Vec<String>,
        follow: bool,
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
            "ogg" | "opus" => "audio/ogg",
            "m4a" | "aac" => "audio/mp4",
            "flac" => "audio/flac",
            "mp4" | "m4v" => "video/mp4",
            "webm" => "video/webm",
            "mov" => "video/quicktime",
            "mkv" => "video/x-matroska",
            "ogv" => "video/ogg",
            "bmp" => "image/bmp",
            "avif" => "image/avif",
            "pdf" => "application/pdf",
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
        let Ok(real) = file.canonicalize() else { return respond(404, "text/plain", Vec::new()) };
        let key = |p: &Path| crate::ext::path_key(&canvas_core::scan::clean_path(p.to_path_buf()).to_string_lossy());
        let real_key = key(&real);
        let allowed = {
            let shared = shared.lock().unwrap();
            shared.files.iter().any(|f| key(f) == real_key)
                || shared.roots.iter().any(|root| {
                    let mut prefix = key(root);
                    prefix.push('/');
                    real_key.starts_with(&prefix)
                })
        };
        if !allowed {
            return respond(403, "text/plain", b"outside the allowed folders".to_vec());
        }
        let range = request.headers().get("Range").and_then(|v| v.to_str().ok()).and_then(parse_range);
        match range {
            Some((from, to)) => serve_range(&real, from, to),
            None => match std::fs::read(&real) {
                Ok(bytes) => respond(200, mime(&real), bytes),
                Err(_) => respond(404, "text/plain", Vec::new()),
            },
        }
    }

    fn parse_range(header: &str) -> Option<(u64, Option<u64>)> {
        let (from, to) = header.trim().strip_prefix("bytes=")?.split(',').next()?.split_once('-')?;
        Some((from.trim().parse().ok()?, to.trim().parse().ok()))
    }

    fn serve_range(path: &Path, from: u64, to: Option<u64>) -> Response<Cow<'static, [u8]>> {
        use std::io::{Read, Seek, SeekFrom};
        const PIECE: u64 = 4 * 1024 * 1024;
        let Ok(mut file) = std::fs::File::open(path) else { return respond(404, "text/plain", Vec::new()) };
        let size = file.metadata().map(|m| m.len()).unwrap_or(0);
        if from >= size {
            return Response::builder()
                .status(416)
                .header("Content-Range", format!("bytes */{size}"))
                .body(Cow::Borrowed(&[][..]))
                .unwrap_or_else(|_| Response::new(Cow::Borrowed(&[][..])));
        }
        let last = to.unwrap_or(u64::MAX).min(size - 1).min(from + PIECE - 1);
        let mut bytes = vec![0u8; (last - from + 1) as usize];
        if file.seek(SeekFrom::Start(from)).is_err() || file.read_exact(&mut bytes).is_err() {
            return respond(404, "text/plain", Vec::new());
        }
        Response::builder()
            .status(206)
            .header(CONTENT_TYPE, mime(path))
            .header("Accept-Ranges", "bytes")
            .header("Content-Range", format!("bytes {from}-{last}/{size}"))
            .header("Access-Control-Allow-Origin", "*")
            .body(Cow::Owned(bytes))
            .unwrap_or_else(|_| Response::new(Cow::Borrowed(&[][..])))
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
                shown: Vec::new(),
                keys: Vec::new(),
                links: Vec::new(),
                follow: true,
            }
        }

        pub fn set_roots(&mut self, roots: Vec<PathBuf>) {
            self.shared.lock().unwrap().roots = roots.into_iter().map(|r| r.canonicalize().unwrap_or(r)).collect();
        }

        pub fn allow_file(&mut self, file: &Path) {
            let file = file.canonicalize().unwrap_or_else(|_| file.to_path_buf());
            let mut shared = self.shared.lock().unwrap();
            if !shared.files.contains(&file) {
                shared.files.push(file);
                if shared.files.len() > 64 {
                    shared.files.remove(0);
                }
            }
        }

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

        pub fn post(&mut self, handle: &str, message: &Value) {
            let script = format!("window.postMessage({message}, '*')");
            match self.views.get_mut(handle) {
                Some(view) if view.ready => {
                    let _ = view.web.evaluate_script(&script);
                }
                Some(view) => view.queued.push(script),
                None => {}
            }
        }

        pub fn run_in(&mut self, handle: &str, script: &str) {
            if let Some(view) = self.views.get(handle) {
                let _ = view.web.evaluate_script(script);
            }
        }

        pub fn run(&mut self, script: &str) {
            for (handle, _) in &self.shown {
                if let Some(view) = self.views.get(handle) {
                    let _ = view.web.evaluate_script(script);
                }
            }
        }

        pub fn close(&mut self, handle: &str) {
            self.views.remove(handle);
            self.shared.lock().unwrap().html.remove(handle);
            self.shown.retain(|(shown, _)| shown != handle);
        }

        pub fn show(&mut self, window: &winit::window::Window, targets: &[(&str, Rect)]) {
            let (kept, gone): (Vec<_>, Vec<_>) = std::mem::take(&mut self.shown)
                .into_iter()
                .partition(|(shown, _)| targets.iter().any(|(wanted, _)| wanted == shown));
            for (handle, _) in gone {
                if let Some(view) = self.views.get(&handle) {
                    let _ = view.web.set_visible(false);
                }
            }
            for (handle, area) in targets.iter().copied() {
                if !self.views.contains_key(handle) {
                    if !self.has_html(handle) {
                        continue;
                    }
                    match self.create(window, handle, area) {
                        Ok(web) => {
                            let theme = canvas_render::theme::revision();
                            self.views
                                .insert(handle.to_string(), View { web, ready: false, queued: Vec::new(), theme });
                        }
                        Err(e) => {
                            log::error!("webview {handle} could not be created: {e}");
                            continue;
                        }
                    }
                }
                let placed = kept.iter().find(|(shown, _)| shown == handle).map(|(_, at)| *at);
                if let Some(view) = self.views.get(handle) {
                    if placed != Some(area) {
                        let _ = view.web.set_bounds(bounds(area));
                    }
                    if placed.is_none() {
                        let _ = view.web.set_visible(true);
                    }
                }
                self.shown.push((handle.to_string(), area));
            }
        }

        fn create(&self, window: &winit::window::Window, handle: &str, area: Rect) -> wry::Result<WebView> {
            let shared = self.shared.clone();
            let (tx, owner) = (self.tx.clone(), handle.to_string());
            WebViewBuilder::new()
                .with_bounds(bounds(area))
                .with_https_scheme(true)
                .with_custom_protocol("ccres".into(), move |_id, request| serve(&shared, &request))
                .with_initialization_script(format!(
                    "window.__ccTheme = {};\nwindow.__ccFollow = {};\n{INIT_JS}",
                    theme_json(),
                    self.follow
                ))
                .with_browser_accelerator_keys(false)
                .with_navigation_handler({
                    let (tx, owner) = (self.tx.clone(), handle.to_string());
                    let own = format!("{ORIGIN}/view/{handle}");
                    move |url: String| {
                        let elsewhere = url.split('#').next() != Some(own.as_str());
                        let web = url.starts_with("https://") || url.starts_with("http://");
                        if owner != crate::media::HANDLE || !elsewhere || !web {
                            return true;
                        }
                        let _ = tx.send((owner.clone(), serde_json::json!({ "navigate": url }).to_string()));
                        crate::ext::wake_ui();
                        false
                    }
                })
                .with_ipc_handler(move |request: Request<String>| {
                    let _ = tx.send((owner.clone(), request.body().clone()));
                    crate::ext::wake_ui();
                })
                .with_background_color({
                    let [r, g, b, _] = canvas_render::theme::unpack(canvas_render::theme::get().card);
                    (r, g, b, 255)
                })
                .with_url(format!("{ORIGIN}/view/{handle}"))
                .build_as_child(window)
        }

        pub fn set_follow(&mut self, follow: bool) {
            self.follow = follow;
            let script = format!("window.__codeCanvasFollow && window.__codeCanvasFollow({follow})");
            for view in self.views.values() {
                let _ = view.web.evaluate_script(&script);
            }
        }

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
                } else if let Some(url) = v.get("navigate").and_then(Value::as_str) {
                    if handle == crate::media::HANDLE {
                        self.links.push(url.to_string());
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

        pub fn take_keys(&mut self) -> Vec<PageKey> {
            std::mem::take(&mut self.keys)
        }

        pub fn take_links(&mut self) -> Vec<String> {
            std::mem::take(&mut self.links)
        }

        pub fn focus_page(&self) {
            if let Some(view) = self.shown.first().and_then(|(handle, _)| self.views.get(handle)) {
                let _ = view.web.focus();
            }
        }

        pub fn focus_app(&self) {
            let shown = self.shown.first().and_then(|(handle, _)| self.views.get(handle));
            if let Some(view) = shown.or_else(|| self.views.values().next()) {
                let _ = view.web.focus_parent();
            }
        }
    }
}

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
        pub fn allow_file(&mut self, _file: &std::path::Path) {}
        pub fn set_html(&mut self, _handle: &str, _html: String) {}
        pub fn post(&mut self, _handle: &str, _message: &Value) {}
        pub fn close(&mut self, _handle: &str) {}
        pub fn run(&mut self, _script: &str) {}
        pub fn run_in(&mut self, _handle: &str, _script: &str) {}
        pub fn show(&mut self, _window: &winit::window::Window, _targets: &[(&str, Rect)]) {}
        pub fn poll(&mut self) -> Vec<PageMessage> {
            Vec::new()
        }
        pub fn take_keys(&mut self) -> Vec<PageKey> {
            Vec::new()
        }
        pub fn take_links(&mut self) -> Vec<String> {
            Vec::new()
        }
        pub fn focus_app(&self) {}
        pub fn focus_page(&self) {}
        pub fn retheme(&mut self) {}
        pub fn set_follow(&mut self, _follow: bool) {}
    }
}

pub use platform::WebViews;

pub const AVAILABLE: bool = cfg!(windows);

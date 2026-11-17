//! Code Canvas — see a whole codebase on an infinite, GPU-rendered canvas.
//!
//! Usage: `code-canvas [FOLDER ...]` — every folder opens in its own tab.
//! Without arguments the previous session is restored.

// A GUI app: no console window on Windows release builds. Logs go to a file.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod bench;
mod camera;
mod codicons;
mod commit_ai;
mod editor;
mod ext;
mod files;
mod git;
mod icon;
mod icon_theme;
mod icons;
mod keys;
mod overlay;
mod page;
mod script;
mod session;
mod themes;
mod ui;
mod watch;
mod webview;

use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

/// Writes log lines to the log file and, when there is one, to stderr.
struct Tee(Mutex<Option<std::fs::File>>);

impl Write for Tee {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Some(f) = self.0.lock().unwrap().as_mut() {
            let _ = f.write_all(buf);
        }
        let _ = std::io::stderr().write_all(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if let Some(f) = self.0.lock().unwrap().as_mut() {
            let _ = f.flush();
        }
        Ok(())
    }
}

fn log_path() -> PathBuf {
    session::data_dir().join("code-canvas.log")
}

fn init_logging() {
    let file = std::fs::File::create(log_path()).ok();
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info,wgpu_core=warn,wgpu_hal=warn,naga=warn,egui_wgpu=warn"),
    )
    .target(env_logger::Target::Pipe(Box::new(Tee(Mutex::new(file)))))
    .init();

    // Without a console a panic would vanish silently: log it and tell the user.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("panic: {info}");
        default_hook(info);
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title("Code Canvas crashed")
            .set_description(format!("{info}\n\nDetails were written to:\n{}", log_path().display()))
            .show();
    }));
}

/// `code-canvas --install-extension <publisher.name | file.vsix> ...`:
/// install extensions without opening a window (like `code` does). The
/// outcome goes to the log; the exit code is the number of failures.
fn install_extensions(args: &[String]) -> i32 {
    let dir = session::data_dir().join("extensions");
    let mut failures = 0;
    let mut wanted = args.iter().skip_while(|a| *a != "--install-extension").filter(|a| !a.starts_with("--"));
    for what in &mut wanted {
        let done = match what.split_once('.') {
            _ if what.to_lowercase().ends_with(".vsix") => ext::market::install_vsix(what.as_ref(), &dir),
            Some((namespace, name)) => ext::market::install(namespace, name, &dir),
            None => Err("expected publisher.name or a .vsix file".to_string()),
        };
        match done {
            Ok(path) => log::info!("installed {what} in {}", path.display()),
            Err(e) => {
                failures += 1;
                log::error!("{what} was not installed: {e}");
            }
        }
    }
    failures
}

fn main() -> anyhow::Result<()> {
    init_logging();
    log::info!("Code Canvas {} — log: {}", env!("CARGO_PKG_VERSION"), log_path().display());

    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--install-extension") {
        std::process::exit(install_extensions(&args));
    }
    let mut paths: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    paths.retain(|p| {
        let ok = p.is_dir();
        if !ok {
            log::warn!("not a folder, ignored: {}", p.display());
        }
        ok
    });

    let event_loop = winit::event_loop::EventLoop::new()?;
    // Background work (the extension host) wakes the UI thread through this.
    let proxy = event_loop.create_proxy();
    let waker = ext::Waker::new(move || {
        let _ = proxy.send_event(());
    });
    ext::set_ui_waker(waker.clone());
    let mut app = app::App::new(paths, waker);
    event_loop.run_app(&mut app)?;
    Ok(())
}

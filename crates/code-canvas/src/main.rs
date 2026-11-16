//! Code Canvas — see a whole codebase on an infinite, GPU-rendered canvas.
//!
//! Usage: `code-canvas [FOLDER ...]` — every folder opens in its own tab.
//! Without arguments the previous session is restored.

// A GUI app: no console window on Windows release builds. Logs go to a file.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod bench;
mod camera;
mod claude;
mod icon;
mod icons;
mod overlay;
mod page;
mod script;
mod session;
mod ui;

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

fn main() -> anyhow::Result<()> {
    init_logging();
    log::info!("Code Canvas {} — log: {}", env!("CARGO_PKG_VERSION"), log_path().display());

    let mut paths: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    paths.retain(|p| {
        let ok = p.is_dir();
        if !ok {
            log::warn!("not a folder, ignored: {}", p.display());
        }
        ok
    });

    let event_loop = winit::event_loop::EventLoop::new()?;
    let mut app = app::App::new(paths);
    event_loop.run_app(&mut app)?;
    Ok(())
}

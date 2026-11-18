#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod account;
mod agents;
mod app;
mod bench;
mod camera;
mod codex;
mod codicons;
mod commit_ai;
mod consoles;
mod doodle;
mod editor;
mod ext;
mod files;
mod git;
mod github;
mod grammars;
mod history;
mod icon;
mod icon_theme;
mod icons;
mod keys;
mod markdown;
mod media;
mod models;
mod overlay;
mod page;
mod paths;
mod pictures;
mod plist;
mod prefs;
mod rails;
mod rebase;
mod script;
mod session;
mod share;
mod sync;
mod term;
mod themes;
mod tree_ops;
mod trust;
mod ui;
mod update;
mod versions;
mod watch;
mod webview;
mod window_place;

use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

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

    let default_hook = std::panic::take_hook();
    // A test instance (a QA script or the benchmark) only writes the crash to its log: a window would pop up on
    // the screen of whoever is working while the tests run.
    let testing = std::env::var_os("CODE_CANVAS_SCRIPT").is_some() || std::env::var_os("CODE_CANVAS_BENCH").is_some();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("panic: {info}");
        default_hook(info);
        if testing {
            return;
        }
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title("Code Canvas crashed")
            .set_description(format!("{info}\n\nDetails were written to:\n{}", log_path().display()))
            .show();
    }));
}

fn install_extensions(args: &[String]) -> i32 {
    let dir = session::data_dir().join("extensions");
    let mut failures = 0;
    let mut wanted = args.iter().skip_while(|a| *a != "--install-extension").filter(|a| !a.starts_with("--"));
    for what in &mut wanted {
        let done = match what.split_once('.') {
            _ if what.to_lowercase().ends_with(".vsix") => ext::market::install_vsix(what.as_ref(), &dir),
            Some((namespace, name)) => match name.split_once('@') {
                Some((name, version)) => ext::market::install(namespace, name, Some(version), &dir),
                None => ext::market::install(namespace, name, None, &dir),
            },
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
    let asked: Vec<String> = std::env::args().skip(1).take(2).collect();
    if asked.first().is_some_and(|flag| flag == "--agent-hook") {
        std::process::exit(agents::hook(asked.get(1).map_or("assistant", String::as_str)));
    }
    if let Some(on) = asked.first().and_then(|flag| match flag.as_str() {
        "--connect-claude" => Some(true),
        "--disconnect-claude" => Some(false),
        _ => None,
    }) {
        std::process::exit(match agents::connect_claude(on) {
            Ok(()) => 0,
            Err(why) => {
                eprintln!("{why}");
                1
            }
        });
    }
    init_logging();
    log::info!("Code Canvas {} — log: {}", env!("CARGO_PKG_VERSION"), log_path().display());
    update::clean_up();

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
    let proxy = event_loop.create_proxy();
    let waker = ext::Waker::new(move || {
        let _ = proxy.send_event(());
    });
    ext::set_ui_waker(waker.clone());
    let mut app = app::App::new(paths, waker);
    event_loop.run_app(&mut app)?;
    Ok(())
}

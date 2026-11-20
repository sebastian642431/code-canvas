use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use serde_json::Value;

pub const REPOSITORY: &str = "sebastian642431/hawk-canvas";
const FORMER_REPOSITORY: &str = "sebastian642431/code-canvas";
/// The program in a release; releases from before the name Hawk Canvas carry only the second.
const ASSETS: [&str; 2] = ["hawk-canvas.exe", "code-canvas.exe"];
const EVERY: Duration = Duration::from_secs(6 * 3600);

pub fn current() -> &'static str {
    static VERSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    VERSION.get_or_init(|| {
        std::env::var("CODE_CANVAS_PRETEND_VERSION").unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_string())
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: String,
    pub page: String,
    pub download: Option<String>,
}

fn numbers(version: &str) -> Vec<u64> {
    let core = version.trim().trim_start_matches('v').split(['-', '+']).next().unwrap_or("");
    core.split('.').map_while(|part| part.parse().ok()).collect()
}

pub fn newer(current: &str, candidate: &str) -> bool {
    let pre_release = candidate.split('+').next().unwrap_or("").contains('-');
    let (now, then) = (numbers(current), numbers(candidate));
    !pre_release && !then.is_empty() && then > now
}

pub fn parse_release(json: &Value) -> Option<Release> {
    let text = |key: &str| json.get(key).and_then(Value::as_str);
    if json.get("draft").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let version = text("tag_name")?.trim_start_matches('v').to_string();
    let wanted = format!("https://github.com/{REPOSITORY}/releases/download/");
    let former = format!("https://github.com/{FORMER_REPOSITORY}/releases/download/");
    let assets = json.get("assets").and_then(Value::as_array).map_or(&[][..], |assets| assets.as_slice());
    let download = ASSETS.iter().find_map(|name| {
        assets
            .iter()
            .filter(|a| a.get("name").and_then(Value::as_str) == Some(*name))
            .filter_map(|a| a.get("browser_download_url").and_then(Value::as_str))
            .find(|url| url.starts_with(&wanted) || url.starts_with(&former))
            .map(str::to_string)
    });
    let page =
        text("html_url").map(str::to_string).unwrap_or_else(|| format!("https://github.com/{REPOSITORY}/releases"));
    Some(Release { version, page, download: download.filter(|_| cfg!(windows)) })
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(600))).build().into()
}

const USER_AGENT: &str = concat!("hawk-canvas/", env!("CARGO_PKG_VERSION"));

fn latest() -> Result<Release, String> {
    let url = format!("https://api.github.com/repos/{REPOSITORY}/releases/latest");
    let mut response = agent()
        .get(&url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())?;
    let body = response.body_mut().read_to_string().map_err(|e| e.to_string())?;
    let json: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    parse_release(&json).ok_or_else(|| "no release".to_string())
}

const ASIDES: usize = 8;

fn aside(exe: &Path, nth: usize) -> PathBuf {
    match nth {
        0 => exe.with_extension("old.exe"),
        nth => exe.with_extension(format!("old-{nth}.exe")),
    }
}

pub fn swap(exe: &Path, fresh: &Path) -> Result<(), String> {
    let mut failed = String::new();
    for old in (0..ASIDES).map(|nth| aside(exe, nth)) {
        let _ = std::fs::remove_file(&old);
        match std::fs::rename(exe, &old) {
            Ok(()) => {
                if let Err(e) = std::fs::rename(fresh, exe) {
                    let _ = std::fs::rename(&old, exe);
                    return Err(format!("the new version could not be put in place: {e}"));
                }
                return Ok(());
            }
            Err(e) => failed = e.to_string(),
        }
    }
    Err(format!("the running program could not be moved aside: {failed}"))
}

pub fn clean_up() {
    if let Ok(exe) = std::env::current_exe() {
        for nth in 0..ASIDES {
            let _ = std::fs::remove_file(aside(&exe, nth));
        }
        let fresh = exe.with_extension("new.exe");
        let age = std::fs::metadata(&fresh).and_then(|file| file.modified()).ok().and_then(|at| at.elapsed().ok());
        if age.is_some_and(|age| age > std::time::Duration::from_secs(3600)) {
            let _ = std::fs::remove_file(fresh);
        }
    }
}

fn install(release: &Release) -> Result<PathBuf, String> {
    use std::io::Read;
    let url = release.download.as_deref().ok_or("this release has no program for this system")?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let fresh = exe.with_extension("new.exe");
    let mut response = agent().get(url).header("User-Agent", USER_AGENT).call().map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    const LIMIT: u64 = 400 * 1024 * 1024;
    response.body_mut().as_reader().take(LIMIT).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() < 1024 * 1024 || !bytes.starts_with(b"MZ") {
        return Err("what was downloaded is not the program".to_string());
    }
    std::fs::write(&fresh, &bytes).map_err(|e| format!("the download could not be saved: {e}"))?;
    swap(&exe, &fresh)?;
    Ok(exe)
}

pub fn open_page(url: &str) {
    let mut command = if cfg!(windows) {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", "", url]);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg(url);
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(url);
        c
    };
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    if let Err(e) = command.spawn() {
        log::warn!("could not open {url}: {e}");
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub enum State {
    #[default]
    Unknown,
    Checking,
    UpToDate,
    Available(String),
    Installing(String),
    Failed(String),
}

pub enum News {
    Available(Release),
    UpToDate,
    CheckFailed(String),
    Installed(PathBuf),
    Failed(String),
}

pub struct Updater {
    enabled: bool,
    checked: Option<Instant>,
    check: Option<Receiver<Result<Release, String>>>,
    installing: Option<Receiver<Result<PathBuf, String>>>,
    pub offer: Option<Release>,
    told: Option<String>,
    asked: bool,
    last: Option<Result<(), String>>,
    waker: fn(),
}

impl Updater {
    pub fn new(enabled: bool, waker: fn()) -> Self {
        Updater {
            enabled,
            checked: None,
            check: None,
            installing: None,
            offer: None,
            told: None,
            asked: false,
            last: None,
            waker,
        }
    }

    pub fn check_now(&mut self) {
        self.checked = None;
        self.told = None;
        self.asked = true;
    }

    pub fn state(&self) -> State {
        if self.installing.is_some() {
            return State::Installing(self.offer.as_ref().map(|r| r.version.clone()).unwrap_or_default());
        }
        if self.check.is_some() {
            return State::Checking;
        }
        match (&self.offer, &self.last) {
            (Some(release), _) => State::Available(release.version.clone()),
            (None, Some(Ok(()))) => State::UpToDate,
            (None, Some(Err(why))) => State::Failed(why.clone()),
            (None, None) => State::Unknown,
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn poll(&mut self) -> Option<News> {
        if let Some(rx) = &self.installing {
            if let Ok(done) = rx.try_recv() {
                self.installing = None;
                return Some(match done {
                    Ok(exe) => News::Installed(exe),
                    Err(why) => News::Failed(why),
                });
            }
        }
        if let Some(rx) = &self.check {
            if let Ok(found) = rx.try_recv() {
                self.check = None;
                let asked = std::mem::take(&mut self.asked);
                match found {
                    Ok(release) if newer(current(), &release.version) => {
                        let fresh = self.told.as_deref() != Some(release.version.as_str());
                        self.offer = Some(release.clone());
                        self.last = Some(Ok(()));
                        if fresh {
                            self.told = Some(release.version.clone());
                            return Some(News::Available(release));
                        }
                    }
                    Ok(_) => {
                        self.offer = None;
                        self.last = Some(Ok(()));
                        if asked {
                            return Some(News::UpToDate);
                        }
                    }
                    Err(why) => {
                        log::debug!("looking for updates: {why}");
                        self.last = Some(Err(why.clone()));
                        if asked {
                            return Some(News::CheckFailed(why));
                        }
                    }
                }
            }
        }
        let due = self.checked.is_none_or(|at| at.elapsed() > EVERY);
        if (self.enabled || self.asked) && due && self.check.is_none() {
            self.checked = Some(Instant::now());
            let (tx, rx) = mpsc::channel();
            let waker = self.waker;
            std::thread::spawn(move || {
                let _ = tx.send(latest());
                waker();
            });
            self.check = Some(rx);
        }
        None
    }

    pub fn install(&mut self) -> bool {
        let Some(release) = self.offer.clone().filter(|_| self.installing.is_none()) else { return false };
        let (tx, rx) = mpsc::channel();
        let waker = self.waker;
        std::thread::spawn(move || {
            let _ = tx.send(install(&release));
            waker();
        });
        self.installing = Some(rx);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn which_version_is_newer() {
        assert!(newer("0.1.0", "0.2.0"));
        assert!(newer("0.1.9", "0.1.10"));
        assert!(newer("0.9.9", "v1.0.0"));
        assert!(!newer("0.2.0", "0.2.0"));
        assert!(!newer("0.2.0", "0.1.9"));
        assert!(!newer("0.2.0", "0.3.0-rc.1"));
        assert!(!newer("0.2.0", "nightly"));
        assert!(!current().is_empty());
    }

    #[test]
    fn a_release_from_githubs_answer() {
        let answer = json!({
            "tag_name": "v0.2.0",
            "html_url": "https://github.com/sebastian642431/hawk-canvas/releases/tag/v0.2.0",
            "draft": false,
            "assets": [
                { "name": "code-canvas-windows-x64.zip", "browser_download_url": "https://github.com/sebastian642431/hawk-canvas/releases/download/v0.2.0/code-canvas-windows-x64.zip" },
                { "name": "hawk-canvas.exe", "browser_download_url": "https://github.com/sebastian642431/hawk-canvas/releases/download/v0.2.0/hawk-canvas.exe" }
            ]
        });
        let release = parse_release(&answer).unwrap();
        assert_eq!(release.version, "0.2.0");
        assert!(release.page.ends_with("/tag/v0.2.0"));
        if cfg!(windows) {
            assert!(release.download.unwrap().ends_with("/v0.2.0/hawk-canvas.exe"));
        }

        let elsewhere = json!({ "tag_name": "v9.0.0", "assets": [
            { "name": "hawk-canvas.exe", "browser_download_url": "https://example.com/hawk-canvas.exe" }
        ]});
        assert_eq!(parse_release(&elsewhere).unwrap().download, None);

        let former = json!({ "tag_name": "v0.1.0", "assets": [
            { "name": "hawk-canvas.exe", "browser_download_url": "https://github.com/sebastian642431/code-canvas/releases/download/v0.1.0/hawk-canvas.exe" }
        ]});
        assert_eq!(parse_release(&former).unwrap().download.is_some(), cfg!(windows));

        let both = |names: &[&str]| {
            let base = "https://github.com/sebastian642431/hawk-canvas/releases/download/v1.0.0/";
            let assets: Vec<Value> = names
                .iter()
                .map(|name| json!({ "name": name, "browser_download_url": format!("{base}{name}") }))
                .collect();
            parse_release(&json!({ "tag_name": "v1.0.0", "assets": assets })).unwrap().download
        };
        if cfg!(windows) {
            let new_first = both(&["code-canvas.exe", "hawk-canvas.exe"]).unwrap();
            assert!(new_first.ends_with("/hawk-canvas.exe"), "the program of its new name is taken first");
            let old_only = both(&["code-canvas.exe"]).unwrap();
            assert!(old_only.ends_with("/code-canvas.exe"), "a release from before the new name still updates");
        }
        assert_eq!(parse_release(&json!({ "tag_name": "v1.0.0", "draft": true })), None);
        assert_eq!(parse_release(&json!({ "message": "Not Found" })), None);
    }

    #[test]
    fn the_running_program_is_moved_aside() {
        let dir = std::env::temp_dir().join(format!("code-canvas-test-update-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (exe, fresh) = (dir.join("code-canvas.exe"), dir.join("code-canvas.new.exe"));
        std::fs::write(&exe, "old").unwrap();
        std::fs::write(&fresh, "new").unwrap();
        swap(&exe, &fresh).unwrap();
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(dir.join("code-canvas.old.exe")).unwrap(), "old");
        assert!(!fresh.exists());

        assert!(swap(&exe, &fresh).is_err());
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "new");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_older_copy_still_in_use_does_not_stop_the_next_update() {
        let dir = std::env::temp_dir().join(format!("code-canvas-test-update-busy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("code-canvas.old.exe")).unwrap();
        std::fs::write(dir.join("code-canvas.old.exe").join("held"), "a copy that cannot be removed").unwrap();
        let (exe, fresh) = (dir.join("code-canvas.exe"), dir.join("code-canvas.new.exe"));
        std::fs::write(&exe, "running").unwrap();
        std::fs::write(&fresh, "newer").unwrap();
        swap(&exe, &fresh).unwrap();
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "newer");
        assert_eq!(std::fs::read_to_string(dir.join("code-canvas.old-1.exe")).unwrap(), "running");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore]
    fn the_real_latest_release() {
        let release = latest().expect("a release");
        println!("latest: {} ({:?}); this build is {}", release.version, release.download, current());
        assert!(!numbers(&release.version).is_empty());
    }
}

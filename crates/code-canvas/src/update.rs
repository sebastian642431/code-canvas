//! Updates: is there a newer release than this build, and installing it.
//!
//! The latest release is read from GitHub when the app starts and a few
//! times a day (one small request, nothing about the user in it). A newer
//! one is offered once in a notice; taking it downloads the release's
//! `code-canvas.exe`, puts it in place of the running one and starts it.
//!
//! Replacing a running program: on Windows its file cannot be overwritten,
//! but it can be renamed. The running one becomes `code-canvas.old.exe`,
//! the new one takes its name, and the old file is removed the next time
//! the app starts.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use serde_json::Value;

/// Where releases are published: `owner/name` on GitHub.
pub const REPOSITORY: &str = "sebastian642431/code-canvas";
/// The file of a release that is this program, for this system.
const ASSET: &str = "code-canvas.exe";
/// How often the latest release is looked up while the app stays open.
const EVERY: Duration = Duration::from_secs(6 * 3600);

/// The version of this build. `CODE_CANVAS_PRETEND_VERSION` replaces it, to
/// see how an offered update looks without waiting for a release.
pub fn current() -> &'static str {
    static VERSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    VERSION.get_or_init(|| {
        std::env::var("CODE_CANVAS_PRETEND_VERSION").unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_string())
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    /// Without the `v` of its tag: `0.2.0`.
    pub version: String,
    /// Its page, for people.
    pub page: String,
    /// The program itself, when the release has one for this system.
    pub download: Option<String>,
}

fn numbers(version: &str) -> Vec<u64> {
    let core = version.trim().trim_start_matches('v').split(['-', '+']).next().unwrap_or("");
    core.split('.').map_while(|part| part.parse().ok()).collect()
}

/// Is `candidate` a later version than `current`? Pre-releases are not.
pub fn newer(current: &str, candidate: &str) -> bool {
    let pre_release = candidate.split('+').next().unwrap_or("").contains('-');
    let (now, then) = (numbers(current), numbers(candidate));
    !pre_release && !then.is_empty() && then > now
}

/// The release in GitHub's answer for `releases/latest`.
pub fn parse_release(json: &Value) -> Option<Release> {
    let text = |key: &str| json.get(key).and_then(Value::as_str);
    if json.get("draft").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let version = text("tag_name")?.trim_start_matches('v').to_string();
    let wanted = format!("https://github.com/{REPOSITORY}/releases/download/");
    let download = json.get("assets").and_then(Value::as_array).and_then(|assets| {
        assets
            .iter()
            .filter(|a| a.get("name").and_then(Value::as_str) == Some(ASSET))
            .filter_map(|a| a.get("browser_download_url").and_then(Value::as_str))
            // Only from the releases of this repository, whatever the answer says.
            .find(|url| url.starts_with(&wanted))
            .map(str::to_string)
    });
    let page =
        text("html_url").map(str::to_string).unwrap_or_else(|| format!("https://github.com/{REPOSITORY}/releases"));
    Some(Release { version, page, download: download.filter(|_| cfg!(windows)) })
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(600))).build().into()
}

const USER_AGENT: &str = concat!("code-canvas/", env!("CARGO_PKG_VERSION"));

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

/// Put the downloaded program `fresh` in place of the running one, `exe`.
/// The running one is kept as `<name>.old.exe` until the next start.
pub fn swap(exe: &Path, fresh: &Path) -> Result<(), String> {
    let old = exe.with_extension("old.exe");
    let _ = std::fs::remove_file(&old);
    std::fs::rename(exe, &old).map_err(|e| format!("the running program could not be moved aside: {e}"))?;
    if let Err(e) = std::fs::rename(fresh, exe) {
        // Back as it was: better the old version than none.
        let _ = std::fs::rename(&old, exe);
        return Err(format!("the new version could not be put in place: {e}"));
    }
    Ok(())
}

/// Remove what an earlier update left behind next to the program.
pub fn clean_up() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(exe.with_extension("old.exe"));
        let _ = std::fs::remove_file(exe.with_extension("new.exe"));
    }
}

/// Download `release` and put it in place of the running program. Returns
/// the program to start. Blocks: runs on a thread of its own.
fn install(release: &Release) -> Result<PathBuf, String> {
    use std::io::Read;
    let url = release.download.as_deref().ok_or("this release has no program for this system")?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let fresh = exe.with_extension("new.exe");
    let mut response = agent().get(url).header("User-Agent", USER_AGENT).call().map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    const LIMIT: u64 = 400 * 1024 * 1024;
    response.body_mut().as_reader().take(LIMIT).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    // A program, whole: not an error page, not half a download.
    if bytes.len() < 1024 * 1024 || !bytes.starts_with(b"MZ") {
        return Err("what was downloaded is not the program".to_string());
    }
    std::fs::write(&fresh, &bytes).map_err(|e| format!("the download could not be saved: {e}"))?;
    swap(&exe, &fresh)?;
    Ok(exe)
}

/// Open a page in the user's browser.
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
        command.creation_flags(0x0800_0000); // no console window
    }
    if let Err(e) = command.spawn() {
        log::warn!("could not open {url}: {e}");
    }
}

/// Where the updater stands: shown in Settings and in the status bar.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum State {
    /// Nothing has been looked up yet (or looking is switched off).
    #[default]
    Unknown,
    Checking,
    UpToDate,
    /// This version is on offer.
    Available(String),
    /// It is being downloaded and put in place.
    Installing(String),
    /// The last look failed (no network, most likely).
    Failed(String),
}

/// What the updater has to tell the app.
pub enum News {
    /// A release newer than this build exists (told once per version).
    Available(Release),
    /// The user asked, and this build is the latest.
    UpToDate,
    /// The user asked, and the answer could not be had.
    CheckFailed(String),
    /// The new version is in place: start this program and leave.
    Installed(PathBuf),
    Failed(String),
}

pub struct Updater {
    enabled: bool,
    checked: Option<Instant>,
    check: Option<Receiver<Result<Release, String>>>,
    installing: Option<Receiver<Result<PathBuf, String>>>,
    /// The latest release found, if it is newer than this build.
    pub offer: Option<Release>,
    /// The version the user was already told about.
    told: Option<String>,
    /// The user asked for this look: say how it went, whatever the answer.
    asked: bool,
    /// A look finished: `Ok` when the answer arrived.
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

    /// Look for a newer release now, because the user asked (it works with
    /// the automatic look switched off too).
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

    /// Start what is due and take in what finished. Call it often.
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

    /// Download and install the release on offer, in the background.
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
            "html_url": "https://github.com/sebastian642431/code-canvas/releases/tag/v0.2.0",
            "draft": false,
            "assets": [
                { "name": "code-canvas-windows-x64.zip", "browser_download_url": "https://github.com/sebastian642431/code-canvas/releases/download/v0.2.0/code-canvas-windows-x64.zip" },
                { "name": "code-canvas.exe", "browser_download_url": "https://github.com/sebastian642431/code-canvas/releases/download/v0.2.0/code-canvas.exe" }
            ]
        });
        let release = parse_release(&answer).unwrap();
        assert_eq!(release.version, "0.2.0");
        assert!(release.page.ends_with("/tag/v0.2.0"));
        if cfg!(windows) {
            assert!(release.download.unwrap().ends_with("/v0.2.0/code-canvas.exe"));
        }

        // A program from somewhere else is not taken, whatever it is called.
        let elsewhere = json!({ "tag_name": "v9.0.0", "assets": [
            { "name": "code-canvas.exe", "browser_download_url": "https://example.com/code-canvas.exe" }
        ]});
        assert_eq!(parse_release(&elsewhere).unwrap().download, None);
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

        // Nothing to put in place: the program stays as it was.
        assert!(swap(&exe, &fresh).is_err());
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "new");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Asks GitHub: `cargo test -p code-canvas real_latest -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn the_real_latest_release() {
        let release = latest().expect("a release");
        println!("latest: {} ({:?}); this build is {}", release.version, release.download, current());
        assert!(!numbers(&release.version).is_empty());
    }
}

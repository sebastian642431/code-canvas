use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::Receiver;

pub fn inside(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let rel_path = Path::new(rel);
    let stays = rel_path.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
    if !stays {
        return Err(format!("\"{rel}\" is not inside the folder"));
    }
    Ok(root.join(rel_path))
}

fn named(dir: &Path, name: &str) -> Result<PathBuf, String> {
    let name = name.trim().replace('\\', "/");
    let name = name.trim_matches('/');
    if name.is_empty() {
        return Err("a name is needed".to_string());
    }
    let mut out = dir.to_path_buf();
    for part in name.split('/') {
        let part = part.trim();
        let reserved = part.chars().any(|c| matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') || c.is_control());
        if part.is_empty() || part == "." || part == ".." || reserved || part.ends_with('.') {
            return Err(format!("\"{name}\" is not a name a file can have"));
        }
        out.push(part);
    }
    Ok(out)
}

fn fresh(target: PathBuf) -> Result<PathBuf, String> {
    if target.exists() {
        let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        return Err(format!("there is already something named \"{name}\" there"));
    }
    Ok(target)
}

pub fn create_file(dir: &Path, name: &str) -> Result<PathBuf, String> {
    let target = fresh(named(dir, name)?)?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::OpenOptions::new().write(true).create_new(true).open(&target).map_err(|e| e.to_string())?;
    Ok(target)
}

pub fn create_folder(dir: &Path, name: &str) -> Result<PathBuf, String> {
    let target = fresh(named(dir, name)?)?;
    std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
    Ok(target)
}

pub fn rename(path: &Path, name: &str) -> Result<PathBuf, String> {
    let name = name.trim();
    if name.contains('/') || name.contains('\\') {
        return Err("a new name, not a path: it stays in the same folder".to_string());
    }
    let dir = path.parent().ok_or("it has no folder")?;
    let target = named(dir, name)?;
    if target == path {
        return Ok(target);
    }
    let same_file = target.to_string_lossy().eq_ignore_ascii_case(&path.to_string_lossy());
    let target = if same_file { target } else { fresh(target)? };
    std::fs::rename(path, &target).map_err(|e| e.to_string())?;
    Ok(target)
}

pub fn recycle(path: &Path) -> Result<(), String> {
    trash::delete(path).map_err(|e| e.to_string())
}

pub struct Copying {
    pub target: PathBuf,
    pub finished: Receiver<Result<(), String>>,
}

pub fn copy_into(file: PathBuf, dir: &Path) -> Result<Copying, String> {
    let name = file.file_name().ok_or("it has no name")?.to_os_string();
    if file.parent().is_some_and(|p| p == dir) {
        return Err(String::new());
    }
    let target = fresh(dir.join(name))?;
    let to = target.clone();
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = std::fs::copy(&file, &to).map(|_| ()).map_err(|e| e.to_string());
        if let Err(e) = &result {
            log::error!("could not copy {} to {}: {e}", file.display(), to.display());
        }
        let _ = done.send(result);
    });
    Ok(Copying { target, finished })
}

fn is_web_link(url: &str) -> bool {
    let web = url.starts_with("https://") || url.starts_with("http://");
    web && url.chars().all(|c| c.is_ascii_graphic() && !matches!(c, '"' | '\'' | '`' | '<' | '>' | '^' | '|'))
}

pub fn open_web_link(url: &str) -> Result<(), String> {
    if !is_web_link(url) {
        return Err(format!("\"{url}\" is not a link the browser can be given"));
    }
    let mut command = if cfg!(windows) {
        let mut c = Command::new("rundll32.exe");
        c.arg("url.dll,FileProtocolHandler").arg(url);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(url);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    command.spawn().map(|_| ()).map_err(|e| e.to_string())
}

pub fn reveal(path: &Path) -> Result<(), String> {
    let mut command = if cfg!(windows) {
        let mut c = Command::new("explorer.exe");
        c.arg(format!("/select,{}", path.display().to_string().replace('/', "\\")));
        c
    } else if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg("-R").arg(path);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(path.parent().unwrap_or(path));
        c
    };
    command.spawn().map(|_| ()).map_err(|e| e.to_string())
}

pub fn terminal(dir: &Path) -> Result<(), String> {
    if cfg!(windows) {
        let modern = Command::new("wt.exe").arg("-d").arg(dir).spawn();
        if modern.is_ok() {
            return Ok(());
        }
        return Command::new("cmd.exe")
            .args(["/C", "start", "", "cmd.exe"])
            .current_dir(dir)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string());
    }
    let mut command = if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.args(["-a", "Terminal"]).arg(dir);
        c
    } else {
        let mut c = Command::new("x-terminal-emulator");
        c.current_dir(dir);
        c
    };
    command.spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("code-canvas-test-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn paths_stay_inside_the_folder() {
        let root = Path::new("/project");
        assert_eq!(inside(root, "src/main.rs").unwrap(), root.join("src").join("main.rs"));
        assert_eq!(inside(root, "").unwrap(), root.join(""));
        assert!(inside(root, "../other").is_err());
        assert!(inside(root, "src/../../other").is_err());
        assert!(inside(root, "/etc/passwd").is_err());
        assert!(inside(root, "C:\\Windows").is_err() || !cfg!(windows));
    }

    #[test]
    fn names_are_checked() {
        let dir = Path::new("/project/src");
        assert_eq!(named(dir, "a.txt").unwrap(), dir.join("a.txt"));
        assert_eq!(named(dir, "  deep/er\\file.rs ").unwrap(), dir.join("deep").join("er").join("file.rs"));
        for bad in ["", "   ", "..", "a/../b", "what?.txt", "a:b", "tab\there", "ends.", "/"] {
            assert!(named(dir, bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn create_and_rename_never_overwrite() {
        let dir = scratch("create");
        let file = create_file(&dir, "notes/today.md").unwrap();
        assert!(file.is_file() && file.ends_with("notes/today.md"));
        assert!(create_file(&dir, "notes/today.md").is_err());
        assert!(create_folder(&dir, "notes").is_err());
        let folder = create_folder(&dir, "assets/img").unwrap();
        assert!(folder.is_dir());

        std::fs::write(&file, "kept").unwrap();
        let other = create_file(&dir, "notes/other.md").unwrap();
        assert!(rename(&other, "today.md").is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "kept");
        assert!(rename(&other, "sub/other.md").is_err());
        let renamed = rename(&other, "renamed.md").unwrap();
        assert!(renamed.is_file() && !other.exists());
        assert_eq!(rename(&renamed, "renamed.md").unwrap(), renamed);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_plain_web_links_reach_the_browser() {
        assert!(is_web_link("https://example.com/a?b=1&c=2#top"));
        assert!(is_web_link("http://localhost:8080/"));
        assert!(!is_web_link("file:///C:/Windows/System32/calc.exe"));
        assert!(!is_web_link("https://example.com/\" & calc"));
        assert!(!is_web_link("https://example.com/a b"));
        assert!(!is_web_link("javascript:alert(1)"));
    }

    #[test]
    fn a_dropped_file_is_copied_once() {
        let (from, to) = (scratch("drop-from"), scratch("drop-to"));
        let file = from.join("clip.mp4");
        std::fs::write(&file, b"video").unwrap();
        let copying = copy_into(file.clone(), &to).unwrap();
        let finished = copying.finished.recv_timeout(std::time::Duration::from_secs(5));
        assert_eq!(finished, Ok(Ok(())));
        assert_eq!(std::fs::read(&copying.target).unwrap(), b"video");
        assert!(file.exists(), "the original stays where it was");
        assert!(copy_into(file.clone(), &to).is_err_and(|e| !e.is_empty()));
        assert!(copy_into(file, &from).is_err_and(|e| e.is_empty()));
        let _ = std::fs::remove_dir_all(&from);
        let _ = std::fs::remove_dir_all(&to);
    }
}

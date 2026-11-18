use std::path::{Path, PathBuf};

pub fn parent(rel: &str) -> &str {
    rel.rsplit_once('/').map_or("", |(dir, _)| dir)
}

pub fn base(rel: &str) -> &str {
    rel.rsplit_once('/').map_or(rel, |(_, name)| name)
}

pub fn remap(path: &str, from: &str, to: &str) -> Option<String> {
    if path == from {
        return Some(to.to_string());
    }
    path.strip_prefix(from).and_then(|rest| rest.strip_prefix('/')).map(|rest| format!("{to}/{rest}"))
}

#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    Itself,
    Inside,
    SameFolder,
    Root,
}

impl Refusal {
    pub fn why(&self, name: &str) -> String {
        match self {
            Refusal::Itself => format!("\"{name}\" cannot go into itself."),
            Refusal::Inside => format!("\"{name}\" cannot go into a folder inside itself."),
            Refusal::SameFolder => format!("\"{name}\" is already in that folder."),
            Refusal::Root => "The open folder itself cannot be moved from here.".to_string(),
        }
    }
}

pub fn check_drop(from: &str, to_dir: &str, same_root: bool, copy: bool) -> Result<(), Refusal> {
    if from.is_empty() {
        return Err(Refusal::Root);
    }
    if !same_root {
        return Ok(());
    }
    if to_dir == from {
        return Err(Refusal::Itself);
    }
    if to_dir.strip_prefix(from).is_some_and(|rest| rest.starts_with('/')) {
        return Err(Refusal::Inside);
    }
    if !copy && parent(from) == to_dir {
        return Err(Refusal::SameFolder);
    }
    Ok(())
}

pub fn top_most(paths: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for path in paths {
        let covered = paths
            .iter()
            .any(|other| other != path && path.strip_prefix(other.as_str()).is_some_and(|r| r.starts_with('/')));
        if !covered && !out.contains(path) {
            out.push(path.clone());
        }
    }
    out
}

pub fn select_range(rows: &[&str], anchor: &str, to: &str) -> Vec<String> {
    let a = rows.iter().position(|row| *row == anchor);
    let b = rows.iter().position(|row| *row == to);
    match (a, b) {
        (Some(a), Some(b)) => rows[a.min(b)..=a.max(b)].iter().map(|row| row.to_string()).collect(),
        _ => vec![to.to_string()],
    }
}

pub fn paste_dir(path: &str, is_dir: bool) -> String {
    if is_dir {
        path.to_string()
    } else {
        parent(path).to_string()
    }
}

pub fn unique_name(name: &str, is_dir: bool, taken: impl Fn(&str) -> bool) -> String {
    let (stem, ext) = match name.rfind('.').filter(|at| *at > 0 && !is_dir) {
        Some(at) => (&name[..at], &name[at..]),
        None => (name, ""),
    };
    let stem = match stem.rsplit_once(" (") {
        Some((head, tail))
            if tail.strip_suffix(')').is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())) =>
        {
            head
        }
        _ => stem,
    };
    (2..).map(|n| format!("{stem} ({n}){ext}")).find(|candidate| !taken(candidate)).unwrap_or_else(|| name.to_string())
}

#[derive(Clone, Debug)]
pub struct Job {
    pub from_root: PathBuf,
    pub to_root: PathBuf,
    pub items: Vec<String>,
    pub to_dir: String,
    pub copy: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clash {
    Replace,
    KeepBoth,
}

impl Job {
    fn target_dir(&self) -> Result<PathBuf, String> {
        crate::files::inside(&self.to_root, &self.to_dir)
    }

    fn same_folder_copy(&self, item: &str) -> bool {
        self.copy && self.from_root == self.to_root && parent(item) == self.to_dir
    }

    pub fn refusals(&self) -> Vec<String> {
        let same_root = self.from_root == self.to_root;
        self.items
            .iter()
            .filter_map(|item| {
                check_drop(item, &self.to_dir, same_root, self.copy).err().map(|why| why.why(base(item)))
            })
            .collect()
    }

    pub fn clashes(&self) -> Vec<String> {
        let Ok(dir) = self.target_dir() else { return Vec::new() };
        self.items
            .iter()
            .filter(|item| !self.same_folder_copy(item) && dir.join(base(item)).exists())
            .map(|item| base(item).to_string())
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Moved {
    pub from: PathBuf,
    pub to: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub copy: bool,
    pub moved: Vec<Moved>,
}

pub fn run(job: &Job, clash: Clash) -> Result<Outcome, String> {
    let dir = job.target_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut moved: Vec<Moved> = Vec::new();
    for item in &job.items {
        let from = crate::files::inside(&job.from_root, item)?;
        let name = base(item);
        let mut to = dir.join(name);
        let kept_alongside = job.same_folder_copy(item);
        if to.exists() && (kept_alongside || clash == Clash::KeepBoth) {
            let fresh = unique_name(name, from.is_dir(), |candidate| dir.join(candidate).exists());
            to = dir.join(fresh);
        } else if to.exists() {
            crate::files::recycle(&to)?;
        }
        let done = if job.copy { crate::files::copy_tree(&from, &to) } else { crate::files::relocate(&from, &to) };
        if let Err(why) = done {
            let undone = Outcome { copy: job.copy, moved: moved.clone() };
            let _ = undo(&undone);
            return Err(format!("\"{name}\" could not be {}: {why}", if job.copy { "copied" } else { "moved" }));
        }
        moved.push(Moved { from, to });
    }
    Ok(Outcome { copy: job.copy, moved })
}

pub fn undo(done: &Outcome) -> Result<(), String> {
    let mut failed = Vec::new();
    for step in done.moved.iter().rev() {
        let result = if done.copy {
            crate::files::recycle(&step.to)
        } else if step.from.exists() {
            Err("something is there again".to_string())
        } else {
            crate::files::relocate(&step.to, &step.from)
        };
        if let Err(why) = result {
            let name = step.to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            failed.push(format!("\"{name}\": {why}"));
        }
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(failed.join("; "))
    }
}

pub fn describe(done: &Outcome, root: &Path) -> String {
    let rel = |p: &Path| p.strip_prefix(root).unwrap_or(p).to_string_lossy().replace('\\', "/");
    let verb = if done.copy { "Copied" } else { "Moved" };
    match done.moved.as_slice() {
        [one] => format!(
            "{verb} \"{}\" to {}.",
            one.to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            match one.to.parent() {
                Some(dir) if rel(dir).is_empty() || dir == root => "the folder's root".to_string(),
                Some(dir) => rel(dir),
                None => String::new(),
            }
        ),
        many => format!("{verb} {} items.", many.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("code-canvas-test-tree-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn drops_are_checked() {
        assert_eq!(check_drop("src/a.rs", "lib", true, false), Ok(()));
        assert_eq!(check_drop("src/a.rs", "", true, false), Ok(()));
        assert_eq!(check_drop("src/a.rs", "src", true, false), Err(Refusal::SameFolder));
        assert_eq!(check_drop("src/a.rs", "src", true, true), Ok(()));
        assert_eq!(check_drop("src", "src", true, false), Err(Refusal::Itself));
        assert_eq!(check_drop("src", "src/deep/er", true, false), Err(Refusal::Inside));
        assert_eq!(check_drop("src", "src/deep", true, true), Err(Refusal::Inside));
        assert_eq!(check_drop("src", "srcs", true, false), Ok(()));
        assert_eq!(check_drop("a.rs", "", true, false), Err(Refusal::SameFolder));
        assert_eq!(check_drop("src", "src", false, false), Ok(()));
        assert_eq!(check_drop("", "src", true, false), Err(Refusal::Root));
    }

    #[test]
    fn clashing_names_get_a_number() {
        let taken = |names: &'static [&'static str]| move |n: &str| names.contains(&n);
        assert_eq!(unique_name("x.txt", false, taken(&["x.txt"])), "x (2).txt");
        assert_eq!(unique_name("x.txt", false, taken(&["x.txt", "x (2).txt"])), "x (3).txt");
        assert_eq!(unique_name("x (2).txt", false, taken(&["x (2).txt"])), "x (3).txt");
        assert_eq!(unique_name("dir", true, taken(&["dir"])), "dir (2)");
        assert_eq!(unique_name("v1.2", true, taken(&["v1.2"])), "v1.2 (2)");
        assert_eq!(unique_name(".env", false, taken(&[".env"])), ".env (2)");
        assert_eq!(unique_name("a.tar.gz", false, taken(&["a.tar.gz"])), "a.tar (2).gz");
    }

    #[test]
    fn path_math() {
        assert_eq!(parent("a/b/c.rs"), "a/b");
        assert_eq!(parent("c.rs"), "");
        assert_eq!(remap("a/b/c.rs", "a/b", "x/b").as_deref(), Some("x/b/c.rs"));
        assert_eq!(remap("a/bc", "a/b", "x"), None);
        assert_eq!(remap("a/b", "a/b", "x/b").as_deref(), Some("x/b"));
        assert_eq!(paste_dir("src/a.rs", false), "src");
        assert_eq!(paste_dir("src", true), "src");
        assert_eq!(paste_dir("a.rs", false), "");
        let all = vec!["a".to_string(), "a/b.rs".to_string(), "ab".to_string(), "a".to_string()];
        assert_eq!(top_most(&all), vec!["a".to_string(), "ab".to_string()]);
    }

    #[test]
    fn ranges_of_rows() {
        let rows = ["a", "a/b", "a/c", "d", "e"];
        assert_eq!(select_range(&rows, "a/b", "d"), vec!["a/b", "a/c", "d"]);
        assert_eq!(select_range(&rows, "e", "a/c"), vec!["a/c", "d", "e"]);
        assert_eq!(select_range(&rows, "zzz", "d"), vec!["d"]);
        assert_eq!(select_range(&rows, "d", "d"), vec!["d"]);
    }

    #[test]
    fn a_file_moves_in_and_out_and_back() {
        let root = scratch("move");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("a.txt"), "a").unwrap();
        let job = Job {
            from_root: root.clone(),
            to_root: root.clone(),
            items: vec!["a.txt".into()],
            to_dir: "src".into(),
            copy: false,
        };
        assert!(job.clashes().is_empty());
        let done = run(&job, Clash::Replace).unwrap();
        assert!(root.join("src/a.txt").is_file() && !root.join("a.txt").exists());
        undo(&done).unwrap();
        assert!(root.join("a.txt").is_file() && !root.join("src/a.txt").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_clash_is_found_and_kept_both() {
        let root = scratch("clash");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("a.txt"), "new").unwrap();
        std::fs::write(root.join("src/a.txt"), "old").unwrap();
        let job = Job {
            from_root: root.clone(),
            to_root: root.clone(),
            items: vec!["a.txt".into()],
            to_dir: "src".into(),
            copy: false,
        };
        assert_eq!(job.clashes(), vec!["a.txt".to_string()]);
        let done = run(&job, Clash::KeepBoth).unwrap();
        assert_eq!(std::fs::read_to_string(root.join("src/a.txt")).unwrap(), "old");
        assert_eq!(std::fs::read_to_string(root.join("src/a (2).txt")).unwrap(), "new");
        assert_eq!(done.moved.len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn copying_beside_itself_numbers_the_copy_and_undo_removes_it() {
        let root = scratch("copy");
        std::fs::create_dir_all(root.join("d/inner")).unwrap();
        std::fs::write(root.join("d/inner/f.txt"), "f").unwrap();
        let job = Job {
            from_root: root.clone(),
            to_root: root.clone(),
            items: vec!["d".into()],
            to_dir: String::new(),
            copy: true,
        };
        assert!(job.clashes().is_empty());
        let done = run(&job, Clash::Replace).unwrap();
        assert_eq!(std::fs::read_to_string(root.join("d (2)/inner/f.txt")).unwrap(), "f");
        assert!(root.join("d/inner/f.txt").is_file());
        assert_eq!(describe(&done, &root), "Copied \"d (2)\" to the folder's root.");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn nothing_into_itself_runs() {
        let root = scratch("self");
        std::fs::create_dir_all(root.join("d/e")).unwrap();
        let job = Job {
            from_root: root.clone(),
            to_root: root.clone(),
            items: vec!["d".into()],
            to_dir: "d/e".into(),
            copy: false,
        };
        assert_eq!(job.refusals().len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }
}

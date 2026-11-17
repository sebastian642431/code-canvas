use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Partial {
    pub dir: String,
    pub prefix: String,
    pub start: usize,
}

pub fn partial(line: &str, col: usize) -> Option<Partial> {
    let before: String = line.chars().take(col).collect();
    let (at, quote) = before.char_indices().rev().find(|(_, c)| matches!(c, '"' | '\'' | '`'))?;
    if before.matches(quote).count() % 2 == 0 {
        return None;
    }
    let typed = &before[at + quote.len_utf8()..];
    if typed.chars().any(char::is_whitespace) || typed.contains("://") {
        return None;
    }
    let slash = typed.rfind('/')?;
    let (dir, prefix) = typed.split_at(slash + 1);
    Some(Partial { dir: dir.to_string(), prefix: prefix.to_string(), start: col - prefix.chars().count() })
}

#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub name: String,
    pub is_dir: bool,
}

fn tidy(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    Some(out)
}

pub fn folder(root: &Path, file: &str, dir: &str) -> Option<PathBuf> {
    let typed = dir.trim_end_matches('/');
    let base = match dir.strip_prefix('/') {
        Some(_) => root.to_path_buf(),
        None => root.join(file).parent()?.to_path_buf(),
    };
    tidy(&base.join(typed.trim_start_matches('/')))
}

const MAX: usize = 60;

pub fn candidates(folder: &Path, prefix: &str) -> Vec<Candidate> {
    let Ok(entries) = std::fs::read_dir(folder) else { return Vec::new() };
    let wanted = prefix.to_lowercase();
    let mut out: Vec<Candidate> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let hidden = name.starts_with('.') && !prefix.starts_with('.');
            let matches = name.to_lowercase().starts_with(&wanted) && name != prefix;
            (matches && !hidden).then(|| Candidate { is_dir: e.file_type().is_ok_and(|t| t.is_dir()), name })
        })
        .collect();
    out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    out.truncate(MAX);
    out
}

pub fn completion(candidate: &Candidate, editing: &str, line: &str) -> String {
    if candidate.is_dir {
        return format!("{}/", candidate.name);
    }
    let script = |name: &str| {
        let ext = name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
        matches!(ext.as_str(), "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs")
    };
    let importing = ["import", "require(", "from ", "export "].iter().any(|word| line.contains(word));
    let declares_types = candidate.name.to_lowercase().ends_with(".d.ts");
    if script(editing) && script(&candidate.name) && importing && !declares_types {
        return candidate.name.rsplit_once('.').map_or(candidate.name.clone(), |(stem, _)| stem.to_string());
    }
    candidate.name.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_being_typed() {
        let at_end = |line: &str| partial(line, line.chars().count());
        let p = at_end(r#"import x from "./comp"#).unwrap();
        assert_eq!((p.dir.as_str(), p.prefix.as_str(), p.start), ("./", "comp", 17));
        let p = at_end("mod x; // see '../docs/").unwrap();
        assert_eq!((p.dir.as_str(), p.prefix.as_str()), ("../docs/", ""));
        assert_eq!(at_end(r#"let a = "src/ui/git"#).unwrap().dir, "src/ui/");
        assert_eq!(at_end(r#"url("/assets/"#).unwrap().dir, "/assets/");
        assert_eq!(at_end("x = \"./añ").unwrap().start, 7);

        assert_eq!(at_end(r#"let a = "hello"#), None);
        assert_eq!(at_end(r#"let a = "done"; b / c"#), None);
        assert_eq!(at_end("a / b"), None);
        assert_eq!(at_end(r#"fetch("https://example.com/"#), None);
        assert_eq!(at_end(r#"say("one two/"#), None);
        assert_eq!(partial(r#"open("./a/b")"#, 10).unwrap().dir, "./a/");
    }

    #[test]
    fn where_it_points() {
        let root = Path::new("/project");
        let at = |file: &str, dir: &str| folder(root, file, dir);
        assert_eq!(at("src/ui/panel.rs", "./"), Some(root.join("src").join("ui")));
        assert_eq!(at("src/ui/panel.rs", "../"), Some(root.join("src")));
        assert_eq!(at("src/ui/panel.rs", "../../docs/"), Some(root.join("docs")));
        assert_eq!(at("src/ui/panel.rs", "icons/"), Some(root.join("src").join("ui").join("icons")));
        assert_eq!(at("src/ui/panel.rs", "/assets/"), Some(root.join("assets")));
        assert_eq!(at("main.rs", "./"), Some(root.to_path_buf()));
    }

    #[test]
    fn what_is_offered() {
        let dir = std::env::temp_dir().join(format!("code-canvas-test-paths-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("components")).unwrap();
        std::fs::create_dir_all(dir.join(".hidden")).unwrap();
        for name in ["App.tsx", "api.ts", "config.json", ".env"] {
            std::fs::write(dir.join(name), "").unwrap();
        }
        let names = |prefix: &str| candidates(&dir, prefix).into_iter().map(|c| c.name).collect::<Vec<_>>();
        assert_eq!(names(""), vec!["components", "api.ts", "App.tsx", "config.json"]);
        assert_eq!(names("a"), vec!["api.ts", "App.tsx"]);
        assert_eq!(names("CO"), vec!["components", "config.json"]);
        assert_eq!(names("."), vec![".hidden", ".env"]);
        assert_eq!(names("api.ts"), Vec::<String>::new());
        assert!(candidates(&dir.join("missing"), "").is_empty());
        assert!(candidates(&dir, "")[0].is_dir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_is_written() {
        let file = |name: &str| Candidate { name: name.to_string(), is_dir: false };
        let import = r#"import App from "./A"#;
        assert_eq!(completion(&file("App.tsx"), "main.ts", import), "App");
        assert_eq!(completion(&file("types.d.ts"), "main.ts", import), "types.d.ts");
        assert_eq!(completion(&file("style.css"), "main.ts", import), "style.css");
        assert_eq!(completion(&file("App.tsx"), "main.ts", r#"const p = "./A"#), "App.tsx");
        assert_eq!(completion(&file("App.tsx"), "notes.md", import), "App.tsx");
        let folder = Candidate { name: "components".into(), is_dir: true };
        assert_eq!(completion(&folder, "main.ts", import), "components/");
    }
}

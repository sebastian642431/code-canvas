use super::*;
use std::path::Component;

#[derive(Debug, PartialEq)]
pub(super) enum Matched {
    One(usize, String),
    Several(Vec<usize>),
    Nothing,
}

pub(super) struct Folder<'a> {
    pub page: usize,
    pub name: &'a str,
    pub paths: Vec<&'a str>,
}

pub(super) fn rel_under(root: &Path, path: &Path) -> Option<String> {
    let whole = path.to_string_lossy().replace('\\', "/");
    let whole = whole.trim_end_matches('/');
    let key = ext::path_key(whole);
    let root = ext::path_key(&root.to_string_lossy());
    let rest = key.strip_prefix(root.trim_end_matches('/'))?;
    if !(rest.is_empty() || rest.starts_with('/')) || key.len() != whole.len() {
        return None;
    }
    Some(whole[whole.len() - rest.len()..].trim_start_matches('/').to_string())
}

pub(super) fn folders_above(rel: &str) -> Vec<&str> {
    rel.match_indices('/').map(|(at, _)| &rel[..at]).collect()
}

fn same(a: &str, b: &str) -> bool {
    a == b || a.to_lowercase() == b.to_lowercase()
}

fn parts_in_common(wanted: &[&str], name: &str, rel: &str) -> usize {
    let theirs = rel.rsplit('/').filter(|part| !part.is_empty()).chain([name]);
    wanted.iter().rev().zip(theirs).take_while(|(a, b)| same(a, b)).count()
}

/// A link Claude writes relative to the folder it works in reaches here joined to the wrong place, so
/// the one meant is the path in an open folder (its name included) that ends with the most of the link.
pub(super) fn match_tail(wanted: &[&str], folders: &[Folder], active: usize) -> Matched {
    let mut best = 1;
    let mut hits: Vec<(usize, &str)> = Vec::new();
    for folder in folders {
        for &rel in &folder.paths {
            let n = parts_in_common(wanted, folder.name, rel);
            if n < best {
                continue;
            }
            if n > best {
                best = n;
                hits.clear();
            }
            hits.push((folder.page, rel));
        }
    }
    let mut pages: Vec<usize> = hits.iter().map(|&(page, _)| page).collect();
    pages.sort_unstable();
    pages.dedup();
    let page = match pages.as_slice() {
        [] => return Matched::Nothing,
        _ if pages.contains(&active) => active,
        [only] => *only,
        _ => return Matched::Several(pages),
    };
    let rel = hits
        .iter()
        .filter(|&&(p, _)| p == page)
        .map(|&(_, rel)| rel)
        .min_by_key(|rel| (rel.matches('/').count(), rel.len()));
    rel.map_or(Matched::Nothing, |rel| Matched::One(page, rel.to_string()))
}

fn dirs_of<'a>(dir: &'a canvas_core::Dir, out: &mut Vec<&'a str>) {
    for d in &dir.dirs {
        out.push(&d.rel_path);
        dirs_of(d, out);
    }
}

impl App {
    fn folder_holding(&self, path: &Path) -> Option<(usize, String)> {
        self.pages
            .iter()
            .enumerate()
            .filter(|(_, page)| !page.only)
            .filter_map(|(i, page)| Some((i, rel_under(&page.root, path)?)))
            .min_by_key(|(_, rel)| rel.len())
    }

    fn link_target(&self, path: &Path) -> Matched {
        let wanted: Vec<String> = path
            .components()
            .filter_map(|part| match part {
                Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect();
        let wanted: Vec<&str> = wanted.iter().map(String::as_str).collect();
        let folders: Vec<Folder> = self
            .pages
            .iter()
            .enumerate()
            .filter(|(_, page)| !page.only)
            .filter_map(|(i, page)| {
                let r = page.ready()?;
                let mut paths = vec![""];
                dirs_of(&r.repo.tree, &mut paths);
                paths.extend(r.repo.files.iter().map(|f| f.rel_path.as_str()));
                Some(Folder { page: i, name: page.root.file_name()?.to_str()?, paths })
            })
            .collect();
        match_tail(&wanted, &folders, self.active)
    }

    /// `None` when the link could mean things in several open folders (a notice says so).
    pub(super) fn resolve_link(&mut self, path: &Path) -> Option<PathBuf> {
        let path: PathBuf = canvas_core::scan::clean_path(path.to_path_buf()).components().collect();
        let on_canvas = self.locate_file(&path).is_some_and(|(i, rel)| {
            self.pages[i].ready().is_some_and(|r| r.layout.node_for_path(&r.repo, &rel).is_some())
        });
        if path.exists() || on_canvas {
            return Some(path);
        }
        match self.link_target(&path) {
            Matched::One(i, rel) => {
                let root = &self.pages[i].root;
                let found = if rel.is_empty() { root.clone() } else { root.join(&rel) };
                log::info!("the link to {} is taken to be {}", path.display(), found.display());
                Some(found)
            }
            Matched::Several(pages) => {
                let names: Vec<String> = pages
                    .iter()
                    .map(|&i| self.pages[i].root.file_name().unwrap_or_default().to_string_lossy().into_owned())
                    .collect();
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                let text = format!(
                    "{name} is in more than one open folder ({}): open it from the one you mean.",
                    names.join(", ")
                );
                self.ext.notice(ext::Level::Info, "Files", text);
                None
            }
            Matched::Nothing => Some(path),
        }
    }

    pub(super) fn show_dir(&mut self, path: PathBuf) {
        let Some((i, rel)) = self.folder_holding(&path) else { return self.open_folder(path, None) };
        self.switch_to(i);
        self.show_when_read = None;
        if rel.is_empty() {
            return self.request_redraw();
        }
        let page = &self.pages[i];
        let Some(r) = page.ready() else {
            self.show_when_read = Some((path, None));
            return self.request_redraw();
        };
        let above = folders_above(&rel);
        if let Some(hidden) = above.iter().copied().chain([rel.as_str()]).find(|d| page.filter.hidden.contains(*d)) {
            let text = format!("{hidden} is hidden on the canvas: tick it in the explorer to see it.");
            return self.ext.notice(ext::Level::Info, "Files", text);
        }
        let shown = r.layout.node_for_path(&r.repo, &rel).is_some();
        let narrowed = !shown && (page.only_changed || page.only_connected || page.showing.is_some());
        let folded: Vec<String> =
            above.iter().filter(|d| page.filter.collapsed.contains(**d)).map(|d| d.to_string()).collect();
        let near = std::iter::once(rel.as_str())
            .chain(above.iter().rev().copied())
            .find(|d| r.layout.node_for_path(&r.repo, d).is_some())
            .map(str::to_string);
        if narrowed || !folded.is_empty() {
            let page = &mut self.pages[i];
            if narrowed {
                page.only_changed = false;
                page.only_connected = false;
                if page.showing.is_some() {
                    page.show_commit(None);
                }
            }
            for d in &folded {
                page.filter.collapsed.remove(d);
            }
            self.relayout();
            self.show_when_read = Some((path, None));
            return self.request_redraw();
        }
        match near {
            Some(d) => self.apply(Action::FlyToPath(d)),
            None => self.request_redraw(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder<'a>(page: usize, name: &'a str, paths: &[&'a str]) -> Folder<'a> {
        Folder { page, name, paths: paths.to_vec() }
    }

    fn parts(path: &str) -> Vec<&str> {
        path.split('/').filter(|part| !part.is_empty()).collect()
    }

    #[test]
    fn a_folder_link_with_a_wrong_start_finds_the_folder_already_open() {
        let open = [folder(0, "senebi", &["", "apps", "apps/senebiClient", "apps/senebiClient/oldClient", "apps/web"])];
        let wanted = parts("C:/Users/me/work/senebiClient/oldClient/");
        assert_eq!(match_tail(&wanted, &open, 0), Matched::One(0, "apps/senebiClient/oldClient".into()));
    }

    #[test]
    fn the_name_of_the_open_folder_counts_as_part_of_the_path() {
        let open = [folder(0, "senebiClient", &["", "oldClient", "src/oldClient"])];
        let wanted = parts("D:/elsewhere/senebiClient/oldClient");
        assert_eq!(match_tail(&wanted, &open, 0), Matched::One(0, "oldClient".into()));
        assert_eq!(match_tail(&parts("D:/elsewhere/senebiClient"), &open, 0), Matched::One(0, String::new()));
    }

    #[test]
    fn the_longest_ending_wins_and_case_does_not_matter() {
        let open = [folder(0, "repo", &["a/client", "b/old/client", "b/old/client/main.rs"])];
        assert_eq!(match_tail(&parts("x/Old/Client"), &open, 0), Matched::One(0, "b/old/client".into()));
        assert_eq!(
            match_tail(&parts("x/old/client/main.rs"), &open, 0),
            Matched::One(0, "b/old/client/main.rs".into())
        );
    }

    #[test]
    fn within_one_folder_the_shallowest_match_is_taken() {
        let open = [folder(0, "repo", &["deep/down/utils", "utils", "other/utils"])];
        assert_eq!(match_tail(&parts("/nowhere/utils"), &open, 0), Matched::One(0, "utils".into()));
    }

    #[test]
    fn matches_in_several_folders_prefer_the_active_one_or_say_so() {
        let open = [folder(0, "one", &["src/old"]), folder(1, "two", &["lib/old"]), folder(2, "three", &["x"])];
        assert_eq!(match_tail(&parts("/q/old"), &open, 1), Matched::One(1, "lib/old".into()));
        assert_eq!(match_tail(&parts("/q/old"), &open, 2), Matched::Several(vec![0, 1]));
        assert_eq!(match_tail(&parts("/q/src/old"), &open, 2), Matched::One(0, "src/old".into()));
    }

    #[test]
    fn nothing_matches_when_the_last_part_is_not_there() {
        let open = [folder(0, "repo", &["", "src", "src/main.rs"])];
        assert_eq!(match_tail(&parts("/repo/src/gone.rs"), &open, 0), Matched::Nothing);
        assert_eq!(match_tail(&[], &open, 0), Matched::Nothing);
    }

    #[test]
    fn a_path_inside_an_open_folder_gives_its_relative_path() {
        let root = Path::new("C:/code/repo");
        assert_eq!(rel_under(root, Path::new("C:/code/repo/src/old/")), Some("src/old".into()));
        assert_eq!(rel_under(root, Path::new("C:\\code\\repo\\src")), Some("src".into()));
        assert_eq!(rel_under(root, Path::new("C:/code/repo/")), Some(String::new()));
        assert_eq!(rel_under(root, Path::new("C:/code/repository/src")), None);
        assert_eq!(rel_under(root, Path::new("C:/code")), None);
    }

    #[test]
    fn the_folders_above_a_path_go_from_the_top_down() {
        assert_eq!(folders_above("a/b/c"), vec!["a", "a/b"]);
        assert!(folders_above("a").is_empty());
    }
}

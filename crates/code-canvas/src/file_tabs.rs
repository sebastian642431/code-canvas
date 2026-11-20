//! The tabs above the canvas: "Canvas" first, then the files opened to be read like in an editor.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub enum TabOp {
    /// Opens the file in a tab, or brings its tab to the front. A relative path is inside the folder in view.
    Open(PathBuf),
    Canvas,
    Front(usize),
    Close(usize),
    CloseOthers(usize),
    CloseAll,
    Cycle(isize),
    /// Moves a tab to a slot of the row (0 is before the first tab).
    Move(usize, usize),
    /// Back to the canvas, flying to the card of the file in front.
    Reveal,
    Press {
        row: usize,
        col: usize,
        shift: bool,
        alt: bool,
        double: bool,
    },
    DragTo {
        row: usize,
        col: usize,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Reveal {
    Caret,
    Row(usize, usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tab {
    pub path: PathBuf,
    pub scroll: [f32; 2],
}

/// Rows and columns of the file in front, for a selection where the file cannot be edited.
pub type Cell = (usize, usize);

#[derive(Default)]
pub struct Tabs {
    pub list: Vec<Tab>,
    front: Option<usize>,
    pub pick: Option<(Cell, Cell)>,
    /// A place shown in the file in front (row, column, length), from a link or a search.
    pub mark: Option<(usize, usize, usize)>,
    pub reveal: Option<Reveal>,
    /// Rows to scroll the file in front by, asked by the keyboard.
    pub nudge: f32,
    /// How many rows the file in front shows at once.
    pub page_rows: f32,
    pub bar_offset: f32,
    pub bar_seen: Option<usize>,
    /// The button over the name of the card under the pointer that opens it in a tab, in points.
    pub card_button: Option<(egui::Rect, PathBuf)>,
}

pub fn same_file(a: &Path, b: &Path) -> bool {
    crate::ext::path_key(&a.to_string_lossy()) == crate::ext::path_key(&b.to_string_lossy())
}

impl Tabs {
    pub fn restore(paths: Vec<PathBuf>, front: Option<usize>, keep: impl Fn(&Path) -> bool) -> Tabs {
        let wanted = front.and_then(|i| paths.get(i).cloned());
        let mut tabs = Tabs::default();
        for path in paths.into_iter().filter(|p| keep(p)) {
            if tabs.position(&path).is_none() {
                tabs.list.push(Tab { path, scroll: [0.0; 2] });
            }
        }
        tabs.front = wanted.and_then(|path| tabs.position(&path));
        tabs
    }

    pub fn saved(&self) -> (Vec<PathBuf>, Option<usize>) {
        (self.list.iter().map(|tab| tab.path.clone()).collect(), self.front)
    }

    pub fn front(&self) -> Option<usize> {
        self.front
    }

    pub fn front_path(&self) -> Option<&Path> {
        self.front.and_then(|i| self.list.get(i)).map(|tab| tab.path.as_path())
    }

    pub fn front_tab_mut(&mut self) -> Option<&mut Tab> {
        self.front.and_then(|i| self.list.get_mut(i))
    }

    pub fn position(&self, path: &Path) -> Option<usize> {
        self.list.iter().position(|tab| same_file(&tab.path, path))
    }

    /// Returns whether the tab in front changed.
    pub fn set_front(&mut self, front: Option<usize>) -> bool {
        let front = front.filter(|&i| i < self.list.len());
        if front == self.front {
            return false;
        }
        self.front = front;
        self.forget_view();
        true
    }

    fn forget_view(&mut self) {
        self.pick = None;
        self.mark = None;
        self.reveal = None;
    }

    /// A new tab goes right after the one in front, as in an editor.
    pub fn open(&mut self, path: PathBuf) -> usize {
        let at = match self.position(&path) {
            Some(at) => at,
            None => {
                let at = self.front.map_or(self.list.len(), |front| front + 1);
                self.list.insert(at, Tab { path, scroll: [0.0; 2] });
                if let Some(front) = self.front.filter(|&front| front >= at) {
                    self.front = Some(front + 1);
                }
                at
            }
        };
        self.set_front(Some(at));
        at
    }

    /// Closing the tab in front brings the one now in its place, else the one before it, else the canvas.
    pub fn close(&mut self, i: usize) {
        if i >= self.list.len() {
            return;
        }
        self.list.remove(i);
        match self.front {
            Some(front) if front == i => {
                self.front = (!self.list.is_empty()).then(|| i.min(self.list.len() - 1));
                self.forget_view();
            }
            Some(front) if front > i => self.front = Some(front - 1),
            _ => {}
        }
    }

    pub fn retain(&mut self, keep: impl Fn(&Path) -> bool) {
        let front = self.front_path().map(Path::to_path_buf);
        self.list.retain(|tab| keep(&tab.path));
        let still = front.as_ref().and_then(|path| self.position(path));
        if front.is_some() && still.is_none() {
            self.forget_view();
        }
        self.front = still;
    }

    /// The canvas counts as the first place of the cycle.
    pub fn cycle(&mut self, delta: isize) {
        let places = self.list.len() as isize + 1;
        let at = self.front.map_or(0, |front| front as isize + 1);
        let next = (at + delta).rem_euclid(places) as usize;
        self.set_front(next.checked_sub(1));
    }

    pub fn move_tab(&mut self, from: usize, slot: usize) {
        if from >= self.list.len() {
            return;
        }
        let front = self.front_path().map(Path::to_path_buf);
        let tab = self.list.remove(from);
        let to = if slot > from { slot - 1 } else { slot };
        self.list.insert(to.min(self.list.len()), tab);
        self.front = front.and_then(|path| self.position(&path));
    }
}

/// The name of each file, and the end of its folder when another tab has the same name.
pub fn labels(paths: &[&Path]) -> Vec<(String, String)> {
    let parts: Vec<Vec<String>> = paths
        .iter()
        .map(|path| path.components().map(|part| part.as_os_str().to_string_lossy().into_owned()).collect())
        .collect();
    let name = |i: usize| parts[i].last().cloned().unwrap_or_default();
    let tail = |i: usize, n: usize| {
        let all = &parts[i];
        let end = all.len().saturating_sub(1);
        all[end.saturating_sub(n)..end].join("/")
    };
    (0..paths.len())
        .map(|i| {
            let alike: Vec<usize> = (0..paths.len()).filter(|&j| j != i && name(j) == name(i)).collect();
            if alike.is_empty() {
                return (name(i), String::new());
            }
            let most = parts[i].len().saturating_sub(1);
            let n = (1..=most).find(|&n| alike.iter().all(|&j| tail(j, n) != tail(i, n))).unwrap_or(most);
            (name(i), tail(i, n))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tabs(names: &[&str]) -> Tabs {
        let mut tabs = Tabs::default();
        for name in names {
            tabs.open(PathBuf::from(format!("/r/{name}")));
        }
        tabs
    }

    fn names(tabs: &Tabs) -> Vec<String> {
        tabs.list.iter().map(|tab| tab.path.file_name().unwrap().to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn opening_puts_the_tab_after_the_one_in_front_and_brings_it_forward() {
        let mut t = tabs(&["a", "b", "c"]);
        assert_eq!(names(&t), ["a", "b", "c"]);
        t.set_front(Some(0));
        t.open(PathBuf::from("/r/d"));
        assert_eq!(names(&t), ["a", "d", "b", "c"]);
        assert_eq!(t.front(), Some(1));
        assert_eq!(t.open(PathBuf::from("/r/c")), 3);
        assert_eq!(t.list.len(), 4, "an open file is not opened twice");
        t.set_front(None);
        t.open(PathBuf::from("/r/e"));
        assert_eq!(names(&t), ["a", "d", "b", "c", "e"]);
    }

    #[test]
    fn closing_brings_the_neighbour_then_the_canvas() {
        let mut t = tabs(&["a", "b", "c"]);
        t.set_front(Some(1));
        t.close(1);
        assert_eq!((names(&t), t.front()), (vec!["a".into(), "c".into()], Some(1)));
        t.close(1);
        assert_eq!(t.front(), Some(0));
        t.close(0);
        assert_eq!(t.front(), None);
        assert!(t.list.is_empty());
    }

    #[test]
    fn closing_another_tab_keeps_the_one_in_front() {
        let mut t = tabs(&["a", "b", "c"]);
        t.set_front(Some(2));
        t.pick = Some(((1, 0), (1, 4)));
        t.close(0);
        assert_eq!(t.front_path(), Some(Path::new("/r/c")));
        assert!(t.pick.is_some(), "what was selected in the file in front stays");
        t.set_front(None);
        t.close(0);
        assert_eq!(t.front(), None);
    }

    #[test]
    fn cycling_goes_through_the_canvas() {
        let mut t = tabs(&["a", "b"]);
        t.set_front(None);
        t.cycle(1);
        assert_eq!(t.front(), Some(0));
        t.cycle(1);
        t.cycle(1);
        assert_eq!(t.front(), None);
        t.cycle(-1);
        assert_eq!(t.front(), Some(1));
    }

    #[test]
    fn moving_keeps_the_tab_in_front() {
        let mut t = tabs(&["a", "b", "c"]);
        t.set_front(Some(0));
        t.move_tab(0, 3);
        assert_eq!(names(&t), ["b", "c", "a"]);
        assert_eq!(t.front(), Some(2));
        t.move_tab(2, 0);
        assert_eq!(names(&t), ["a", "b", "c"]);
        t.move_tab(1, 2);
        assert_eq!(names(&t), ["a", "b", "c"], "a slot next to the tab leaves it where it is");
    }

    #[test]
    fn restoring_skips_what_is_gone() {
        let paths = vec![PathBuf::from("/r/a"), PathBuf::from("/r/gone"), PathBuf::from("/r/c")];
        let t = Tabs::restore(paths.clone(), Some(2), |p| !p.ends_with("gone"));
        assert_eq!(names(&t), ["a", "c"]);
        assert_eq!(t.front_path(), Some(Path::new("/r/c")));
        let t = Tabs::restore(paths, Some(1), |p| !p.ends_with("gone"));
        assert_eq!(t.front(), None, "the canvas is in front when the file in front is gone");
        assert_eq!(t.saved(), (vec![PathBuf::from("/r/a"), PathBuf::from("/r/c")], None));
    }

    #[test]
    fn retain_drops_the_tabs_of_a_closed_folder() {
        let mut t = tabs(&["a", "b"]);
        t.open(PathBuf::from("/other/x"));
        t.retain(|p| p.starts_with("/r"));
        assert_eq!(names(&t), ["a", "b"]);
        assert_eq!(t.front(), None);
    }

    #[test]
    fn same_names_show_enough_of_their_folders() {
        let paths = [Path::new("/r/src/ui/mod.rs"), Path::new("/r/src/app/mod.rs"), Path::new("/r/main.rs")];
        let got = labels(&paths);
        assert_eq!(got[0], ("mod.rs".into(), "ui".into()));
        assert_eq!(got[1], ("mod.rs".into(), "app".into()));
        assert_eq!(got[2], ("main.rs".into(), String::new()));
        let deep = [Path::new("/r/a/x/lib.rs"), Path::new("/r/b/x/lib.rs")];
        let got = labels(&deep);
        assert_eq!((got[0].1.as_str(), got[1].1.as_str()), ("a/x", "b/x"));
    }
}

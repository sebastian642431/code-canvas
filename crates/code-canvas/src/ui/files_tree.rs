use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use egui::{Align2, FontId, Pos2, Rect, Sense, Stroke, Vec2};

use super::{c, menu_row, pal, Action, FileOp, Naming, NamingKind};
use crate::icons;
use crate::tree_ops;
use canvas_render::theme;

const EXPAND_AFTER: Duration = Duration::from_millis(600);
const EDGE: f32 = 30.0;

#[derive(Clone, Debug, PartialEq)]
pub enum DropAt {
    Dir(String),
    Folder(PathBuf),
}

pub enum TreeOp {
    Move { from_root: PathBuf, to_root: PathBuf, items: Vec<String>, to_dir: String, copy: bool },
    Cut(Vec<String>),
    CopyFiles(Vec<String>),
    Paste(String),
    Duplicate(Vec<String>),
    Delete(Vec<String>),
    Undo,
    Refused(String),
}

pub struct TreeDrag {
    pub root: PathBuf,
    pub paths: Vec<String>,
    pub is_dir: bool,
    over_row: Option<String>,
    since: Instant,
}

pub struct Clip {
    pub root: PathBuf,
    pub paths: Vec<String>,
    pub cut: bool,
}

pub struct Tree {
    pub sel: BTreeSet<String>,
    pub anchor: Option<String>,
    pub drag: Option<TreeDrag>,
    pub focus: bool,
    pub rows: Vec<(Rect, String, bool, bool)>,
    pub list: Rect,
    pub folders: Vec<(Rect, PathBuf)>,
    pub root_row: Rect,
    pub clip: Option<Clip>,
    pub can_undo: Option<String>,
    pub scroll: f32,
    pub os_over: Option<Pos2>,
    pub page: u32,
}

impl Default for Tree {
    fn default() -> Tree {
        Tree {
            sel: BTreeSet::new(),
            anchor: None,
            drag: None,
            focus: false,
            rows: Vec::new(),
            list: Rect::NOTHING,
            folders: Vec::new(),
            root_row: Rect::NOTHING,
            clip: None,
            can_undo: None,
            scroll: 0.0,
            os_over: None,
            page: u32::MAX,
        }
    }
}

pub struct Target {
    pub at: DropAt,
    pub light: Rect,
}

impl Tree {
    pub fn begin_frame(&mut self) {
        self.rows.clear();
        self.folders.clear();
        self.list = Rect::NOTHING;
        self.root_row = Rect::NOTHING;
    }

    pub fn acted_on(&self, path: &str) -> Vec<String> {
        if self.sel.contains(path) && self.sel.len() > 1 {
            tree_ops::top_most(&self.sel.iter().cloned().collect::<Vec<_>>())
        } else {
            vec![path.to_string()]
        }
    }

    fn row_at(&self, p: Pos2) -> Option<&(Rect, String, bool, bool)> {
        self.rows.iter().find(|row| row.0.contains(p))
    }

    pub fn target_at(&self, p: Pos2, active: &Path) -> Option<Target> {
        if let Some((rect, folder)) = self.folders.iter().find(|(rect, _)| rect.contains(p)) {
            let at = if folder == active { DropAt::Dir(String::new()) } else { DropAt::Folder(folder.clone()) };
            return Some(Target { at, light: *rect });
        }
        if self.root_row.contains(p) {
            return Some(Target { at: DropAt::Dir(String::new()), light: self.root_row });
        }
        if !self.list.contains(p) {
            return None;
        }
        match self.row_at(p) {
            Some((rect, path, true, _)) => Some(Target { at: DropAt::Dir(path.clone()), light: *rect }),
            Some((_, path, false, _)) => {
                let dir = tree_ops::parent(path).to_string();
                let light = self.rows.iter().find(|row| row.1 == dir).map_or(self.list, |row| row.0);
                Some(Target { at: DropAt::Dir(dir), light })
            }
            None => Some(Target { at: DropAt::Dir(String::new()), light: self.list }),
        }
    }

    pub fn start_drag(&mut self, root: &Path, path: &str, is_dir: bool) {
        if !self.sel.contains(path) {
            self.sel.clear();
            self.sel.insert(path.to_string());
            self.anchor = Some(path.to_string());
        }
        let paths = self.acted_on(path);
        let is_dir = is_dir || paths.len() > 1;
        self.drag = Some(TreeDrag { root: root.to_path_buf(), paths, is_dir, over_row: None, since: Instant::now() });
    }
}

fn verdict(drag: &TreeDrag, at: &DropAt, active: &Path, copy: bool) -> Result<(), String> {
    let (dir, same_root) = match at {
        DropAt::Dir(dir) => (dir.as_str(), drag.root == active),
        DropAt::Folder(folder) => ("", drag.root == *folder),
    };
    for path in &drag.paths {
        tree_ops::check_drop(path, dir, same_root, copy).map_err(|why| why.why(tree_ops::base(path)))?;
    }
    Ok(())
}

fn label_of(drag: &TreeDrag) -> String {
    match drag.paths.as_slice() {
        [one] => tree_ops::base(one).to_string(),
        many => format!("{} items", many.len()),
    }
}

fn paint_target(painter: &egui::Painter, light: Rect, ok: bool) {
    let tint = if ok { pal().accent } else { c(theme::get().git_removed) };
    painter.rect_filled(light.shrink2(Vec2::new(2.0, 0.0)), 6.0, tint.gamma_multiply(0.16));
    painter.rect_stroke(
        light.shrink2(Vec2::new(2.0, 0.0)),
        6.0,
        Stroke::new(1.5, tint.gamma_multiply(0.9)),
        egui::StrokeKind::Inside,
    );
}

pub(super) fn drag_step(
    ctx: &egui::Context,
    st: &mut super::UiState,
    active: Option<&Path>,
    actions: &mut Vec<Action>,
) {
    let Some(active) = active else {
        st.tree.drag = None;
        st.tree.scroll = 0.0;
        return;
    };
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("tree drag")));
    if let Some(p) = st.tree.os_over {
        if let Some(target) = st.tree.target_at(p, active) {
            paint_target(&painter, target.light, true);
        }
        ctx.request_repaint();
    }
    st.tree.scroll = 0.0;
    if st.tree.drag.is_none() {
        return;
    }
    let (pos, down, released, ctrl, escaped) = ctx.input(|i| {
        (
            i.pointer.latest_pos(),
            i.pointer.any_down(),
            i.pointer.any_released(),
            i.modifiers.ctrl || i.modifiers.command,
            i.key_pressed(egui::Key::Escape),
        )
    });
    let target = pos.and_then(|p| st.tree.target_at(p, active));
    let over_row = pos.and_then(|p| st.tree.row_at(p)).filter(|row| row.2 && !row.3).map(|row| row.1.clone());
    let list = st.tree.list;
    let tree = &mut st.tree;
    let Some(drag) = tree.drag.as_mut() else { return };
    if escaped {
        tree.drag = None;
        return;
    }
    let state = target.as_ref().map(|t| verdict(drag, &t.at, active, ctrl));
    if released || !down {
        let drag = tree.drag.take().unwrap();
        match (target, state) {
            (Some(t), Some(Ok(()))) => {
                let (to_root, to_dir) = match t.at {
                    DropAt::Dir(dir) => (active.to_path_buf(), dir),
                    DropAt::Folder(folder) => (folder, String::new()),
                };
                actions.push(Action::Tree(TreeOp::Move {
                    from_root: drag.root,
                    to_root,
                    items: drag.paths,
                    to_dir,
                    copy: ctrl,
                }));
            }
            (Some(_), Some(Err(why))) => actions.push(Action::Tree(TreeOp::Refused(why))),
            _ => {}
        }
        return;
    }
    if drag.over_row != over_row {
        drag.over_row = over_row.clone();
        drag.since = Instant::now();
    } else if let Some(path) = over_row {
        if drag.since.elapsed() >= EXPAND_AFTER {
            drag.since = Instant::now();
            actions.push(Action::ToggleTreeOpen(path));
        }
    }
    if let Some(p) = pos.filter(|p| list.x_range().contains(p.x)) {
        let (above, below) = (p.y - list.top(), list.bottom() - p.y);
        if (-60.0..EDGE).contains(&above) {
            tree.scroll = -((EDGE - above) * 0.45).clamp(1.0, 18.0);
        } else if (-60.0..EDGE).contains(&below) {
            tree.scroll = ((EDGE - below) * 0.45).clamp(1.0, 18.0);
        }
    }
    ctx.request_repaint();
    let Some(p) = pos else { return };
    let ok = !matches!(state, Some(Err(_)));
    if let Some(t) = &target {
        paint_target(&painter, t.light, ok);
    }
    let name = label_of(drag);
    let glyph = if drag.is_dir { icons::FOLDER } else { icons::FILE };
    let plus = if ctrl && ok { "+ " } else { "" };
    let blocked = if ok { "" } else { "  not here" };
    let text = format!("{plus}{glyph}  {name}{blocked}");
    let galley = painter.layout_no_wrap(text, FontId::proportional(13.0), pal().text);
    let tag = Rect::from_min_size(p + Vec2::new(14.0, 10.0), galley.size() + Vec2::new(20.0, 12.0));
    painter.rect_filled(tag, 8.0, pal().card);
    let edge = if ok { pal().accent } else { c(theme::get().git_removed) };
    painter.rect_stroke(tag, 8.0, Stroke::new(1.0, edge), egui::StrokeKind::Inside);
    painter.galley(tag.min + Vec2::new(10.0, 6.0), galley, pal().text);
}

pub(super) struct Quick<'a> {
    pub path: &'a str,
    pub is_dir: bool,
    pub open: bool,
    pub naming: &'a mut Option<Naming>,
}

fn quick_button(ui: &mut egui::Ui, at: Rect, id: egui::Id, glyph: &str, tip: &str) -> egui::Response {
    let resp = ui.interact(at, id, Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(at, 5.0, pal().border_strong);
    }
    let color = if resp.hovered() { pal().text } else { pal().text_dim };
    ui.painter().text(at.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(13.0), color);
    resp.on_hover_text(tip)
}

pub(super) fn quick_width() -> f32 {
    2.0 * 22.0 + 6.0
}

pub(super) fn quick_buttons(ui: &mut egui::Ui, row: Rect, quick: Quick, actions: &mut Vec<Action>) {
    let Quick { path, is_dir, open, naming } = quick;
    let mid = row.center().y;
    let mut slot = {
        let mut x = row.right() - 10.0;
        move || {
            let at = Rect::from_center_size(Pos2::new(x - 10.0, mid), Vec2::splat(20.0));
            x -= 22.0;
            at
        }
    };
    let ids: Vec<egui::Id> = (0..2).map(|k| ui.make_persistent_id(("tree quick", path, k))).collect();
    if is_dir {
        let plus = quick_button(ui, slot(), ids[0], icons::PLUS, "New file or folder in it");
        egui::Popup::menu(&plus).show(|ui| {
            ui.set_width(170.0);
            ui.spacing_mut().item_spacing.y = 1.0;
            let mut kind = None;
            if menu_row(ui, icons::FILE, "New file") {
                kind = Some(NamingKind::NewFile);
            }
            if menu_row(ui, icons::FOLDER, "New folder") {
                kind = Some(NamingKind::NewFolder);
            }
            if let Some(kind) = kind {
                *naming = Some(Naming { kind, path: path.to_string(), text: String::new(), focus: true });
                if !open {
                    actions.push(Action::ToggleTreeOpen(path.to_string()));
                }
                ui.close();
            }
        });
        if quick_button(ui, slot(), ids[1], icons::CODE, "Open a terminal in it").clicked() {
            actions.push(Action::File(FileOp::Terminal(path.to_string())));
        }
    } else {
        if quick_button(ui, slot(), ids[0], icons::LOCATE_FIXED, "Show its card on the canvas").clicked() {
            actions.push(Action::Tabs(crate::file_tabs::TabOp::Canvas));
            actions.push(Action::FlyToPath(path.to_string()));
        }
        if quick_button(ui, slot(), ids[1], icons::PENCIL, "Rename").clicked() {
            let name = tree_ops::base(path).to_string();
            *naming = Some(Naming { kind: NamingKind::Rename, path: path.to_string(), text: name, focus: true });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> Tree {
        let mut tree = Tree::default();
        let row = |y: f32| Rect::from_min_size(Pos2::new(0.0, y), Vec2::new(200.0, 24.0));
        tree.rows = vec![
            (row(100.0), "docs".into(), true, true),
            (row(124.0), "docs/a.txt".into(), false, false),
            (row(148.0), "src".into(), true, false),
        ];
        tree.list = Rect::from_min_size(Pos2::new(0.0, 90.0), Vec2::new(200.0, 300.0));
        tree.root_row = Rect::from_min_size(Pos2::new(0.0, 10.0), Vec2::new(200.0, 30.0));
        tree.folders =
            vec![(Rect::from_min_size(Pos2::new(0.0, 50.0), Vec2::new(200.0, 26.0)), PathBuf::from("/other"))];
        tree
    }

    #[test]
    fn the_target_is_the_folder_under_the_pointer_or_the_folder_of_a_file() {
        let tree = tree();
        let active = Path::new("/main");
        let at = |x: f32, y: f32| tree.target_at(Pos2::new(x, y), active).map(|t| t.at);
        assert_eq!(at(50.0, 105.0), Some(DropAt::Dir("docs".into())));
        assert_eq!(at(50.0, 130.0), Some(DropAt::Dir("docs".into())));
        assert_eq!(at(50.0, 155.0), Some(DropAt::Dir("src".into())));
        assert_eq!(at(50.0, 300.0), Some(DropAt::Dir(String::new())));
        assert_eq!(at(50.0, 20.0), Some(DropAt::Dir(String::new())));
        assert_eq!(at(50.0, 60.0), Some(DropAt::Folder(PathBuf::from("/other"))));
        assert_eq!(at(50.0, 80.0), None);
        assert_eq!(at(500.0, 300.0), None);
    }

    #[test]
    fn what_a_drag_carries_follows_the_selection() {
        let mut tree = tree();
        tree.start_drag(Path::new("/main"), "docs/a.txt", false);
        assert_eq!(tree.drag.as_ref().map(|d| d.paths.clone()), Some(vec!["docs/a.txt".to_string()]));
        tree.sel = ["docs", "docs/a.txt", "src"].iter().map(|p| p.to_string()).collect();
        tree.start_drag(Path::new("/main"), "src", true);
        assert_eq!(tree.drag.as_ref().map(|d| d.paths.clone()), Some(vec!["docs".to_string(), "src".to_string()]));
    }

    #[test]
    fn a_drag_is_checked_against_where_it_lands() {
        let drag = TreeDrag {
            root: PathBuf::from("/main"),
            paths: vec!["docs".into()],
            is_dir: true,
            over_row: None,
            since: Instant::now(),
        };
        let main = Path::new("/main");
        assert!(verdict(&drag, &DropAt::Dir("src".into()), main, false).is_ok());
        assert!(verdict(&drag, &DropAt::Dir("docs/deep".into()), main, false).is_err());
        assert!(verdict(&drag, &DropAt::Dir(String::new()), main, false).is_err());
        assert!(verdict(&drag, &DropAt::Dir(String::new()), main, true).is_ok());
        assert!(verdict(&drag, &DropAt::Folder(PathBuf::from("/other")), main, false).is_ok());
    }
}

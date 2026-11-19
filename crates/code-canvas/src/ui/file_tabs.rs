//! The row of tabs above the canvas: "Canvas", then the files open to be read like in an editor.

use std::path::{Path, PathBuf};

use egui::{Align2, Color32, FontId, Frame, Pos2, Rect, Sense, Stroke, Vec2};

use super::{file_glyph, menu_gap, menu_row, pal, text_fit, Action, UiState, EDGE};
use crate::file_tabs::{labels, TabOp};
use crate::icons;
use crate::keys::Command;
use crate::page::Page;

pub(super) const BAR_H: f32 = 34.0;
const HIGH: f32 = 26.0;
const GAP: f32 = 3.0;
const NARROWEST: f32 = 84.0;
const WIDEST: f32 = 230.0;

pub(super) fn codicon(name: &str, or: &'static str) -> &'static str {
    crate::codicons::glyph(name).unwrap_or(or)
}

/// The folder that holds a file, and the file's path inside it; the deepest folder when they nest.
pub(super) fn locate(pages: &[Page], path: &Path) -> Option<(usize, String)> {
    let whole = path.to_string_lossy().replace('\\', "/");
    let key = crate::ext::path_key(&whole);
    let inside = pages.iter().enumerate().filter_map(|(i, page)| {
        let mut root = crate::ext::path_key(&page.root.to_string_lossy());
        if !root.ends_with('/') {
            root.push('/');
        }
        let rel = whole.get(root.len()..).filter(|_| key.starts_with(&root) && key.len() == whole.len())?;
        Some((root.len(), i, rel.to_string()))
    });
    inside.max_by_key(|(len, ..)| *len).map(|(_, i, rel)| (i, rel))
}

struct Shown {
    path: PathBuf,
    name: String,
    dir: String,
    dirty: bool,
    want: f32,
}

pub(super) fn bar(ui: &mut egui::Ui, st: &mut UiState, pages: &[Page], actions: &mut Vec<Action>) {
    let top = egui::Panel::top("file tabs").frame(Frame::NONE.fill(pal().chrome)).exact_size(BAR_H);
    top.show_separator_line(false).show(ui, |ui| row(ui, st, pages, actions));
}

fn row(ui: &mut egui::Ui, st: &mut UiState, pages: &[Page], actions: &mut Vec<Action>) {
    let all = ui.max_rect();
    let strip = Rect::from_min_max(
        Pos2::new(all.left() + EDGE + 2.0, all.bottom() - HIGH - 4.0),
        Pos2::new(all.right() - EDGE - 2.0, all.bottom() - 4.0),
    );
    let font = FontId::proportional(12.5);
    let small = FontId::proportional(11.0);
    let front = st.tabs.front();

    let canvas_w = 30.0 + ui.painter().layout_no_wrap("Canvas".into(), font.clone(), pal().text).size().x + 12.0;
    let canvas_at = Rect::from_min_size(strip.min, Vec2::new(canvas_w, HIGH));
    let resp = ui.interact(canvas_at, egui::Id::unique("file tab canvas"), Sense::click());
    chip(ui, canvas_at, front.is_none(), resp.hovered(), false);
    let color = if front.is_none() || resp.hovered() { pal().text } else { pal().text_dim };
    let mid = canvas_at.center().y;
    let glyph = codicon("layout", icons::LAYOUT_GRID);
    ui.painter().text(Pos2::new(canvas_at.left() + 15.0, mid), Align2::CENTER_CENTER, glyph, font.clone(), color);
    ui.painter().text(Pos2::new(canvas_at.left() + 28.0, mid), Align2::LEFT_CENTER, "Canvas", font.clone(), color);
    let tip = format!("The canvas  ·  {}", st.keys.hint("next tab", Command::NextFolder));
    if resp.on_hover_text(tip).clicked() {
        actions.push(Action::Tabs(TabOp::Canvas));
    }

    let room = Rect::from_min_max(Pos2::new(canvas_at.right() + 8.0, strip.top()), strip.max);
    let paths: Vec<&Path> = st.tabs.list.iter().map(|tab| tab.path.as_path()).collect();
    let shown: Vec<Shown> = labels(&paths)
        .into_iter()
        .zip(&paths)
        .map(|((name, dir), path)| {
            let dirty = locate(pages, path).is_some_and(|(i, rel)| pages[i].editors.get(&rel).is_some_and(|e| e.dirty));
            let text = ui.painter().layout_no_wrap(name.clone(), font.clone(), pal().text).size().x;
            let after = match dir.is_empty() {
                true => 0.0,
                false => ui.painter().layout_no_wrap(dir.clone(), small.clone(), pal().text).size().x + 6.0,
            };
            Shown { path: path.to_path_buf(), name, dir, dirty, want: (text + after + 56.0).clamp(NARROWEST, WIDEST) }
        })
        .collect();
    if shown.is_empty() {
        if room.width() > 260.0 {
            let hint = "Files open here from the button by a card's name, the files list or Ctrl+P (Ctrl+Enter)";
            let at = Pos2::new(room.left() + 6.0, room.center().y);
            text_fit(ui.painter(), at, hint, small, pal().text_faint, room.width() - 12.0);
        }
        return;
    }

    let total: f32 = shown.iter().map(|tab| tab.want + GAP).sum();
    let spare = (total - room.width()).max(0.0);
    let hovered = ui.rect_contains_pointer(strip);
    if hovered {
        let wheel = ui.input(|input| input.smooth_scroll_delta);
        st.tabs.bar_offset -= wheel.x + wheel.y;
    }
    if st.tabs.bar_seen != front {
        st.tabs.bar_seen = front;
        if let Some(front) = front {
            let start: f32 = shown[..front].iter().map(|tab| tab.want + GAP).sum();
            let end = start + shown[front].want;
            if start < st.tabs.bar_offset {
                st.tabs.bar_offset = start;
            } else if end > st.tabs.bar_offset + room.width() {
                st.tabs.bar_offset = end - room.width();
            }
        }
    }
    st.tabs.bar_offset = st.tabs.bar_offset.clamp(0.0, spare);
    let painter = ui.painter().with_clip_rect(room);

    let mut x = room.left() - st.tabs.bar_offset;
    let mut rects = Vec::with_capacity(shown.len());
    let (mut dragging, mut dropped) = (None, None);
    for (i, tab) in shown.iter().enumerate() {
        let rect = Rect::from_min_size(Pos2::new(x, strip.top()), Vec2::new(tab.want, HIGH));
        x += tab.want + GAP;
        rects.push(rect);
        let seen = rect.intersect(room);
        if !seen.is_positive() {
            continue;
        }
        let resp = ui.interact(seen, egui::Id::unique(("file tab", &tab.path)), Sense::click_and_drag());
        if resp.dragged() {
            dragging = Some(i);
        }
        if resp.drag_stopped() {
            dropped = Some(i);
        }
        let on = front == Some(i);
        let lit = resp.hovered() && dragging != Some(i);
        chip_on(&painter, rect, on, lit, dragging == Some(i));
        let mid = rect.center().y;
        let icon_at = Rect::from_center_size(Pos2::new(rect.left() + 15.0, mid), Vec2::splat(15.0));
        let whole = room.contains_rect(icon_at);
        if !(whole && st.file_icons.paint(ui, &tab.name, false, false, icon_at, false)) {
            painter.text(icon_at.center(), Align2::CENTER_CENTER, file_glyph(&tab.name), font.clone(), pal().text_dim);
        }
        let close = Rect::from_center_size(Pos2::new(rect.right() - 13.0, mid), Vec2::splat(18.0));
        let closing = lit || on;
        let left = rect.left() + 28.0;
        let right = close.left() - 2.0;
        let color = if on || lit { pal().text } else { pal().text_dim };
        let name_w = text_fit(&painter, Pos2::new(left, mid), &tab.name, font.clone(), color, right - left);
        if !tab.dir.is_empty() {
            let at = Pos2::new(left + name_w + 6.0, mid);
            text_fit(&painter, at, &tab.dir, small.clone(), pal().text_faint, right - at.x);
        }
        let mut closed = false;
        if closing && room.contains_rect(close) {
            let cross = ui.interact(close, egui::Id::unique(("file tab close", &tab.path)), Sense::click());
            if cross.hovered() {
                painter.rect_filled(close, 5.0, pal().border_strong);
            }
            if tab.dirty && !cross.hovered() {
                painter.circle_filled(close.center(), 3.5, pal().text);
            } else {
                let tint = if cross.hovered() { pal().text } else { pal().text_dim };
                painter.text(close.center(), Align2::CENTER_CENTER, icons::X, FontId::proportional(12.0), tint);
            }
            let tip = if tab.dirty { "Close (the changes stay, not saved, on the card)" } else { "Close" };
            closed = cross.on_hover_text(st.keys.hint(tip, Command::CloseFolder)).clicked();
        } else if tab.dirty {
            painter.circle_filled(close.center(), 3.5, pal().text_dim);
        }
        if closed || resp.middle_clicked() {
            actions.push(Action::Tabs(TabOp::Close(i)));
            continue;
        }
        let path = tab.path.to_string_lossy().into_owned();
        resp.context_menu(|ui| {
            ui.set_width(220.0);
            ui.spacing_mut().item_spacing.y = 1.0;
            let mut chosen = Vec::new();
            if menu_row(ui, icons::X, "Close") {
                chosen.push(Action::Tabs(TabOp::Close(i)));
            }
            if shown.len() > 1 && menu_row(ui, icons::X, "Close the others") {
                chosen.push(Action::Tabs(TabOp::CloseOthers(i)));
            }
            if menu_row(ui, icons::X, "Close every tab") {
                chosen.push(Action::Tabs(TabOp::CloseAll));
            }
            menu_gap(ui);
            if menu_row(ui, icons::LOCATE_FIXED, "Reveal on the canvas") {
                chosen.extend([Action::Tabs(TabOp::Front(i)), Action::Tabs(TabOp::Reveal)]);
            }
            if menu_row(ui, icons::COPY, "Copy path") {
                ui.ctx().copy_text(path.clone());
                ui.close();
            }
            if !chosen.is_empty() {
                actions.extend(chosen);
                ui.close();
            }
        });
        let resp = resp.on_hover_text(path.as_str());
        if resp.clicked() || resp.drag_started() {
            actions.push(Action::Tabs(TabOp::Front(i)));
        }
    }
    if let (Some(i), Some(at)) = (dragging.or(dropped), ui.input(|input| input.pointer.latest_pos())) {
        let slot = rects.iter().filter(|rect| rect.center().x < at.x).count();
        if dragging.is_some() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            let line = match slot.checked_sub(1).and_then(|before| rects.get(before)) {
                Some(before) => before.right() + GAP * 0.5,
                None => room.left() + 1.0,
            };
            let span = egui::Rangef::new(strip.top() + 3.0, strip.bottom() - 3.0);
            painter.vline(line, span, Stroke::new(2.0, pal().accent));
        } else if slot != i && slot != i + 1 {
            actions.push(Action::Tabs(TabOp::Move(i, slot)));
        }
    }
    if spare > 0.0 {
        let shade = |at: f32, toward: f32| {
            let fade =
                Rect::from_min_max(Pos2::new(at.min(toward), strip.top()), Pos2::new(at.max(toward), strip.bottom()));
            ui.painter().rect_filled(fade, 0.0, pal().chrome.gamma_multiply(0.7));
        };
        if st.tabs.bar_offset > 0.5 {
            shade(room.left(), room.left() + 6.0);
        }
        if st.tabs.bar_offset < spare - 0.5 {
            shade(room.right() - 6.0, room.right());
        }
    }
}

fn chip(ui: &egui::Ui, rect: Rect, on: bool, hovered: bool, held: bool) {
    chip_on(ui.painter(), rect, on, hovered, held);
}

fn chip_on(painter: &egui::Painter, rect: Rect, on: bool, hovered: bool, held: bool) {
    let fill = if held || (hovered && !on) {
        pal().hover
    } else if on {
        pal().raised
    } else {
        Color32::TRANSPARENT
    };
    let edge = if on { pal().border_strong } else { Color32::TRANSPARENT };
    painter.rect(rect, 7.0, fill, Stroke::new(1.0, edge), egui::StrokeKind::Inside);
}

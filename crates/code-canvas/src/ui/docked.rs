use egui::{Align2, CornerRadius, FontId, Pos2, Rect, RichText, Sense, Stroke, Vec2};

use super::{
    ext_panels, menu_row, paint_look, pal, text_fit, Action, DropAt, Panel, RailDrag, RailEntry, RailLook, UiState,
};
use crate::ext::Extensions;
use crate::icons;

const TAB_H: f32 = 30.0;
const GAP: f32 = 4.0;
const NARROW_TAB: f32 = 38.0;

/// How wide each tab is: with its title when all of them fit, else only icons.
pub(super) fn tab_widths(natural: &[f32], room: f32) -> Vec<f32> {
    let gaps = GAP * natural.len().saturating_sub(1) as f32;
    match natural.iter().sum::<f32>() + gaps <= room {
        true => natural.to_vec(),
        false => vec![NARROW_TAB; natural.len()],
    }
}

/// Where a card dropped at `at` goes, from the corner of the canvas: its title under the pointer.
pub(super) fn card_at(canvas: Rect, at: Pos2) -> [f32; 4] {
    let (width, height) = (420.0, (canvas.height() - 16.0).clamp(150.0, 640.0));
    let x = (at.x - canvas.left() - 60.0).clamp(8.0, (canvas.width() - width - 8.0).max(8.0));
    let y = (at.y - canvas.top() - 20.0).clamp(8.0, (canvas.height() - height - 8.0).max(8.0));
    [x, y, width, height]
}

pub(super) fn views_dock(
    ui: &mut egui::Ui,
    st: &mut UiState,
    ext: &mut Extensions,
    entries: &[RailEntry],
    actions: &mut Vec<Action>,
) {
    let all = ui.max_rect();
    let mut tabs = st.docked_here.clone();
    if st.docked(&st.dock) && !tabs.contains(&st.dock) {
        tabs.push(st.dock.clone());
    }
    let row = Rect::from_min_max(all.min + Vec2::new(6.0, 7.0), Pos2::new(all.right() - 6.0, all.top() + 7.0 + TAB_H));
    let content = Rect::from_min_max(Pos2::new(row.left(), row.bottom()), all.max - Vec2::new(6.0, 6.0));
    let font = FontId::proportional(12.5);
    let entry_of = |panel: &Panel| entries.iter().find(|entry| entry.panel == *panel);
    let titles: Vec<String> =
        tabs.iter().map(|tab| entry_of(tab).map_or_else(|| tab.key(), |e| e.tip.clone())).collect();
    let natural: Vec<f32> = titles
        .iter()
        .map(|title| ui.painter().layout_no_wrap(title.clone(), font.clone(), pal().text).size().x.min(150.0) + 46.0)
        .collect();
    let widths = tab_widths(&natural, row.width());
    let named = widths.first().is_some_and(|width| *width > NARROW_TAB);
    let first_selected = tabs.first() == Some(&st.dock);
    let round = CornerRadius { nw: if first_selected { 0 } else { 8 }, ne: 8, sw: 8, se: 8 };
    ui.painter().rect_filled(content, round, pal().card);
    st.dock_tabs.clear();
    let (mut pick, mut float) = (None, None);
    let mut x = row.left();
    let puzzle = RailLook::Glyph(icons::PUZZLE);
    for ((tab, title), width) in tabs.iter().zip(&titles).zip(&widths) {
        let rect = Rect::from_min_size(Pos2::new(x, row.top()), Vec2::new(*width, TAB_H));
        x += width + GAP;
        if rect.right() > row.right() + 1.0 {
            break;
        }
        let key = tab.key();
        st.dock_tabs.push((key.clone(), rect));
        let resp = ui.interact(rect, ui.scope_id().with(("docked tab", &key)), Sense::click_and_drag());
        let selected = st.dock == *tab;
        let lifted = st.rail_drag.as_ref().is_some_and(|drag| drag.key == key);
        if selected {
            ui.painter().rect_filled(rect, CornerRadius { nw: 8, ne: 8, sw: 0, se: 0 }, pal().card);
            let mark = Rect::from_min_max(
                rect.left_top() + Vec2::new(8.0, 0.0),
                Pos2::new(rect.right() - 8.0, rect.top() + 2.0),
            );
            ui.painter().rect_filled(mark, 1.0, pal().accent);
        } else if resp.hovered() {
            ui.painter().rect_filled(rect.shrink2(Vec2::new(0.0, 3.0)), 7.0, pal().hover);
        }
        let color = if selected || resp.hovered() { pal().text } else { pal().text_dim };
        if !lifted {
            let look = entry_of(tab).map_or(&puzzle, |entry| &entry.look);
            let mid = rect.center().y;
            let icon_at = if named { Pos2::new(rect.left() + 17.0, mid) } else { rect.center() };
            paint_look(ui, st, ext, look, icon_at, 15.0, color);
            if named {
                let at = Pos2::new(rect.left() + 32.0, mid);
                text_fit(ui.painter(), at, title, font.clone(), color, rect.right() - 10.0 - at.x);
            }
        }
        if resp.drag_started_by(egui::PointerButton::Primary) {
            st.rail_drag = Some(RailDrag { key: key.clone(), target: None });
        }
        resp.context_menu(|ui| {
            ui.set_width(200.0);
            if menu_row(ui, icons::APP_WINDOW, "Float on the canvas") {
                float = Some(tab.clone());
                ui.close();
            }
        });
        let tip = format!("{title}\nDrag it out of the panel, or right click it, to float it on the canvas");
        if resp.on_hover_text(tip).clicked() {
            pick = Some(tab.clone());
        }
    }
    let inner = content.shrink(1.0);
    ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
        ui.set_clip_rect(inner.intersect(ui.clip_rect()));
        match st.dock.clone() {
            Panel::Webview(view) => ext_panels::webview_panel(ui, st, ext, &view),
            Panel::Tree(id) => ext_panels::tree_panel(ui, st, ext, &id),
            _ => {
                ui.add_space(inner.height() * 0.3);
                ui.vertical_centered(|ui| {
                    ui.set_max_width(inner.width() - 40.0);
                    ui.label(RichText::new("Nothing is docked here yet").size(13.5).color(pal().text));
                    ui.add_space(6.0);
                    let how = "Drag the icon of an extension from a bar onto this panel, or right click it and choose \
                               Dock in the side panel.";
                    ui.label(RichText::new(how).size(12.0).color(pal().text_dim));
                });
            }
        }
    });
    if let Some(tab) = pick.filter(|tab| *tab != st.dock) {
        st.show(tab);
        actions.push(Action::RailsChanged);
    }
    if let Some(tab) = float {
        st.float_view(&tab, true);
        actions.push(Action::SetPrefs(st.prefs.clone()));
        actions.push(Action::RailsChanged);
    }
}

/// While the icon of an extension's view is dragged over the side panel: dropping it there docks
/// it, before the tab under the pointer when it is over the tabs.
pub(super) fn drop_on_dock(ctx: &egui::Context, st: &mut UiState, entries: &[RailEntry], dock: Rect) {
    let Some(key) = st.rail_drag.as_ref().map(|drag| drag.key.clone()) else { return };
    let Some(panel) = Panel::from_key(&key).filter(Panel::of_extension) else { return };
    let Some(at) = ctx.input(|input| input.pointer.latest_pos()).filter(|at| dock.contains(*at)) else { return };
    let tabs_shown = st.dock_entry() == Panel::Views && !st.dock_tabs.is_empty();
    let over_tabs = tabs_shown && st.dock_tabs.iter().any(|(_, rect)| at.y <= rect.bottom() + 4.0);
    if st.docked(&panel) && !over_tabs {
        return;
    }
    let others: Vec<(String, Rect)> = st.dock_tabs.iter().filter(|(tab, _)| *tab != key).cloned().collect();
    let before = match over_tabs {
        true => others.iter().find(|(_, rect)| at.x < rect.center().x).map(|(tab, _)| tab.clone()),
        false => None,
    };
    if over_tabs {
        let x = match (&before, others.last()) {
            (Some(next), _) => others.iter().find(|(tab, _)| tab == next).map_or(dock.left(), |(_, r)| r.left()) - 2.0,
            (None, Some((_, last))) => last.right() + 2.0,
            (None, None) => dock.left() + 8.0,
        };
        let band = st.dock_tabs[0].1.y_range();
        let painter =
            ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("dock drop line")));
        painter.vline(x, band, Stroke::new(2.0, pal().accent));
    } else {
        let title = entries.iter().find(|entry| entry.panel == panel).map_or_else(String::new, |e| e.tip.clone());
        egui::Area::new(egui::Id::unique("dock drop"))
            .order(egui::Order::Foreground)
            .interactable(false)
            .fixed_pos(dock.min)
            .show(ctx, |ui| {
                let (rect, _) = ui.allocate_exact_size(dock.size(), Sense::hover());
                let fill = pal().accent.gamma_multiply(0.14);
                ui.painter().rect(
                    rect.shrink(3.0),
                    10.0,
                    fill,
                    Stroke::new(2.0, pal().accent),
                    egui::StrokeKind::Inside,
                );
                let font = FontId::proportional(14.0);
                let text = format!("Keep {title} in this panel");
                ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, font, pal().text);
            });
    }
    if let Some(drag) = st.rail_drag.as_mut() {
        drag.target = Some(DropAt::Dock(before));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prefs::Prefs;
    use egui::pos2;

    fn state(docked: &[&str]) -> UiState {
        let prefs = Prefs { docked: docked.iter().map(|key| key.to_string()).collect(), ..Prefs::default() };
        UiState { prefs, ..UiState::default() }
    }

    fn chat() -> Panel {
        Panel::Webview("chat".to_string())
    }

    #[test]
    fn a_view_floats_as_a_card_until_it_is_docked() {
        let mut st = state(&[]);
        st.show(chat());
        assert!(st.cards.contains(&chat()), "the default: a card on the canvas");
        assert_eq!(st.dock, Panel::Files);
        assert!(st.dock_view(chat(), None));
        assert!(st.cards.is_empty(), "its card goes into the panel");
        assert_eq!((st.files, st.dock.clone()), (true, chat()));
        assert!(st.is_open(&chat()) && st.is_open(&Panel::Views));
        assert_eq!(st.prefs.docked, ["view:chat"]);
        assert!(!st.dock_view(Panel::Symbols, None), "only what an extension brings is docked");
    }

    #[test]
    fn floating_it_again_opens_its_card_and_gives_the_panel_back() {
        let mut st = state(&["view:chat", "tree:shelf"]);
        st.settle_docked(&[chat(), Panel::Tree("shelf".to_string())]);
        st.show(chat());
        assert!(st.float_view(&chat(), true));
        assert_eq!(st.dock, Panel::Tree("shelf".to_string()), "the next docked view takes its place");
        assert!(st.cards.contains(&chat()));
        assert!(st.float_view(&Panel::Tree("shelf".to_string()), false));
        assert_eq!(st.dock, Panel::Files, "nothing docked: the files");
        assert!(st.prefs.docked.is_empty());
        assert!(!st.float_view(&chat(), false), "a view that floats already");
    }

    #[test]
    fn the_icon_of_the_docked_views_opens_the_last_tab_shown() {
        let mut st = state(&["view:chat", "view:codex"]);
        let codex = Panel::Webview("codex".to_string());
        st.settle_docked(&[chat(), codex.clone()]);
        st.show(codex.clone());
        st.show(Panel::Files);
        assert!(!st.is_open(&Panel::Views));
        st.toggle(Panel::Views);
        assert_eq!(st.dock, codex);
        st.toggle(Panel::Views);
        assert!(!st.files, "a second click hides the panel");
    }

    #[test]
    fn what_another_computer_floated_leaves_the_panel() {
        let mut st = state(&["view:chat"]);
        st.restore(true, &["view:chat".to_string(), "symbols".to_string()]);
        assert_eq!(st.dock, chat(), "the docked view shown when the app closed is shown again");
        assert_eq!(st.cards, [Panel::Symbols]);
        st.prefs.docked.clear();
        st.settle_docked(&[chat()]);
        assert_eq!(st.dock, Panel::Files);
        let mut st = state(&["view:chat"]);
        st.cards.push(chat());
        st.settle_docked(&[chat()]);
        assert!(st.cards.is_empty(), "a docked view is never a card too");
    }

    #[test]
    fn a_docked_view_not_there_yet_keeps_its_tab() {
        let mut st = state(&["view:chat"]);
        st.restore(true, &["view:chat".to_string()]);
        st.settle_docked(&[]);
        assert_eq!(st.dock, chat(), "its extension has not said yet whether it shows it");
        let mut st = state(&["view:chat"]);
        st.restore(true, &["views".to_string()]);
        st.settle_docked(&[chat()]);
        assert_eq!(st.dock, chat());
    }

    #[test]
    fn tabs_lose_their_titles_when_they_do_not_fit() {
        assert_eq!(tab_widths(&[100.0, 120.0], 300.0), [100.0, 120.0]);
        assert_eq!(tab_widths(&[100.0, 120.0], 220.0), [NARROW_TAB, NARROW_TAB]);
        assert!(tab_widths(&[], 100.0).is_empty());
    }

    #[test]
    fn a_tab_dropped_on_the_canvas_is_a_card_under_the_pointer() {
        let canvas = Rect::from_min_max(pos2(300.0, 40.0), pos2(1500.0, 900.0));
        assert_eq!(card_at(canvas, pos2(800.0, 300.0)), [440.0, 212.0, 420.0, 640.0]);
        let [x, y, w, h] = card_at(canvas, pos2(1495.0, 895.0));
        assert!(x + w <= canvas.width() - 8.0 && y + h <= canvas.height() - 8.0, "kept inside the canvas");
        let [x, y, ..] = card_at(canvas, pos2(301.0, 41.0));
        assert_eq!((x, y), (8.0, 8.0));
    }
}

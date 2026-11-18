use egui::{CornerRadius, Frame, Margin, Pos2, Rect, RichText, Stroke};

use super::{menu_gap, menu_row, menu_row_key, pal, Action, UiState};
use crate::icons;
use crate::versions::{Bulk, Choice, Op};

pub struct VersionMenu {
    pub at: Pos2,
    pub path: std::path::PathBuf,
    pub line: u32,
    pub name: String,
    pub wanted: String,
    pub status: String,
    pub choices: Vec<Choice>,
    pub install: Option<String>,
    pub place: Option<Rect>,
}

pub(super) fn version_menu(ctx: &egui::Context, st: &mut UiState, canvas: Rect, actions: &mut Vec<Action>) {
    const WIDTH: f32 = 290.0;
    let Some(menu) = st.version_menu.as_mut() else { return };
    let pressed = ctx.input(|i| i.pointer.any_pressed().then(|| i.pointer.interact_pos()).flatten());
    let outside = pressed.is_some_and(|at| menu.place.is_some_and(|place| !place.contains(at)));
    if outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        st.version_menu = None;
        return;
    }
    let tall = menu.place.map_or(180.0, |place| place.height());
    let at = Pos2::new(
        menu.at.x.min(canvas.right() - WIDTH - 20.0).max(canvas.left() + 4.0),
        menu.at.y.min(canvas.bottom() - tall - 8.0).max(canvas.top() + 4.0),
    );
    let area = egui::Area::new(egui::Id::unique("version menu")).fixed_pos(at).order(egui::Order::Foreground);
    let mut chosen: Vec<Action> = Vec::new();
    let resp = area.show(ctx, |ui| {
        let frame = Frame::NONE
            .fill(pal().card)
            .stroke(Stroke::new(1.0, pal().border))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::same(6));
        frame.show(ui, |ui| {
            ui.set_width(WIDTH);
            ui.spacing_mut().item_spacing.y = 1.0;
            Frame::NONE.inner_margin(Margin { left: 10, right: 10, top: 4, bottom: 4 }).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&menu.name).size(13.0).strong().color(pal().text));
                    ui.label(RichText::new(&menu.wanted).size(12.5).color(pal().text_dim));
                });
                ui.label(RichText::new(&menu.status).size(11.5).color(pal().text_faint));
            });
            menu_gap(ui);
            if menu.choices.is_empty() {
                Frame::NONE.inner_margin(Margin::symmetric(10, 4)).show(ui, |ui| {
                    let text = "Nothing to change it to: it already takes the newest version, or it is a range written by hand.";
                    ui.add(egui::Label::new(RichText::new(text).size(12.0).color(pal().text_dim)).wrap());
                });
            }
            for choice in &menu.choices {
                if menu_row_key(ui, icons::ARROW_UP, &choice.text, &choice.label) {
                    chosen.push(Action::Versions(Op::Set {
                        path: menu.path.clone(),
                        line: menu.line,
                        name: menu.name.clone(),
                        text: choice.text.clone(),
                    }));
                }
            }
            menu_gap(ui);
            if menu_row(ui, icons::PACKAGE, "Update every dependency of the file") {
                chosen.push(Action::Versions(Op::Update(Bulk::Latest, Some(menu.path.clone()))));
            }
            if let Some(command) = &menu.install {
                if menu_row(ui, icons::DOWNLOAD, &format!("Run {command} in the console")) {
                    chosen.push(Action::Versions(Op::Install(Some(menu.path.clone()))));
                }
            }
        });
    });
    let Some(menu) = st.version_menu.as_mut() else { return };
    menu.place = Some(resp.response.rect);
    st.floating.push(resp.response.rect);
    if !chosen.is_empty() {
        actions.extend(chosen);
        st.version_menu = None;
    }
}

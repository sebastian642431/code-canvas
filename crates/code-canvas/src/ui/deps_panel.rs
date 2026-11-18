use std::collections::HashMap;

use canvas_core::deps::DepKind;
use canvas_core::layout::NodeKind;
use egui::{Align, Align2, FontId, Frame, Layout, Margin, Pos2, Rect, RichText, Sense, Vec2};

use super::{c, check, pal, panel_title, section, well, Action, UiState, ROW_H};
use crate::icons;
use crate::page::{Page, Ready};
use canvas_render::theme;

fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn neighbours(r: &Ready, node: usize, outgoing: bool, kinds: &[bool; 5]) -> Vec<(u32, u32)> {
    let Some(n) = r.layout.nodes.get(node) else { return Vec::new() };
    let range = node as u32..n.subtree_end;
    let inside = |f: u32| r.layout.file_anchor.get(f as usize).is_some_and(|a| range.contains(a));
    let files: Vec<u32> = match n.kind {
        NodeKind::File(fi) => vec![fi as u32],
        NodeKind::Dir => (0..r.repo.files.len() as u32).filter(|f| inside(*f)).collect(),
    };
    let mut count: HashMap<u32, u32> = HashMap::new();
    for f in files {
        if outgoing {
            for e in r.deps.outgoing(f).iter().filter(|e| kinds[e.kind.index()] && !inside(e.to)) {
                *count.entry(e.to).or_default() += 1;
            }
        } else {
            for e in r.deps.incoming(f).filter(|e| kinds[e.kind.index()] && !inside(e.from)) {
                *count.entry(e.from).or_default() += 1;
            }
        }
    }
    let mut out: Vec<(u32, u32)> = count.into_iter().collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

#[derive(Clone, Copy, PartialEq)]
enum Sample {
    Edge,
    Cycle,
    Bundle,
}

fn legend(ui: &mut egui::Ui, sample: Sample, text: &str) {
    let color = c(theme::get().dep_out);
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(46.0, 16.0), Sense::hover());
        let painter = ui.painter();
        let y = rect.center().y;
        let (start, end) = (rect.left() + 2.0, rect.right() - 4.0);
        let width = if sample == Sample::Bundle { 4.0 } else { 1.5 };
        let stroke = egui::Stroke::new(width, color);
        match sample {
            Sample::Cycle => {
                let mut x = start;
                while x < end - 7.0 {
                    painter.line_segment([Pos2::new(x, y), Pos2::new((x + 4.2).min(end - 7.0), y)], stroke);
                    x += 7.0;
                }
            }
            _ => {
                painter.line_segment([Pos2::new(start, y), Pos2::new(end - 6.0, y)], stroke);
            }
        }
        let head = if sample == Sample::Bundle { 6.5 } else { 3.6 };
        let tip = vec![Pos2::new(end, y), Pos2::new(end - head * 2.2, y - head), Pos2::new(end - head * 2.2, y + head)];
        painter.add(egui::Shape::convex_polygon(tip, color, egui::Stroke::NONE));
        if sample == Sample::Bundle {
            let chip = Rect::from_center_size(Pos2::new(rect.center().x - 4.0, y), Vec2::new(18.0, 13.0));
            painter.rect(chip, 6.5, c(theme::get().chip_fill), egui::Stroke::new(1.0, color), egui::StrokeKind::Inside);
            painter.text(
                chip.center(),
                Align2::CENTER_CENTER,
                "12",
                FontId::proportional(9.5),
                c(theme::get().chip_text),
            );
        }
        ui.add(egui::Label::new(RichText::new(text).size(11.5).color(pal().text_dim)).wrap());
    });
}

pub fn deps_panel(ui: &mut egui::Ui, st: &mut UiState, page: Option<&Page>, actions: &mut Vec<Action>) {
    Frame::NONE.inner_margin(Margin { left: 10, right: 10, top: 10, bottom: 0 }).show(ui, |ui| {
        panel_title(ui, "Dependencies", |_| {});
    });
    let Some((page, r)) = page.and_then(|p| Some((p, p.ready()?))) else {
        ui.label(RichText::new("    Open a folder to see its dependencies.").size(11.5).color(pal().text_dim));
        return;
    };
    let deps = &r.deps;
    let before = st.deps;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        Frame::NONE.inner_margin(Margin { left: 10, right: 10, top: 0, bottom: 8 }).show(ui, |ui| {
            ui.label(
                RichText::new(format!(
                    "{} edges between {} pairs of files · read from import statements in {} ms",
                    thousands(deps.edges.len()),
                    thousands(deps.pairs()),
                    deps.ms
                ))
                .size(11.5)
                .color(pal().text_dim),
            );

            section(ui, icons::EYE, "Show");
            well(ui, |ui| {
                check(ui, &mut st.deps.selected, "Edges of the selected file or folder", true, false);
                check(ui, &mut st.deps.all, "Every edge, faint", true, false);
            });
            ui.add_space(4.0);
            legend(ui, Sample::Edge, "an import: the arrow points at the file imported");
            let cycles = match deps.cycles {
                0 => "part of an import cycle (there are none)".to_string(),
                1 => format!("part of an import cycle · 1 cycle, {} edges", thousands(deps.cyclic_edges)),
                n => format!("part of an import cycle · {n} cycles, {} edges", thousands(deps.cyclic_edges)),
            };
            match page.arranged {
                true => legend(ui, Sample::Cycle, &format!("against the order of the folder, or {cycles}")),
                false => legend(ui, Sample::Cycle, &cycles),
            }
            if page.dep_zoom.is_some() {
                legend(ui, Sample::Bundle, "imports between two folders, together: zoom in or select a file inside");
            }

            section(ui, icons::LAYOUT_GRID, "Arrange");
            well(ui, |ui| {
                let mut on = page.arranged;
                if check(ui, &mut on, "Arrange the folder by its imports", true, false).clicked() {
                    actions.push(Action::Arrange(on));
                }
                ui.add(
                    egui::Label::new(
                        RichText::new(
                            "In every folder, what is imported comes first, from the top left, and what imports \
                             it after it. Files and folders that import each other stay together.",
                        )
                        .size(11.0)
                        .color(pal().text_faint),
                    )
                    .wrap(),
                );
            });

            section(ui, icons::SHARE_2, "Relationships");
            well(ui, |ui| {
                for kind in DepKind::ALL {
                    let n = deps.counts[kind.index()] as usize;
                    ui.horizontal(|ui| {
                        check(ui, &mut st.deps.kinds[kind.index()], kind.label(), n > 0, false);
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new(thousands(n)).monospace().size(12.0).color(if n > 0 {
                                pal().text_dim
                            } else {
                                pal().text_faint
                            }));
                        });
                    });
                }
                ui.add_space(2.0);
                ui.label(
                    RichText::new(format!(
                        "{} imports of packages outside the folder · {} that point at no file here",
                        thousands(deps.external as usize),
                        thousands(deps.unresolved as usize)
                    ))
                    .size(11.0)
                    .color(pal().text_faint),
                );
            });

            let Some((node, n)) = page.selected.and_then(|i| Some((i, r.layout.nodes.get(i)?))) else {
                section(ui, icons::WAYPOINTS, "Selection");
                ui.label(
                    RichText::new("    Click a file or a folder to see what it depends on.")
                        .size(11.5)
                        .color(pal().text_faint),
                );
                return;
            };
            let name = if n.rel_path.is_empty() { r.repo.name.clone() } else { n.name.clone() };
            section(ui, icons::WAYPOINTS, &name);
            let (drawn, total) = page.dep_edges;
            if total > drawn {
                ui.label(
                    RichText::new(format!("    {} of its {} edges are drawn", thousands(drawn), thousands(total)))
                        .size(11.0)
                        .color(pal().text_faint),
                );
            }
            ui.spacing_mut().item_spacing.y = 0.0;
            for (outgoing, title, color) in
                [(true, "Depends on", theme::get().dep_out), (false, "Used by", theme::get().dep_in)]
            {
                let list = neighbours(r, node, outgoing, &st.deps.kinds);
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    let (dot, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                    ui.painter().circle_filled(dot.center(), 4.0, c(color));
                    ui.label(
                        RichText::new(format!("{title}  ·  {}", thousands(list.len()))).size(12.5).color(pal().text),
                    );
                });
                for (file, count) in list.iter().take(200) {
                    let f = &r.repo.files[*file as usize];
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
                    if !ui.is_rect_visible(rect) {
                        continue;
                    }
                    let painter = ui.painter();
                    if resp.hovered() {
                        painter.rect_filled(rect.shrink2(Vec2::new(4.0, 0.0)), 6.0, pal().row);
                    }
                    let mid = rect.center().y;
                    let mut right = rect.right() - 10.0;
                    if *count > 1 {
                        let g = painter.layout_no_wrap(count.to_string(), FontId::monospace(11.0), pal().text_faint);
                        right -= g.size().x;
                        painter.galley(Pos2::new(right, mid - g.size().y * 0.5), g, pal().text_faint);
                        right -= 6.0;
                    }
                    let clip =
                        Rect::from_min_max(Pos2::new(rect.left() + 22.0, rect.top()), Pos2::new(right, rect.bottom()));
                    let clipped = painter.with_clip_rect(clip);
                    let ng = clipped.layout_no_wrap(f.name.clone(), FontId::proportional(12.5), pal().text);
                    let nw = ng.size().x;
                    clipped.galley(Pos2::new(clip.left(), mid - ng.size().y * 0.5), ng, pal().text);
                    let dir = f.rel_path.rsplit_once('/').map_or("", |(d, _)| d);
                    clipped.text(
                        Pos2::new(clip.left() + nw + 8.0, mid),
                        Align2::LEFT_CENTER,
                        dir,
                        FontId::proportional(11.5),
                        pal().text_faint,
                    );
                    if resp.clicked() {
                        actions.push(Action::FlyToPath(f.rel_path.clone()));
                    }
                    resp.on_hover_text(&f.rel_path);
                }
                if list.len() > 200 {
                    ui.label(
                        RichText::new(format!("        and {} more", thousands(list.len() - 200)))
                            .size(11.0)
                            .color(pal().text_faint),
                    );
                }
                if list.is_empty() {
                    ui.label(RichText::new("        nothing outside it").size(11.5).color(pal().text_faint));
                }
            }
        });
    });
    if st.deps != before {
        actions.push(Action::DepsChanged);
    }
}

//! Panels for extensions: the registry and what is installed, the problems
//! extensions report, and their messages.

use canvas_core::Site;
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Frame, Layout, Margin, Pos2, Rect, RichText, Sense, Stroke, Vec2,
};

use super::{
    c, icon, panel_title, search_field, section, small_icon, well, Action, UiState, ACCENT, BORDER, CARD, HOVER,
    RAISED, ROW_H, TEXT, TEXT_DIM, TEXT_FAINT,
};
use crate::ext::{Extensions, Installed, Level, NodeStatus, Severity, Status};
use crate::icons;
use crate::page::Page;
use canvas_render::theme;

fn severity_color(s: Severity) -> Color32 {
    c(match s {
        Severity::Error => theme::PROBLEM_ERROR,
        Severity::Warning => theme::PROBLEM_WARNING,
        Severity::Info | Severity::Hint => theme::PROBLEM_INFO,
    })
}

fn severity_glyph(s: Severity) -> &'static str {
    match s {
        Severity::Error => icons::CIRCLE_ALERT,
        Severity::Warning => icons::TRIANGLE_ALERT,
        Severity::Info | Severity::Hint => icons::INFO,
    }
}

/// A small rounded button with a label.
fn pill(ui: &mut egui::Ui, label: &str, strong: bool) -> bool {
    let font = FontId::proportional(12.0);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, TEXT);
    let size = Vec2::new(galley.size().x + 18.0, 22.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let fill = match (strong, resp.hovered()) {
        (true, false) => ACCENT.gamma_multiply(0.85),
        (true, true) => ACCENT,
        (false, false) => RAISED,
        (false, true) => HOVER,
    };
    ui.painter().rect_filled(rect, 6.0, fill);
    let color = if strong { Color32::from_rgb(8, 10, 16) } else { TEXT };
    ui.painter().galley(rect.center() - galley.size() * 0.5, galley, color);
    resp.clicked()
}

fn compact(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{:.1}K", n as f64 / 1e3),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

enum Op {
    Uninstall(String),
    Enable(String, bool),
    Install(String, String),
}

pub fn extensions_panel(ui: &mut egui::Ui, ext: &mut Extensions, actions: &mut Vec<Action>) {
    Frame::NONE.inner_margin(Margin { left: 12, right: 8, top: 10, bottom: 6 }).show(ui, |ui| {
        panel_title(ui, "Extensions", |ui| {
            if small_icon(ui, icons::FILE_ARCHIVE, "Install from a .vsix file") {
                actions.push(Action::InstallVsix);
            }
        });
        ui.add_space(6.0);
        let resp = search_field(ui, &mut ext.query, "Search VS Code extensions", icons::SEARCH);
        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            ext.search();
        }
        ui.label(RichText::new("Enter to search Open VSX, the open extension registry").size(11.0).color(TEXT_FAINT));
        if let NodeStatus::Missing(why) = &ext.node {
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "Node.js was not found. Extensions that run code (linters, formatters, language servers) need it.",
                )
                .size(11.5)
                .color(c(theme::PROBLEM_WARNING)),
            )
            .on_hover_text(why);
        }
    });

    let mut op = None;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        Frame::NONE.inner_margin(Margin { left: 10, right: 10, top: 0, bottom: 8 }).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 5.0;
            section(ui, icons::PACKAGE, &format!("Installed  ·  {}", ext.installed.len()));
            if let (NodeStatus::Ready(version), true) = (&ext.node, ext.host_running) {
                ui.label(RichText::new(format!("    running on Node.js {version}")).size(11.0).color(TEXT_FAINT));
            }
            if ext.installed.is_empty() && ext.installing.is_empty() {
                well(ui, |ui| {
                    ui.label(
                        RichText::new(
                            "Nothing yet. Search above: ESLint and Prettier are a good start for a JavaScript or TypeScript project.",
                        )
                        .size(12.0)
                        .color(TEXT_DIM),
                    );
                });
            }
            for label in &ext.installing {
                well(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new(format!("Installing {label} …")).size(12.5).color(TEXT));
                    });
                });
            }
            for e in &ext.installed {
                well(ui, |ui| {
                    ui.horizontal(|ui| {
                        let name = ui.label(RichText::new(&e.display_name).size(13.0).color(if e.enabled { TEXT } else { TEXT_DIM }));
                        if !e.description.is_empty() {
                            name.on_hover_text(&e.description);
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            if small_icon(ui, icons::TRASH_2, "Uninstall") {
                                op = Some(Op::Uninstall(e.id.clone()));
                            }
                            let (glyph, tip) =
                                if e.enabled { (icons::SQUARE_CHECK, "Enabled: click to disable") } else { (icons::SQUARE, "Disabled: click to enable") };
                            if small_icon(ui, glyph, tip) {
                                op = Some(Op::Enable(e.id.clone(), !e.enabled));
                            }
                        });
                    });
                    let (state, color) = match e.status {
                        Status::Active => (format!("running · started in {} ms", e.activation_ms), c(0x3ddc84ff)),
                        Status::Activating => ("starting …".to_string(), TEXT_DIM),
                        Status::Failed => ("failed to start".to_string(), c(theme::PROBLEM_ERROR)),
                        Status::Declarative => ("data only (nothing to run)".to_string(), TEXT_DIM),
                        Status::Disabled => ("disabled".to_string(), TEXT_FAINT),
                        Status::Idle if matches!(ext.node, NodeStatus::Missing(_)) => ("needs Node.js".to_string(), c(theme::PROBLEM_WARNING)),
                        Status::Idle if ext.host_running => ("waiting for a file it handles".to_string(), TEXT_DIM),
                        Status::Idle => ("starts when a folder is open".to_string(), TEXT_DIM),
                    };
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.label(RichText::new(format!("{} · {} · ", e.publisher, e.version)).size(11.5).color(TEXT_FAINT));
                        ui.label(RichText::new(state).size(11.5).color(color));
                    });
                    if e.status == Status::Failed && !e.error.is_empty() {
                        ui.label(RichText::new(&e.error).size(11.5).color(c(theme::PROBLEM_ERROR)));
                    }
                    if !e.unsupported.is_empty() {
                        ui.label(
                            RichText::new(format!("{} VS Code features it asked for are not available yet", e.unsupported.len()))
                                .size(11.0)
                                .color(TEXT_FAINT),
                        )
                        .on_hover_text(e.unsupported.join("\n"));
                    }
                });
            }

            if ext.searching || !ext.results.is_empty() || !ext.market_error.is_empty() {
                section(ui, icons::DOWNLOAD, "Open VSX");
            }
            if ext.searching {
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    ui.spinner();
                    ui.label(RichText::new("Searching …").size(12.0).color(TEXT_DIM));
                });
            }
            if !ext.market_error.is_empty() {
                well(ui, |ui| {
                    ui.label(RichText::new(format!("The registry could not be reached: {}", ext.market_error)).size(12.0).color(c(theme::PROBLEM_ERROR)));
                });
            }
            for item in &ext.results {
                let id = item.id();
                well(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&item.display_name).size(13.0).color(TEXT));
                        if item.verified {
                            ui.label(icon(icons::CIRCLE_CHECK, 12.0, ACCENT)).on_hover_text("Verified publisher");
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ext.installing.contains(&id) {
                                ui.spinner();
                            } else if ext.is_installed(&id) {
                                ui.label(RichText::new("installed").size(11.5).color(TEXT_FAINT));
                            } else if pill(ui, "Install", true) {
                                op = Some(Op::Install(item.namespace.clone(), item.name.clone()));
                            }
                        });
                    });
                    ui.label(
                        RichText::new(format!("{} · {} downloads · {}", item.namespace, compact(item.downloads), item.version))
                            .size(11.5)
                            .color(TEXT_FAINT),
                    );
                    if !item.description.is_empty() {
                        let short: String = item.description.chars().take(140).collect();
                        ui.label(RichText::new(short).size(12.0).color(TEXT_DIM));
                    }
                });
            }
        });
    });
    match op {
        Some(Op::Uninstall(id)) => ext.uninstall(&id),
        Some(Op::Enable(id, on)) => ext.set_enabled(&id, on),
        Some(Op::Install(namespace, name)) => ext.install(&namespace, &name),
        None => {}
    }
}

pub fn problems_panel(ui: &mut egui::Ui, page: Option<&Page>, ext: &Extensions, actions: &mut Vec<Action>) {
    let mut files: Vec<_> = page.map(|p| ext.diagnostics_under(&p.root).collect()).unwrap_or_default();
    files.sort_by(|a, b| a.0.cmp(b.0));
    let errors: u32 = files.iter().map(|(_, d)| d.errors).sum();
    let warnings: u32 = files.iter().map(|(_, d)| d.warnings).sum();

    Frame::NONE.inner_margin(Margin { left: 12, right: 8, top: 10, bottom: 6 }).show(ui, |ui| {
        panel_title(ui, "Problems", |_| {});
        ui.add_space(4.0);
        let summary = if files.is_empty() {
            if !ext.installed.iter().any(|e| e.enabled && e.has_code) {
                "Problems come from extensions. Install a linter (ESLint, for example) from the Extensions panel.".to_string()
            } else {
                "No problems in the files you have looked at. Linters check the files whose code is readable on screen: zoom into one.".to_string()
            }
        } else {
            format!("{errors} errors · {warnings} warnings · in {} files", files.len())
        };
        ui.label(RichText::new(summary).size(11.5).color(TEXT_DIM));
    });
    let Some(repo) = page.and_then(|p| p.ready()).map(|r| &r.repo) else { return };

    enum Row<'a> {
        File(&'a str, u32, u32),
        Item(&'a str, usize),
    }
    let mut rows = Vec::new();
    for (rel, d) in &files {
        rows.push(Row::File(rel, d.errors, d.warnings));
        rows.extend((0..d.items.len()).map(|i| Row::Item(rel, i)));
    }
    let lookup = |rel: &str| files.iter().find(|(r, _)| *r == rel).map(|(_, d)| *d);

    ui.spacing_mut().item_spacing.y = 0.0;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, ROW_H, rows.len(), |ui, range| {
        for row in &rows[range] {
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
            if !ui.is_rect_visible(rect) {
                continue;
            }
            let painter = ui.painter();
            if resp.hovered() {
                painter.rect_filled(rect.shrink2(Vec2::new(6.0, 0.0)), 6.0, Color32::from_rgb(20, 22, 27));
            }
            let mid = rect.center().y;
            match row {
                Row::File(rel, errors, warnings) => {
                    let name = rel.rsplit('/').next().unwrap_or(rel);
                    let galley = painter.layout_no_wrap(name.to_string(), FontId::proportional(12.5), TEXT);
                    let w = galley.size().x;
                    painter.galley(Pos2::new(rect.left() + 12.0, mid - galley.size().y * 0.5), galley, TEXT);
                    let counts = format!("{}", errors + warnings);
                    let cg = painter.layout_no_wrap(counts, FontId::proportional(11.5), TEXT_DIM);
                    let cx = rect.right() - 12.0 - cg.size().x;
                    painter.galley(Pos2::new(cx, mid - cg.size().y * 0.5), cg, TEXT_DIM);
                    let dir = rel.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
                    let clip = Rect::from_min_max(
                        Pos2::new(rect.left() + 20.0 + w, rect.top()),
                        Pos2::new(cx - 6.0, rect.bottom()),
                    );
                    painter.with_clip_rect(clip).text(
                        clip.left_center(),
                        Align2::LEFT_CENTER,
                        dir,
                        FontId::proportional(11.5),
                        TEXT_FAINT,
                    );
                    if resp.clicked() {
                        actions.push(Action::FlyToPath(rel.to_string()));
                    }
                    resp.on_hover_text(*rel);
                }
                Row::Item(rel, i) => {
                    let Some(d) = lookup(rel).and_then(|f| f.items.get(*i)) else { continue };
                    painter.text(
                        Pos2::new(rect.left() + 22.0, mid),
                        Align2::LEFT_CENTER,
                        severity_glyph(d.severity),
                        FontId::proportional(13.0),
                        severity_color(d.severity),
                    );
                    let place = format!("{}", d.line + 1);
                    let pg = painter.layout_no_wrap(place, FontId::monospace(11.0), TEXT_FAINT);
                    let px = rect.right() - 12.0 - pg.size().x;
                    painter.galley(Pos2::new(px, mid - pg.size().y * 0.5), pg, TEXT_FAINT);
                    let clip = Rect::from_min_max(
                        Pos2::new(rect.left() + 42.0, rect.top()),
                        Pos2::new(px - 6.0, rect.bottom()),
                    );
                    let first = d.message.lines().next().unwrap_or_default();
                    painter.with_clip_rect(clip).text(
                        clip.left_center(),
                        Align2::LEFT_CENTER,
                        first,
                        FontId::proportional(12.0),
                        TEXT,
                    );
                    if resp.clicked() {
                        if let Some(file) = repo.file_index(rel) {
                            let len = d.end_col.saturating_sub(d.col).clamp(1, 400) as u16;
                            actions.push(Action::FlyToSite(Site {
                                file: file as u32,
                                line: d.line,
                                col: d.col.min(u16::MAX as u32) as u16,
                                len,
                            }));
                        }
                    }
                    resp.on_hover_text(format!("{}\n{}", d.title(), d.message));
                }
            }
        }
    });
}

/// The extension's own icon as a texture, decoded the first time it is shown.
pub fn extension_icon(
    ctx: &egui::Context,
    cache: &mut std::collections::HashMap<String, Option<egui::TextureHandle>>,
    e: &Installed,
) -> Option<egui::TextureHandle> {
    cache
        .entry(e.id.clone())
        .or_insert_with(|| {
            let bytes = std::fs::read(e.icon.as_ref()?).ok()?;
            let small = image::load_from_memory(&bytes).ok()?.thumbnail(64, 64).into_rgba8();
            let size = [small.width() as usize, small.height() as usize];
            let pixels = egui::ColorImage::from_rgba_unmultiplied(size, small.as_raw());
            Some(ctx.load_texture(format!("ext-icon-{}", e.id), pixels, egui::TextureOptions::LINEAR))
        })
        .clone()
}

/// One extension's place in the side bar: what it is, and what it offers.
pub fn extension_view(ui: &mut egui::Ui, st: &mut UiState, ext: &mut Extensions) {
    let Some(e) = ext.installed.iter().find(|e| e.id == st.extension) else {
        Frame::NONE.inner_margin(Margin::same(12)).show(ui, |ui| {
            ui.label(RichText::new("This extension is not installed any more.").size(12.0).color(TEXT_DIM));
        });
        return;
    };
    let mut run = None;
    Frame::NONE.inner_margin(Margin { left: 12, right: 8, top: 10, bottom: 6 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            if let Some(texture) = extension_icon(ui.ctx(), &mut st.ext_icons, e) {
                ui.add(egui::Image::new(&texture).fit_to_exact_size(Vec2::splat(22.0)).corner_radius(4));
            }
            ui.label(RichText::new(&e.display_name).size(14.0).color(TEXT));
        });
        ui.label(RichText::new(format!("{} · {}", e.publisher, e.version)).size(11.5).color(TEXT_FAINT));
        if !e.description.is_empty() {
            ui.add_space(2.0);
            ui.label(RichText::new(&e.description).size(12.0).color(TEXT_DIM));
        }
        if e.status == Status::Failed {
            ui.label(
                RichText::new(format!("It failed to start: {}", e.error)).size(11.5).color(c(theme::PROBLEM_ERROR)),
            );
        }
    });
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        Frame::NONE.inner_margin(Margin { left: 10, right: 10, top: 0, bottom: 8 }).show(ui, |ui| {
            if !e.views.is_empty() {
                section(ui, icons::LAYOUT_GRID, "Views");
                well(ui, |ui| {
                    ui.label(RichText::new(e.views.join("  ·  ")).size(12.0).color(TEXT));
                    ui.label(
                        RichText::new("Shown by VS Code in the side bar. Code Canvas cannot show an extension's own views and webviews yet.")
                            .size(11.5)
                            .color(TEXT_FAINT),
                    );
                });
            }
            section(ui, icons::CORNER_DOWN_LEFT, &format!("Commands  ·  {}", e.commands.len()));
            ui.spacing_mut().item_spacing.y = 0.0;
            for (id, title) in &e.commands {
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), Sense::click());
                if !ui.is_rect_visible(rect) {
                    continue;
                }
                if resp.hovered() {
                    ui.painter().rect_filled(rect, 6.0, Color32::from_rgb(20, 22, 27));
                }
                let clip = rect.shrink2(Vec2::new(8.0, 0.0));
                ui.painter().with_clip_rect(clip).text(clip.left_center(), Align2::LEFT_CENTER, title, FontId::proportional(12.5), TEXT);
                if resp.on_hover_text(id).clicked() {
                    run = Some((id.clone(), title.clone()));
                }
            }
            if e.commands.is_empty() {
                ui.label(RichText::new("    It has no commands: it works on its own.").size(11.5).color(TEXT_FAINT));
            }
        });
    });
    if let Some((id, title)) = run {
        ext.run_command(&id, &title);
    }
}

/// Messages from extensions, stacked in the canvas corner; they fade by themselves.
pub fn notices(ctx: &egui::Context, canvas: Rect, ext: &Extensions) {
    if ext.notices.is_empty() {
        return;
    }
    egui::Area::new(egui::Id::unique("notices"))
        .pivot(Align2::RIGHT_BOTTOM)
        .fixed_pos(canvas.right_bottom() - Vec2::new(12.0, 12.0))
        .order(egui::Order::Foreground)
        .interactable(false)
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            {
                for n in &ext.notices {
                    let color = match n.level {
                        Level::Info => ACCENT,
                        Level::Warn => c(theme::PROBLEM_WARNING),
                        Level::Error => c(theme::PROBLEM_ERROR),
                    };
                    Frame::NONE
                        .fill(CARD)
                        .stroke(Stroke::new(1.0, BORDER))
                        .corner_radius(CornerRadius::same(8))
                        .inner_margin(Margin::symmetric(10, 7))
                        .show(ui, |ui| {
                            ui.set_max_width(420.0);
                            ui.horizontal_wrapped(|ui| {
                                ui.label(icon(icons::PUZZLE, 13.0, color));
                                ui.label(RichText::new(&n.source).size(12.0).color(TEXT));
                                let text: String = n.text.chars().take(300).collect();
                                ui.label(RichText::new(text).size(12.0).color(TEXT_DIM));
                            });
                        });
                }
            }
        });
}

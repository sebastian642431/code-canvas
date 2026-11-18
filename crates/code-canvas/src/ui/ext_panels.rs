use canvas_core::Site;
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Frame, Layout, Margin, Pos2, Rect, RichText, Sense, Stroke, Vec2,
};

use super::{c, icon, pal, panel_title, search_field, section, small_icon, well, Action, Panel, UiState, ROW_H};
use crate::ext::views::{MenuItem, ShownRow, TreeRow};
use crate::ext::{Extensions, Installed, Level, NodeStatus, Severity, Status};
use crate::icons;
use crate::page::Page;
use canvas_render::theme;

fn severity_color(s: Severity) -> Color32 {
    c(match s {
        Severity::Error => theme::get().problem_error,
        Severity::Warning => theme::get().problem_warning,
        Severity::Info | Severity::Hint => theme::get().problem_info,
    })
}

fn severity_glyph(s: Severity) -> &'static str {
    match s {
        Severity::Error => icons::CIRCLE_ALERT,
        Severity::Warning => icons::TRIANGLE_ALERT,
        Severity::Info | Severity::Hint => icons::INFO,
    }
}

pub(super) fn pill(ui: &mut egui::Ui, label: &str, strong: bool) -> bool {
    let font = FontId::proportional(12.0);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, pal().text);
    let size = Vec2::new(galley.size().x + 18.0, 22.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let fill = match (strong, resp.hovered()) {
        (true, false) => pal().accent.gamma_multiply(0.85),
        (true, true) => pal().accent,
        (false, false) => pal().raised,
        (false, true) => pal().hover,
    };
    ui.painter().rect_filled(rect, 6.0, fill);
    let color = if strong { pal().on_accent } else { pal().text };
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

fn icon_slot(ui: &egui::Ui, slot: Rect, texture: Option<&egui::TextureHandle>, enabled: bool) {
    match texture {
        Some(t) => {
            let tint = if enabled { Color32::WHITE } else { Color32::from_gray(110) };
            egui::Image::new(t).corner_radius(6).tint(tint).paint_at(ui, slot);
        }
        None => {
            ui.painter().rect_filled(slot, 8.0, pal().raised);
            ui.painter().text(
                slot.center(),
                Align2::CENTER_CENTER,
                icons::PUZZLE,
                FontId::proportional(18.0),
                pal().text_dim,
            );
        }
    }
}

pub fn extensions_panel(ui: &mut egui::Ui, st: &mut UiState, ext: &mut Extensions, actions: &mut Vec<Action>) {
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
        ui.label(
            RichText::new("Enter to search Open VSX, the open extension registry").size(11.0).color(pal().text_faint),
        );
        if let NodeStatus::Missing(why) = &ext.node {
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "Node.js was not found. Extensions that run code (linters, formatters, language servers) need it.",
                )
                .size(11.5)
                .color(c(theme::get().problem_warning)),
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
                ui.label(RichText::new(format!("    running on Node.js {version}")).size(11.0).color(pal().text_faint));
            }
            if ext.installed.is_empty() && ext.installing.is_empty() {
                well(ui, |ui| {
                    ui.label(
                        RichText::new(
                            "Nothing yet. Search above: ESLint and Prettier are a good start for a JavaScript or TypeScript project.",
                        )
                        .size(12.0)
                        .color(pal().text_dim),
                    );
                });
            }
            for label in &ext.installing {
                well(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new(format!("Installing {label} …")).size(12.5).color(pal().text));
                    });
                });
            }
            for e in &ext.installed {
                let texture = extension_icon(ui.ctx(), &mut st.ext_icons, e);
                let (state, color) = match e.status {
                    Status::Active => (format!("{}  {} ms", icons::HISTORY, e.activation_ms), pal().text_faint),
                    Status::Activating => ("starting …".to_string(), pal().text_dim),
                    Status::Failed => ("failed to start".to_string(), c(theme::get().problem_error)),
                    Status::Declarative => (String::new(), pal().text_faint),
                    Status::Disabled => ("disabled".to_string(), pal().text_faint),
                    Status::Idle if matches!(ext.node, NodeStatus::Missing(_)) => ("needs Node.js".to_string(), c(theme::get().problem_warning)),
                    Status::Idle => (String::new(), pal().text_faint),
                };
                let mut open = false;
                let mut row = Frame::NONE.corner_radius(CornerRadius::same(8)).inner_margin(Margin::symmetric(8, 7)).begin(ui);
                {
                    let ui = &mut row.content_ui;
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        let (slot, icon_resp) = ui.allocate_exact_size(Vec2::splat(38.0), Sense::click());
                        icon_slot(ui, slot, texture.as_ref(), e.enabled);
                        open |= icon_resp.clicked();
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if !state.is_empty() {
                                    ui.label(RichText::new(&state).size(11.0).color(color));
                                }
                                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                    let name_color = if e.enabled { pal().text } else { pal().text_dim };
                                    let name = RichText::new(&e.display_name).size(13.5).strong().color(name_color);
                                    open |= ui.add(egui::Label::new(name).truncate().sense(Sense::click())).clicked();
                                });
                            });
                            let second = if e.status == Status::Failed && !e.error.is_empty() { &e.error } else { &e.description };
                            let second_color = if e.status == Status::Failed { c(theme::get().problem_error) } else { pal().text_dim };
                            ui.add(egui::Label::new(RichText::new(second).size(12.0).color(second_color)).truncate());
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.spacing_mut().item_spacing.x = 0.0;
                                if small_icon(ui, icons::TRASH_2, "Uninstall") {
                                    op = Some(Op::Uninstall(e.id.clone()));
                                }
                                let (glyph, tip) = if e.enabled {
                                    (icons::SQUARE_CHECK, "Enabled: click to disable")
                                } else {
                                    (icons::SQUARE, "Disabled: click to enable")
                                };
                                if small_icon(ui, glyph, tip) {
                                    op = Some(Op::Enable(e.id.clone(), !e.enabled));
                                }
                                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                    let mut by = e.publisher.clone();
                                    if !e.unsupported.is_empty() {
                                        by.push_str(&format!("  ·  {} features not available yet", e.unsupported.len()));
                                    }
                                    let label = ui.add(egui::Label::new(RichText::new(by).size(11.5).color(pal().text_faint)).truncate());
                                    if !e.unsupported.is_empty() {
                                        label.on_hover_text(e.unsupported.join("\n"));
                                    }
                                });
                            });
                        });
                    });
                }
                if ui.rect_contains_pointer(row.content_ui.min_rect().expand(6.0)) {
                    row.frame.fill = pal().well;
                }
                row.end(ui);
                if open {
                    st.extension = e.id.clone();
                    st.show(Panel::Extension);
                }
            }

            if ext.searching || !ext.results.is_empty() || !ext.market_error.is_empty() {
                section(ui, icons::DOWNLOAD, "Open VSX");
            }
            if ext.searching {
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    ui.spinner();
                    ui.label(RichText::new("Searching …").size(12.0).color(pal().text_dim));
                });
            }
            if !ext.market_error.is_empty() {
                well(ui, |ui| {
                    ui.label(RichText::new(format!("The registry could not be reached: {}", ext.market_error)).size(12.0).color(c(theme::get().problem_error)));
                });
            }
            for item in &ext.results {
                let id = item.id();
                well(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&item.display_name).size(13.5).strong().color(pal().text));
                        if item.verified {
                            ui.label(icon(icons::CIRCLE_CHECK, 12.0, pal().accent)).on_hover_text("Verified publisher");
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ext.installing.contains(&id) {
                                ui.spinner();
                            } else if ext.is_installed(&id) {
                                ui.label(RichText::new("installed").size(11.5).color(pal().text_faint));
                            } else if pill(ui, "Install", true) {
                                op = Some(Op::Install(item.namespace.clone(), item.name.clone()));
                            }
                        });
                    });
                    ui.label(
                        RichText::new(format!("{} · {} downloads · {}", item.namespace, compact(item.downloads), item.version))
                            .size(11.5)
                            .color(pal().text_faint),
                    );
                    if !item.description.is_empty() {
                        let short: String = item.description.chars().take(140).collect();
                        ui.label(RichText::new(short).size(12.0).color(pal().text_dim));
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
        ui.label(RichText::new(summary).size(11.5).color(pal().text_dim));
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
                painter.rect_filled(rect.shrink2(Vec2::new(6.0, 0.0)), 6.0, pal().row);
            }
            let mid = rect.center().y;
            match row {
                Row::File(rel, errors, warnings) => {
                    let name = rel.rsplit('/').next().unwrap_or(rel);
                    let galley = painter.layout_no_wrap(name.to_string(), FontId::proportional(12.5), pal().text);
                    let w = galley.size().x;
                    painter.galley(Pos2::new(rect.left() + 12.0, mid - galley.size().y * 0.5), galley, pal().text);
                    let counts = format!("{}", errors + warnings);
                    let cg = painter.layout_no_wrap(counts, FontId::proportional(11.5), pal().text_dim);
                    let cx = rect.right() - 12.0 - cg.size().x;
                    painter.galley(Pos2::new(cx, mid - cg.size().y * 0.5), cg, pal().text_dim);
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
                        pal().text_faint,
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
                    let pg = painter.layout_no_wrap(place, FontId::monospace(11.0), pal().text_faint);
                    let px = rect.right() - 12.0 - pg.size().x;
                    painter.galley(Pos2::new(px, mid - pg.size().y * 0.5), pg, pal().text_faint);
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
                        pal().text,
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

pub fn extension_view(ui: &mut egui::Ui, st: &mut UiState, ext: &mut Extensions) {
    let Some(e) = ext.installed.iter().find(|e| e.id == st.extension) else {
        Frame::NONE.inner_margin(Margin::same(12)).show(ui, |ui| {
            ui.label(RichText::new("This extension is not installed any more.").size(12.0).color(pal().text_dim));
        });
        return;
    };
    let mut run = None;
    let mut back = false;
    Frame::NONE.inner_margin(Margin { left: 12, right: 8, top: 10, bottom: 6 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            back = small_icon(ui, icons::ARROW_LEFT, "Back to the installed extensions");
            if let Some(texture) = extension_icon(ui.ctx(), &mut st.ext_icons, e) {
                ui.add(egui::Image::new(&texture).fit_to_exact_size(Vec2::splat(22.0)).corner_radius(4));
            }
            ui.label(RichText::new(&e.display_name).size(14.0).color(pal().text));
        });
        ui.label(RichText::new(format!("{} · {}", e.publisher, e.version)).size(11.5).color(pal().text_faint));
        if !e.description.is_empty() {
            ui.add_space(2.0);
            ui.label(RichText::new(&e.description).size(12.0).color(pal().text_dim));
        }
        if e.status == Status::Failed {
            ui.label(
                RichText::new(format!("It failed to start: {}", e.error))
                    .size(11.5)
                    .color(c(theme::get().problem_error)),
            );
        }
    });
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        Frame::NONE.inner_margin(Margin { left: 10, right: 10, top: 0, bottom: 8 }).show(ui, |ui| {
            if !e.views.is_empty() {
                section(ui, icons::LAYOUT_GRID, "Views");
                well(ui, |ui| {
                    ui.label(RichText::new(e.views.join("  ·  ")).size(12.0).color(pal().text));
                    ui.label(
                        RichText::new("Its pages and lists open from the right rail.")
                            .size(11.5)
                            .color(pal().text_faint),
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
                    ui.painter().rect_filled(rect, 6.0, pal().row);
                }
                let clip = rect.shrink2(Vec2::new(8.0, 0.0));
                ui.painter().with_clip_rect(clip).text(
                    clip.left_center(),
                    Align2::LEFT_CENTER,
                    title,
                    FontId::proportional(12.5),
                    pal().text,
                );
                if resp.on_hover_text(id).clicked() {
                    run = Some((id.clone(), title.clone()));
                }
            }
            if e.commands.is_empty() {
                ui.label(
                    RichText::new("    It has no commands: it works on its own.").size(11.5).color(pal().text_faint),
                );
            }
        });
    });
    if let Some((id, title)) = run {
        ext.run_command(&id, &title);
    }
    if back {
        st.show(Panel::Extensions);
    }
}

pub fn media_panel(ui: &mut egui::Ui, st: &mut UiState, actions: &mut Vec<Action>) {
    let Some(path) = st.media.clone() else {
        Frame::NONE.inner_margin(Margin::same(12)).show(ui, |ui| {
            ui.label(
                RichText::new("Click an image, a video or a PDF to see it here.").size(12.0).color(pal().text_dim),
            );
        });
        return;
    };
    let mut close = false;
    Frame::NONE.inner_margin(Margin { left: 12, right: 6, top: 8, bottom: 6 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                close = small_icon(ui, icons::X, "Close");
                let glyph = crate::codicons::glyph("link-external").unwrap_or(icons::SHARE_2);
                if small_icon(ui, glyph, "Open with the system's program for this file") {
                    actions.push(Action::OpenExternally(path.clone()));
                }
                let source = crate::codicons::glyph("go-to-file").unwrap_or(icons::CODE);
                if crate::markdown::is_markdown(&path) && small_icon(ui, source, "Show the source on the canvas") {
                    actions.push(Action::ShowSource(path.clone()));
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let label = egui::Label::new(RichText::new(name).size(13.5).color(pal().text)).truncate();
                    ui.add(label).on_hover_text(path.display().to_string());
                });
            });
        });
    });
    if close {
        st.close_panel(&Panel::Media);
        return;
    }
    let rect = ui.available_rect_before_wrap().shrink(1.0);
    ui.allocate_rect(rect, Sense::hover());
    if !crate::webview::AVAILABLE {
        let text = "Seeing media inside the app is only available on Windows for now.";
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(12.5), pal().text_dim);
    }
    st.page_places.push((Panel::Media, rect));
}

pub fn webview_panel(ui: &mut egui::Ui, st: &mut UiState, ext: &mut Extensions, view: &str) {
    if let Some(extension) = view.strip_prefix(crate::ext::TABS) {
        tab_strip(ui, ext, extension);
    } else {
        ext.ensure_view(view);
    }
    let Some(entry) = ext.shown(view) else {
        Frame::NONE.inner_margin(Margin::same(12)).show(ui, |ui| {
            ui.label(
                RichText::new("This view is gone: its extension is not running.").size(12.0).color(pal().text_dim),
            );
        });
        return;
    };
    let (key, waiting, failed) = (entry.key.clone(), entry.handle.is_none(), entry.failed);
    let rect = ui.available_rect_before_wrap().shrink(1.0);
    ui.allocate_rect(rect, Sense::hover());
    if !crate::webview::AVAILABLE {
        let text = "Webviews are only available on Windows for now.";
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(12.5), pal().text_dim);
    } else if failed {
        let text = format!("{} did not start. The message is in the log.", entry.title);
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(12.5), pal().text_dim);
    } else if waiting && !ext.host_running {
        let text = "Open a folder to start the extensions.";
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(12.5), pal().text_dim);
    } else if waiting {
        let text = format!("Starting {} …", entry.title);
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(12.5), pal().text_dim);
        ext.resolve_webview(&key);
    }
    st.page_places.push((Panel::Webview(view.to_string()), rect));
}

const TREE_ROW_H: f32 = 22.0;
const TREE_INDENT: f32 = 12.0;

/// A button of a view: its codicon, or its title when it has none.
fn view_button(ui: &mut egui::Ui, item: &MenuItem) -> bool {
    match item.codicon.as_deref().and_then(crate::codicons::glyph) {
        Some(glyph) => small_icon(ui, glyph, &item.title),
        None => {
            let font = FontId::proportional(11.5);
            let galley = ui.painter().layout_no_wrap(item.title.clone(), font, pal().text_dim);
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(galley.size().x + 10.0, 22.0), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(rect, 6.0, pal().hover);
            }
            ui.painter().galley(rect.center() - galley.size() * 0.5, galley, pal().text_dim);
            resp.clicked()
        }
    }
}

/// Buttons laid over a row, from `right` leftwards, without taking room in the list (the
/// rows below do not move when they appear). Gives the one clicked.
fn buttons_over(ui: &egui::Ui, id: egui::Id, items: &[MenuItem], right: f32, mid: f32) -> (f32, Option<MenuItem>) {
    let font = FontId::proportional(11.5);
    let mut x = right;
    let mut clicked = None;
    for (i, item) in items.iter().enumerate().rev() {
        let glyph = item.codicon.as_deref().and_then(crate::codicons::glyph);
        let galley = ui.painter().layout_no_wrap(item.title.clone(), font.clone(), pal().text_dim);
        let w = if glyph.is_some() { 24.0 } else { galley.size().x + 10.0 };
        x -= w;
        let at = Rect::from_min_size(Pos2::new(x, mid - 11.0), Vec2::new(w, 22.0));
        let resp = ui.interact(at, id.with(i), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(at, 6.0, pal().hover);
        }
        let color = if resp.hovered() { pal().text } else { pal().text_dim };
        match glyph {
            Some(glyph) => {
                ui.painter().text(at.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(14.0), color);
            }
            None => ui.painter().galley(at.center() - galley.size() * 0.5, galley, color),
        }
        if resp.on_hover_text(&item.title).clicked() {
            clicked = Some(item.clone());
        }
    }
    (x, clicked)
}

enum TreeAct {
    /// Opens or closes a row; one with a command runs it too, as in VS Code.
    Toggle(String, TreeRow),
    Click(String, TreeRow),
    Button(String, Option<String>, MenuItem),
}

/// The card of a container of tree views: each view a list of rows that open and close,
/// with the buttons the extension gives its title and its rows.
pub fn tree_panel(ui: &mut egui::Ui, st: &mut UiState, ext: &mut Extensions, id: &str) {
    let Some((extension, name, title, container)) =
        ext.tree_container(id).map(|(e, c)| (e.id.clone(), e.display_name.clone(), c.title.clone(), c.clone()))
    else {
        Frame::NONE.inner_margin(Margin::same(12)).show(ui, |ui| {
            ui.label(
                RichText::new("This list is gone: its extension is not installed any more.")
                    .size(12.0)
                    .color(pal().text_dim),
            );
        });
        return;
    };
    let views = ext.tree_views(&container);
    let single = views.len() == 1;
    let mut act = None;
    let title = if title.is_empty() { name.clone() } else { title };
    Frame::NONE.inner_margin(Margin { left: 12, right: 8, top: 8, bottom: 4 }).show(ui, |ui| {
        panel_title(ui, &title, |ui| {
            if let Some(view) = views.first().filter(|_| single) {
                for item in ext.view_buttons(&extension, &view.id, None).iter().rev() {
                    if view_button(ui, item) {
                        act = Some(TreeAct::Button(view.id.clone(), None, item.clone()));
                    }
                }
            }
        });
    });
    if !ext.host_running {
        Frame::NONE.inner_margin(Margin::same(12)).show(ui, |ui| {
            ui.label(RichText::new("Open a folder to start the extensions.").size(12.0).color(pal().text_dim));
        });
        return;
    }
    if views.is_empty() {
        Frame::NONE.inner_margin(Margin::same(12)).show(ui, |ui| {
            ui.label(RichText::new(format!("{name} shows nothing here now.")).size(12.0).color(pal().text_dim));
        });
        return;
    }
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for view in &views {
            if !single {
                let shut = st.tree_shut.contains(&view.id);
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::click());
                let mid = rect.center().y;
                let chevron = if shut { icons::CHEVRON_RIGHT } else { icons::CHEVRON_DOWN };
                let painter = ui.painter();
                painter.text(
                    Pos2::new(rect.left() + 16.0, mid),
                    Align2::CENTER_CENTER,
                    chevron,
                    FontId::proportional(12.0),
                    pal().text_dim,
                );
                let label = view.name.to_uppercase();
                painter.text(
                    Pos2::new(rect.left() + 26.0, mid),
                    Align2::LEFT_CENTER,
                    label,
                    FontId::proportional(11.5),
                    pal().text,
                );
                if resp.clicked() {
                    if shut {
                        st.tree_shut.remove(&view.id);
                    } else {
                        st.tree_shut.insert(view.id.clone());
                    }
                }
                if !shut && ui.rect_contains_pointer(rect) {
                    let buttons = ext.view_buttons(&extension, &view.id, None);
                    let id = egui::Id::unique(("view title", &view.id));
                    if let (_, Some(item)) = buttons_over(ui, id, &buttons, rect.right() - 8.0, mid) {
                        act = Some(TreeAct::Button(view.id.clone(), None, item));
                    }
                }
                if shut {
                    continue;
                }
            }
            let rows = ext.tree_rows(&view.id);
            let (message, missing) =
                ext.tree_state(&view.id).map_or((String::new(), false), |s| (s.message.clone(), s.missing));
            if !message.is_empty() {
                Frame::NONE.inner_margin(Margin { left: 14, right: 10, top: 2, bottom: 6 }).show(ui, |ui| {
                    ui.label(RichText::new(&message).size(12.0).color(pal().text_dim));
                });
            }
            if rows.is_empty() {
                let text =
                    if missing { format!("{name} gives nothing for this view yet.") } else { "Loading…".to_string() };
                Frame::NONE.inner_margin(Margin { left: 14, right: 10, top: 2, bottom: 8 }).show(ui, |ui| {
                    ui.label(RichText::new(text).size(12.0).color(pal().text_faint));
                });
                continue;
            }
            for shown in rows {
                if let Some(a) = tree_row(ui, st, ext, &extension, &view.id, &shown) {
                    act = Some(a);
                }
            }
            ui.add_space(6.0);
        }
    });
    match act {
        Some(TreeAct::Toggle(view, row)) => {
            ext.toggle_tree_row(&view, &row.handle);
            if row.command {
                ext.click_tree_row(&view, &row);
            }
        }
        Some(TreeAct::Click(view, row)) => ext.click_tree_row(&view, &row),
        Some(TreeAct::Button(view, handle, item)) => ext.tree_command(&view, handle.as_deref(), &item),
        None => {}
    }
}

fn tree_row(
    ui: &mut egui::Ui,
    st: &mut UiState,
    ext: &Extensions,
    extension: &str,
    view: &str,
    shown: &ShownRow,
) -> Option<TreeAct> {
    let row = &shown.row;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), TREE_ROW_H), Sense::click());
    if !ui.is_rect_visible(rect) {
        return None;
    }
    let pointed = ui.rect_contains_pointer(rect);
    let mid = rect.center().y;
    if pointed {
        ui.painter().rect_filled(rect.shrink2(Vec2::new(6.0, 0.0)), 6.0, pal().row);
    }
    let x0 = rect.left() + 10.0 + shown.depth as f32 * TREE_INDENT;
    for d in 0..shown.depth {
        let gx = rect.left() + 16.0 + d as f32 * TREE_INDENT;
        ui.painter().vline(gx, rect.y_range(), Stroke::new(1.0, pal().raised));
    }
    if row.collapsible > 0 {
        let chevron = if shown.open { icons::CHEVRON_DOWN } else { icons::CHEVRON_RIGHT };
        ui.painter().text(
            Pos2::new(x0 + 6.0, mid),
            Align2::CENTER_CENTER,
            chevron,
            FontId::proportional(12.0),
            pal().text_dim,
        );
    }
    let mut x = x0 + 16.0;
    let icon_at = Rect::from_center_size(Pos2::new(x + 7.0, mid), Vec2::splat(15.0));
    let base = row.resource.rsplit(['/', '\\']).next().unwrap_or_default();
    if let Some(glyph) = (!row.icon.is_empty()).then(|| crate::codicons::glyph(&row.icon)).flatten() {
        let color = if row.icon_color & 0xff != 0 { c(row.icon_color) } else { pal().text_dim };
        ui.painter().text(icon_at.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(14.0), color);
        x += 20.0;
    } else if !base.is_empty() && st.file_icons.paint(ui, base, row.collapsible > 0, shown.open, icon_at, false) {
        x += 20.0;
    }
    let buttons = if pointed { ext.view_buttons(extension, view, Some(&row.context)) } else { Vec::new() };
    let id = egui::Id::unique(("tree row", view, &row.handle));
    let (right, clicked) = buttons_over(ui, id, &buttons, rect.right() - 8.0, mid);
    let mut act = clicked.map(|item| TreeAct::Button(view.to_string(), Some(row.handle.clone()), item));
    let font = FontId::proportional(13.0);
    let used = super::text_fit(ui.painter(), Pos2::new(x, mid), &row.label, font, pal().text, right - x);
    if !row.description.is_empty() && right - x - used > 30.0 {
        let at = Pos2::new(x + used + 8.0, mid);
        super::text_fit(ui.painter(), at, &row.description, FontId::proportional(11.5), pal().text_faint, right - at.x);
    }
    let tip = if row.tooltip.is_empty() { None } else { Some(row.tooltip.clone()) };
    let resp = match tip {
        Some(tip) => resp.on_hover_text(tip),
        None => resp,
    };
    if act.is_none() && resp.clicked() {
        act = Some(if row.collapsible > 0 {
            TreeAct::Toggle(view.to_string(), row.clone())
        } else {
            TreeAct::Click(view.to_string(), row.clone())
        });
    }
    act
}

fn tab_strip(ui: &mut egui::Ui, ext: &mut Extensions, extension: &str) {
    let front = ext.shown(&format!("{}{extension}", crate::ext::TABS)).map(|w| w.key.clone());
    let tabs: Vec<(String, String)> = ext.tabs_of(extension).map(|w| (w.key.clone(), w.title.clone())).collect();
    let (mut show, mut close) = (None, None);
    Frame::NONE.inner_margin(Margin { left: 6, right: 6, top: 6, bottom: 4 }).show(ui, |ui| {
        egui::ScrollArea::horizontal().auto_shrink([false, true]).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (key, title) in &tabs {
                    let active = front.as_ref() == Some(key);
                    let font = FontId::proportional(12.0);
                    let color = if active { pal().text } else { pal().text_dim };
                    let short: String = if title.chars().count() > 26 {
                        title.chars().take(25).chain(std::iter::once('…')).collect()
                    } else {
                        title.clone()
                    };
                    let galley = ui.painter().layout_no_wrap(short, font, color);
                    let size = Vec2::new(galley.size().x + 38.0, 26.0);
                    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
                    let hovered = ui.rect_contains_pointer(rect);
                    let fill = match (active, hovered) {
                        (true, _) => pal().raised,
                        (false, true) => pal().row,
                        (false, false) => pal().well,
                    };
                    ui.painter().rect_filled(rect, 6.0, fill);
                    let mid = rect.center().y;
                    ui.painter().galley(Pos2::new(rect.left() + 10.0, mid - galley.size().y * 0.5), galley, color);
                    let cross = Rect::from_center_size(Pos2::new(rect.right() - 13.0, mid), Vec2::splat(18.0));
                    let closing = ui.interact(cross, ui.scope_id().with(("close tab", key)), Sense::click());
                    if closing.hovered() {
                        ui.painter().rect_filled(cross, 4.0, pal().hover);
                    }
                    if active || hovered {
                        let tint = if closing.hovered() { pal().text } else { pal().text_dim };
                        ui.painter().text(
                            cross.center(),
                            Align2::CENTER_CENTER,
                            icons::X,
                            FontId::proportional(12.0),
                            tint,
                        );
                    }
                    if closing.on_hover_text("Close").clicked() {
                        close = Some(key.clone());
                    } else if resp.on_hover_text(title).clicked() {
                        show = Some(key.clone());
                    }
                }
            });
        });
    });
    if let Some(key) = show {
        ext.show_tab(extension, &key);
    }
    if let Some(key) = close {
        ext.close_tab(&key);
    }
}

pub fn notices(ctx: &egui::Context, st: &mut UiState, canvas: Rect, ext: &mut Extensions) {
    if ext.notices.is_empty() {
        return;
    }
    let mut answer: Option<(usize, Option<String>)> = None;
    let resp = egui::Area::new(egui::Id::unique("notices"))
        .pivot(Align2::RIGHT_BOTTOM)
        .fixed_pos(canvas.right_bottom() - Vec2::new(12.0, 12.0))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            for (i, n) in ext.notices.iter().enumerate() {
                let color = match n.level {
                    Level::Info => pal().accent,
                    Level::Warn => c(theme::get().problem_warning),
                    Level::Error => c(theme::get().problem_error),
                };
                Frame::NONE
                    .fill(pal().card)
                    .stroke(Stroke::new(1.0, pal().border))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::symmetric(10, 7))
                    .show(ui, |ui| {
                        ui.set_max_width(420.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.label(icon(icons::PUZZLE, 13.0, color));
                            ui.label(RichText::new(&n.source).size(12.0).color(pal().text));
                            let limit = if n.actions.is_empty() { 300 } else { 900 };
                            let text: String = n.text.chars().take(limit).collect();
                            ui.label(RichText::new(text).size(12.0).color(pal().text_dim));
                        });
                        if !n.actions.is_empty() {
                            ui.add_space(2.0);
                            ui.horizontal(|ui| {
                                for (k, action) in n.actions.iter().enumerate() {
                                    if pill(ui, action, k == 0) {
                                        answer = Some((i, Some(action.clone())));
                                    }
                                }
                                if pill(ui, "Dismiss", false) {
                                    answer = Some((i, None));
                                }
                            });
                        }
                    });
            }
        });
    if ext.notices.iter().any(|n| !n.actions.is_empty()) {
        st.floating.push(resp.response.rect);
    }
    if let Some((i, action)) = answer {
        ext.answer_notice(i, action.as_deref());
    }
}

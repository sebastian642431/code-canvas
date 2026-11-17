//! Panels for extensions: the registry and what is installed, the problems
//! extensions report, and their messages.

use canvas_core::Site;
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Frame, Layout, Margin, Pos2, Rect, RichText, Sense, Stroke, Vec2,
};

use super::{c, icon, pal, panel_title, search_field, section, small_icon, well, Action, LeftPanel, UiState, ROW_H};
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

/// A small rounded button with a label.
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

/// The square an extension's icon goes in (a placeholder when it has none).
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
                // Right of the name, like VS Code: how long it took to start, or what is wrong.
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
                    st.show_left(LeftPanel::Extension);
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
                        RichText::new("Its pages open from the right rail. Lists (tree views) cannot be shown yet.")
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
        st.show_left(LeftPanel::Extensions);
    }
}

/// The media viewer: a header with the file's name, and under it the place
/// where the app puts the browser view that shows the file.
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
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let label = egui::Label::new(RichText::new(name).size(13.5).color(pal().text)).truncate();
                    ui.add(label).on_hover_text(path.display().to_string());
                });
            });
        });
    });
    if close {
        st.right = None;
        return;
    }
    let rect = ui.available_rect_before_wrap().shrink(1.0);
    ui.allocate_rect(rect, Sense::hover());
    if !crate::webview::AVAILABLE {
        let text = "Seeing media inside the app is only available on Windows for now.";
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(12.5), pal().text_dim);
    }
    st.webview_rect = Some(rect);
}

/// The panel an extension's webview is shown in. The page itself is a
/// browser view the app places over this panel; here its place is reserved
/// and the extension is asked to fill it the first time.
pub fn webview_panel(ui: &mut egui::Ui, st: &mut UiState, ext: &mut Extensions) {
    // A view of a manifest whose extension has not started has no entry yet.
    ext.ensure_view(&st.webview);
    let Some(entry) = ext.webviews.iter().find(|w| w.key == st.webview) else {
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
    st.webview_rect = Some(rect);
}

/// Messages from extensions, stacked in the canvas corner. Plain ones fade
/// by themselves; those that ask something show their choices as buttons.
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
    // Only questions take clicks; plain notices let them through to the canvas.
    if ext.notices.iter().any(|n| !n.actions.is_empty()) {
        st.floating.push(resp.response.rect);
    }
    if let Some((i, action)) = answer {
        ext.answer_notice(i, action.as_deref());
    }
}

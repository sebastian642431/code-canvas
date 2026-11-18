use egui::{Color32, FontId, Frame, Margin, Pos2, Rect, RichText, Sense, Stroke, Vec2};

use super::{ask_button, pal, panel_title, small_icon, Action, ChoiceLook, UiState};
use crate::icons;
use crate::term::{self, Mouse, Selection, Spot, Terminal, Unit};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsoleOp {
    Start,
    Install,
    Page,
}

const SIXTEEN: [u32; 16] = [
    0x26252a, 0xf0452f, 0x22e39a, 0xe5c07b, 0x6f9df2, 0xc58af9, 0x4fd6e0, 0xc9c8cd, 0x66656b, 0xff6b57, 0x5af0b4,
    0xf2d49b, 0x94b7f7, 0xd8aefb, 0x86e6ee, 0xffffff,
];

const SIXTEEN_ON_LIGHT: [u32; 16] = [
    0x3b3a40, 0xc4262e, 0x1f7a33, 0x8a6100, 0x1f5fc0, 0x8a3ac8, 0x0b7a8a, 0x6b6a70, 0x5c5b60, 0xd03a3a, 0x2a8a45,
    0x946200, 0x2864d8, 0x9a4ad8, 0x0b8a9a, 0x2b2a30,
];

fn shade(color: vt100::Color, plain: Color32, sixteen: &[u32; 16]) -> Color32 {
    match color {
        vt100::Color::Default => plain,
        vt100::Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
        vt100::Color::Idx(i) if i < 16 => {
            let v = sixteen[i as usize];
            Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
        }
        vt100::Color::Idx(i) if i < 232 => {
            let level = |n: u8| if n == 0 { 0 } else { 55 + n * 40 };
            let i = i - 16;
            Color32::from_rgb(level(i / 36), level(i / 6 % 6), level(i % 6))
        }
        vt100::Color::Idx(i) => {
            let v = 8 + (i - 232) * 10;
            Color32::from_rgb(v, v, v)
        }
    }
}

/// What pasting `text` sends: lines end with Enter, and the whole is marked as pasted when the shell asked for that.
fn pasted(text: &str, screen: &vt100::Screen) -> Vec<u8> {
    let text = text.replace("\r\n", "\r").replace('\n', "\r");
    match screen.bracketed_paste() {
        true => format!("\x1b[200~{text}\x1b[201~").into_bytes(),
        false => text.into_bytes(),
    }
}

fn typed(event: &egui::Event, screen: &vt100::Screen) -> Vec<u8> {
    use egui::{Event, Key};
    match event {
        Event::Text(text) => text.as_bytes().to_vec(),
        Event::Paste(text) => pasted(text, screen),
        Event::Copy => vec![3],
        Event::Cut => vec![24],
        Event::Key { key, pressed: true, modifiers, .. } => {
            let arrow = |letter: char| match (modifiers.ctrl, screen.application_cursor()) {
                (true, _) => format!("\x1b[1;5{letter}"),
                (false, true) => format!("\x1bO{letter}"),
                (false, false) => format!("\x1b[{letter}"),
            };
            let said: String = match key {
                Key::Enter => "\r".into(),
                Key::Tab if modifiers.shift => "\x1b[Z".into(),
                Key::Tab => "\t".into(),
                Key::Backspace if modifiers.ctrl => "\x17".into(),
                Key::Backspace => "\x7f".into(),
                Key::Escape => "\x1b".into(),
                Key::ArrowUp => arrow('A'),
                Key::ArrowDown => arrow('B'),
                Key::ArrowRight => arrow('C'),
                Key::ArrowLeft => arrow('D'),
                Key::Home => "\x1b[H".into(),
                Key::End => "\x1b[F".into(),
                Key::PageUp => "\x1b[5~".into(),
                Key::PageDown => "\x1b[6~".into(),
                Key::Delete => "\x1b[3~".into(),
                Key::Insert => "\x1b[2~".into(),
                Key::Space if modifiers.ctrl => "\0".into(),
                key if modifiers.ctrl && !modifiers.alt && !modifiers.shift => match key.name().as_bytes() {
                    [letter @ b'A'..=b'Z'] => return vec![letter - b'A' + 1],
                    _ => String::new(),
                },
                _ => String::new(),
            };
            said.into_bytes()
        }
        _ => Vec::new(),
    }
}

/// One row of the screen as a laid out line; the cells of `chosen` (columns, start to end) on the selection's colour.
fn row_of(
    screen: &vt100::Screen,
    row: u16,
    cols: u16,
    font: &FontId,
    chosen: std::ops::Range<u16>,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let (text, ground) = (pal().text, Color32::TRANSPARENT);
    let sixteen = if canvas_render::theme::get().is_light() { &SIXTEEN_ON_LIGHT } else { &SIXTEEN };
    for col in 0..cols {
        let Some(cell) = screen.cell(row, col) else { break };
        if cell.is_wide_continuation() {
            continue;
        }
        let (mut front, mut back) = (shade(cell.fgcolor(), text, sixteen), shade(cell.bgcolor(), ground, sixteen));
        if cell.inverse() {
            (front, back) = (if back == ground { pal().well } else { back }, front);
        }
        if chosen.contains(&col) {
            (front, back) = (text, pal().selection);
        }
        if cell.dim() {
            front = front.gamma_multiply(0.6);
        }
        let format = egui::TextFormat {
            font_id: font.clone(),
            color: front,
            background: back,
            italics: cell.italic(),
            underline: if cell.underline() { Stroke::new(1.0, front) } else { Stroke::NONE },
            ..Default::default()
        };
        job.append(if cell.has_contents() { cell.contents() } else { " " }, 0.0, format);
    }
    job
}

/// Copies the selection and lets it go; false when nothing is selected.
fn copy_selection(term: &mut Terminal) -> bool {
    let text = term.selected_text();
    term.selection = None;
    if text.is_empty() {
        return false;
    }
    term::copy(&text);
    log::info!("console: copied {} characters", text.chars().count());
    true
}

/// The mouse on the screen: a press starts a selection (two in a row a word, three a line, Shift grows the one there
/// is), dragging grows it and scrolls when it goes past the top or the bottom, a click alone lets it go. A right
/// click copies the selection like Windows Terminal, or pastes when nothing is selected.
fn pointer(ui: &egui::Ui, resp: &egui::Response, term: &mut Terminal, mouse: &mut Mouse, inner: Rect, cell: Vec2) {
    let (rows, cols) = term.size();
    let spot_at = |term: &Terminal, pos: Pos2, unit: Unit| {
        let row = ((pos.y - inner.top()) / cell.y).floor().clamp(0.0, f32::from(rows) - 1.0) as usize;
        let x = (pos.x - inner.left()) / cell.x;
        let col = match unit {
            Unit::Cell => x.round().clamp(0.0, f32::from(cols)),
            _ => x.floor().clamp(0.0, f32::from(cols) - 1.0),
        };
        Spot::new(term.top() + row, col as u16)
    };
    let (pressed, down, at, time, shift) = ui.input(|input| {
        let pointer = &input.pointer;
        (pointer.primary_pressed(), pointer.primary_down(), pointer.latest_pos(), input.time, input.modifiers.shift)
    });
    if let (true, true, Some(pos)) = (pressed, resp.contains_pointer(), at) {
        let on = spot_at(term, pos, Unit::Word);
        let again = time - mouse.last < 0.45 && mouse.at == Some(on);
        mouse.clicks = if again { mouse.clicks % 3 + 1 } else { 1 };
        (mouse.last, mouse.at, mouse.dragging) = (time, Some(on), true);
        let unit = [Unit::Cell, Unit::Word, Unit::Line][usize::from(mouse.clicks - 1)];
        term.selection = match term.selection {
            Some(mut selection) if shift && mouse.clicks == 1 => {
                selection.head = spot_at(term, pos, selection.unit);
                Some(selection)
            }
            _ => Some(Selection::new(spot_at(term, pos, unit), unit)),
        };
        resp.request_focus();
    }
    if mouse.dragging {
        if let Some(pos) = at {
            let past = match pos.y {
                y if y < inner.top() => 1,
                y if y > inner.bottom() => -1,
                _ => 0,
            };
            if down && past != 0 {
                term.scroll(past);
                ui.ctx().request_repaint();
            }
            if let Some(unit) = term.selection.map(|selection| selection.unit) {
                let head = spot_at(term, pos, unit);
                if let Some(selection) = term.selection.as_mut() {
                    selection.head = head;
                }
            }
        }
        if !down {
            mouse.dragging = false;
            if term.selected().is_none() {
                term.selection = None;
            }
        }
    }
    if resp.secondary_clicked() {
        resp.request_focus();
        if !copy_selection(term) {
            if let Some(text) = term::clipboard().filter(|text| !text.is_empty()) {
                let bytes = pasted(&text, term.screen());
                term.send(&bytes);
            }
        }
    }
}

fn screen(ui: &mut egui::Ui, term: &mut Terminal, focus: &mut bool, wheel: &mut f32, mouse: &mut Mouse) {
    let font = FontId::monospace(13.0);
    let (wide, tall) = ui.fonts_mut(|fonts| (fonts.glyph_width(&font, 'M'), fonts.row_height(&font)));
    let all = ui.available_rect_before_wrap().shrink2(Vec2::new(8.0, 6.0));
    ui.allocate_rect(all, Sense::hover());
    // An id of its own: the keyboard stays with the screen whatever is drawn before it from one frame to the next.
    let resp = ui.interact(all, ui.scope_id().with("console screen"), Sense::click_and_drag());
    ui.painter().rect_filled(all, 8.0, pal().well);
    let inner = all.shrink2(Vec2::new(8.0, 6.0));
    let cols = (inner.width() / wide.max(1.0)).floor().clamp(8.0, 400.0) as u16;
    let rows = (inner.height() / tall.max(1.0)).floor().clamp(2.0, 300.0) as u16;
    term.resize(rows, cols);
    mouse.place = Some((inner.min, Vec2::new(wide, tall)));
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
    }

    if resp.clicked() || std::mem::take(focus) {
        resp.request_focus();
    }
    if resp.clicked_elsewhere() {
        resp.surrender_focus();
    }
    pointer(ui, &resp, term, mouse, inner, Vec2::new(wide, tall));
    // Keys only come while the window has the keyboard: what counts for taking them is that the screen has the focus
    // of the interface (a script's keys come to a window in the background too); the caret shows both.
    let focused = resp.has_focus();
    if ui.memory(|memory| memory.has_focus(resp.id)) {
        let keep = egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true };
        ui.memory_mut(|memory| memory.set_focus_lock_filter(resp.id, keep));
        let events = ui.input(|input| input.events.clone());
        for event in &events {
            use egui::{Event, Key};
            let pages = match event {
                Event::Key { key: Key::PageUp, pressed: true, modifiers, .. } if modifiers.shift => 1,
                Event::Key { key: Key::PageDown, pressed: true, modifiers, .. } if modifiers.shift => -1,
                _ => 0,
            };
            if pages != 0 {
                term.scroll(pages * (rows as isize - 1));
                continue;
            }
            // Ctrl+C copies what is selected and is the interrupt when nothing is; Ctrl+Shift+C only copies, and
            // Ctrl+Shift+V pastes (Ctrl+V comes as a paste already). Escape lets a selection go.
            let shifted = |want: Key| {
                matches!(event, Event::Key { key, pressed: true, modifiers, .. }
                    if *key == want && modifiers.ctrl && modifiers.shift && !modifiers.alt)
            };
            let escape = matches!(event, Event::Key { key: Key::Escape, pressed: true, .. });
            if matches!(event, Event::Copy) || shifted(Key::C) || (escape && term.selection.is_some()) {
                let had = match escape {
                    true => std::mem::take(&mut term.selection).is_some(),
                    false => copy_selection(term),
                };
                if had || !matches!(event, Event::Copy) {
                    continue;
                }
            }
            if shifted(Key::V) {
                if let Some(text) = term::clipboard() {
                    let bytes = pasted(&text, term.screen());
                    term.send(&bytes);
                }
                continue;
            }
            let bytes = typed(event, term.screen());
            if !bytes.is_empty() {
                term.selection = None;
            }
            term.send(&bytes);
        }
    }
    if resp.hovered() {
        *wheel += ui.input(|input| input.smooth_scroll_delta.y) / tall.max(1.0);
        let lines = wheel.trunc();
        if lines != 0.0 {
            *wheel -= lines;
            term.scroll(lines as isize);
        }
    }

    let painter = ui.painter().with_clip_rect(all);
    let (chosen, top) = (term.selected(), term.top());
    let shown = term.screen();
    for row in 0..rows {
        let line = top + usize::from(row);
        let span = match chosen {
            Some((from, to)) if (from.line..=to.line).contains(&line) => {
                let start = if line == from.line { from.col } else { 0 };
                start..if line == to.line { to.col } else { cols }
            }
            _ => 0..0,
        };
        let job = row_of(shown, row, cols, &font, span);
        let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
        painter.galley(Pos2::new(inner.left(), inner.top() + row as f32 * tall), galley, pal().text);
    }
    if !shown.hide_cursor() && shown.scrollback() == 0 && !term.ended {
        let (row, col) = shown.cursor_position();
        let at = Pos2::new(inner.left() + col as f32 * wide, inner.top() + row as f32 * tall);
        let caret = Rect::from_min_size(at, Vec2::new(wide, tall));
        if focused {
            painter.rect_filled(caret, 1.0, pal().text.gamma_multiply(0.55));
        } else {
            painter.rect_stroke(caret, 1.0, Stroke::new(1.0, pal().text_dim), egui::StrokeKind::Inside);
        }
    }
    if shown.scrollback() > 0 {
        let note = format!("{} lines up  ·  type to come back", shown.scrollback());
        let at = Pos2::new(all.right() - 10.0, all.top() + 6.0);
        painter.text(at, egui::Align2::RIGHT_TOP, note, FontId::proportional(11.0), pal().text_faint);
    }
}

fn says(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).size(12.5).color(pal().text_dim)).wrap());
}

pub fn console_panel(ui: &mut egui::Ui, st: &mut UiState, actions: &mut Vec<Action>) {
    Frame::NONE.inner_margin(Margin { left: 12, right: 8, top: 8, bottom: 2 }).show(ui, |ui| {
        panel_title(ui, "Console", |ui| {
            if small_icon(ui, icons::REFRESH_CW, "End this shell and start a new one in the folder") {
                actions.push(Action::Console(ConsoleOp::Start));
            }
        });
    });
    let console = &mut st.console;
    if !console.looked {
        console.look();
    }
    if console.shell.is_none() {
        Frame::NONE.inner_margin(Margin::same(12)).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.label(RichText::new("bash is not on this computer").size(14.0).color(pal().text));
            match (cfg!(windows), console.installing()) {
                (true, true) => {
                    says(ui, "Installing Git for Windows, which brings bash. Windows may ask for permission.")
                }
                (true, false) => {
                    says(ui, "The console runs bash. On Windows it comes with Git for Windows.");
                    if ask_button(ui, "Install Git for Windows", ChoiceLook::Main) {
                        actions.push(Action::Console(ConsoleOp::Install));
                    }
                    if ask_button(ui, "Open the download page", ChoiceLook::Plain) {
                        actions.push(Action::Console(ConsoleOp::Page));
                    }
                }
                (false, _) => says(ui, "The console runs bash: install it with this system's package manager."),
            }
            if let Some(why) = &console.failed {
                says(ui, why);
            }
            if !console.installing() && ask_button(ui, "Look again", ChoiceLook::Plain) {
                console.look();
            }
        });
        return;
    }
    if let Some(why) = console.failed.clone() {
        Frame::NONE.inner_margin(Margin::same(12)).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            says(ui, &format!("The shell did not start: {why}"));
            if ask_button(ui, "Try again", ChoiceLook::Main) {
                actions.push(Action::Console(ConsoleOp::Start));
            }
        });
        return;
    }
    let Some(term) = console.term.as_mut() else {
        actions.push(Action::Console(ConsoleOp::Start));
        return;
    };
    if term.ended {
        Frame::NONE.inner_margin(Margin { left: 12, right: 12, top: 4, bottom: 4 }).show(ui, |ui| {
            ui.horizontal(|ui| {
                says(ui, "The shell ended.");
                if ask_button(ui, "Start a new one", ChoiceLook::Plain) {
                    actions.push(Action::Console(ConsoleOp::Start));
                }
            });
        });
    }
    screen(ui, term, &mut console.focus, &mut console.wheel, &mut console.mouse);
}

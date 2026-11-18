use egui::{Color32, FontId, Frame, Margin, Pos2, Rect, RichText, Sense, Stroke, Vec2};

use super::{ask_button, pal, panel_title, small_icon, Action, ChoiceLook, UiState};
use crate::icons;
use crate::term::Terminal;

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

fn shade(color: vt100::Color, plain: Color32) -> Color32 {
    match color {
        vt100::Color::Default => plain,
        vt100::Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
        vt100::Color::Idx(i) if i < 16 => {
            let v = SIXTEEN[i as usize];
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

fn typed(event: &egui::Event, screen: &vt100::Screen) -> Vec<u8> {
    use egui::{Event, Key};
    match event {
        Event::Text(text) => text.as_bytes().to_vec(),
        Event::Paste(text) => {
            let text = text.replace("\r\n", "\r").replace('\n', "\r");
            match screen.bracketed_paste() {
                true => format!("\x1b[200~{text}\x1b[201~").into_bytes(),
                false => text.into_bytes(),
            }
        }
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

fn row_of(screen: &vt100::Screen, row: u16, cols: u16, font: &FontId) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let (text, ground) = (pal().text, Color32::TRANSPARENT);
    for col in 0..cols {
        let Some(cell) = screen.cell(row, col) else { break };
        if cell.is_wide_continuation() {
            continue;
        }
        let (mut front, mut back) = (shade(cell.fgcolor(), text), shade(cell.bgcolor(), ground));
        if cell.inverse() {
            (front, back) = (if back == ground { pal().well } else { back }, front);
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

fn screen(ui: &mut egui::Ui, term: &mut Terminal, focus: &mut bool, wheel: &mut f32) {
    let font = FontId::monospace(13.0);
    let (wide, tall) = ui.fonts_mut(|fonts| (fonts.glyph_width(&font, 'M'), fonts.row_height(&font)));
    let all = ui.available_rect_before_wrap().shrink2(Vec2::new(8.0, 6.0));
    let resp = ui.allocate_rect(all, Sense::click());
    ui.painter().rect_filled(all, 8.0, pal().well);
    let inner = all.shrink2(Vec2::new(8.0, 6.0));
    let cols = (inner.width() / wide.max(1.0)).floor().clamp(8.0, 400.0) as u16;
    let rows = (inner.height() / tall.max(1.0)).floor().clamp(2.0, 300.0) as u16;
    term.resize(rows, cols);

    if resp.clicked() || std::mem::take(focus) {
        resp.request_focus();
    }
    if resp.clicked_elsewhere() {
        resp.surrender_focus();
    }
    let focused = resp.has_focus();
    if focused {
        let keep = egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true };
        ui.memory_mut(|memory| memory.set_focus_lock_filter(resp.id, keep));
        let events = ui.input(|input| input.events.clone());
        for event in &events {
            let pages = match event {
                egui::Event::Key { key: egui::Key::PageUp, pressed: true, modifiers, .. } if modifiers.shift => 1,
                egui::Event::Key { key: egui::Key::PageDown, pressed: true, modifiers, .. } if modifiers.shift => -1,
                _ => 0,
            };
            if pages != 0 {
                term.scroll(pages * (rows as isize - 1));
                continue;
            }
            let bytes = typed(event, term.screen());
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
    let shown = term.screen();
    for row in 0..rows {
        let job = row_of(shown, row, cols, &font);
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
    screen(ui, term, &mut console.focus, &mut console.wheel);
}

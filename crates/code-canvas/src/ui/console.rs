use egui::{Align2, Color32, FontId, Frame, Margin, Pos2, Rect, RichText, Sense, Stroke, Vec2};

use super::{
    ask_button, ext_panels, menu_gap, menu_row, menu_row_key, paint_look, pal, text_fit, Action, ChoiceLook, Panel,
    RailLook, Side, UiState,
};
use crate::consoles::{Menu, MenuKind, MenuPage, MAIN};
use crate::ext::Extensions;
use crate::icons;
use crate::keys::Command;
use crate::term::{self, Mouse, Selection, Spot, Terminal, Unit};

/// What is done to the consoles. `None` for a console is the one the keyboard goes to.
#[derive(Clone, Debug, PartialEq)]
pub enum ConsoleOp {
    Install,
    Page,
    /// A new console in a card (that of the current one when `None`), of a profile, in a folder.
    New {
        card: Option<u64>,
        profile: Option<String>,
        dir: Option<std::path::PathBuf>,
    },
    Split {
        tab: Option<u64>,
        profile: Option<String>,
    },
    Start(u64),
    /// Ends a console, after asking when something runs in it.
    Kill(Option<u64>),
    End(u64),
    Unsplit(Option<u64>),
    ToCard(Option<u64>),
    Front(u64),
    Cycle(i32),
    Renaming(Option<u64>),
    Rename(u64, Option<String>),
    Colour(u64, Option<u8>),
    Icon(u64, Option<String>),
    Clear(Option<u64>),
    ToCommand(bool),
    RunSelection,
    RunFile,
    /// Types a line into the current console; Enter too when `run`.
    Type {
        text: String,
        run: bool,
    },
    Cd(std::path::PathBuf),
    Default(String),
    Fill(Option<u64>),
    Profiles,
}

const COLOURS: [u8; 6] = [1, 2, 3, 4, 5, 6];
const ICONS: [&str; 14] = [
    "terminal",
    "server",
    "globe",
    "flame",
    "rocket",
    "beaker",
    "bug",
    "database",
    "zap",
    "play",
    "package",
    "cloud",
    "heart",
    "star-full",
];

pub fn palette() -> Vec<(String, Action)> {
    let op = |title: &str, op: ConsoleOp| (format!("Terminal: {title}"), Action::Console(op));
    vec![
        op("New Console", ConsoleOp::New { card: None, profile: None, dir: None }),
        op("Split Console", ConsoleOp::Split { tab: None, profile: None }),
        op("Next Console", ConsoleOp::Cycle(1)),
        op("Previous Console", ConsoleOp::Cycle(-1)),
        op("Kill the Active Console", ConsoleOp::Kill(None)),
        op("Rename the Active Console", ConsoleOp::Renaming(None)),
        op("Unsplit the Active Console", ConsoleOp::Unsplit(None)),
        op("Move the Active Console to a New Card", ConsoleOp::ToCard(None)),
        op("Clear", ConsoleOp::Clear(None)),
        op("Run Selected Text in the Console", ConsoleOp::RunSelection),
        op("Run Active File in the Console", ConsoleOp::RunFile),
        op("Scroll to Previous Command", ConsoleOp::ToCommand(true)),
        op("Scroll to Next Command", ConsoleOp::ToCommand(false)),
        op("Select Default Profile", ConsoleOp::Profiles),
        op("Maximize or Restore the Console", ConsoleOp::Fill(None)),
    ]
}

/// The console card fills the canvas.
pub fn fills(st: &UiState, panel: &Panel) -> bool {
    match panel {
        Panel::Console => st.console.filled == Some(MAIN),
        Panel::Consoles(card) => st.console.filled == Some(*card),
        _ => false,
    }
}

fn codicon(name: &str) -> &'static str {
    crate::codicons::glyph(name).unwrap_or(icons::CODE)
}

fn red() -> Color32 {
    super::c(canvas_render::theme::get().git_removed)
}

fn tint(color: Option<u8>) -> Color32 {
    let sixteen = if canvas_render::theme::get().is_light() { &SIXTEEN_ON_LIGHT } else { &SIXTEEN };
    color.map_or(pal().text_dim, |n| shade(vt100::Color::Idx(n), pal().text_dim, sixteen))
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

#[allow(clippy::too_many_arguments)]
fn screen(
    ui: &mut egui::Ui,
    id: u64,
    all: Rect,
    term: &mut Terminal,
    focus: &mut bool,
    wheel: &mut f32,
    mouse: &mut Mouse,
    ring: bool,
) -> bool {
    let font = FontId::monospace(13.0);
    let (wide, tall) = ui.fonts_mut(|fonts| (fonts.glyph_width(&font, 'M'), fonts.row_height(&font)));
    // An id of its own: the keyboard stays with the screen whatever is drawn before it from one frame to the next.
    let resp = ui.interact(all, egui::Id::unique(("console screen", id)), Sense::click_and_drag());
    ui.painter().rect_filled(all, 8.0, pal().well);
    if ring {
        let stroke = Stroke::new(1.0, pal().accent.gamma_multiply(0.75));
        ui.painter().rect_stroke(all, 8.0, stroke, egui::StrokeKind::Inside);
    }
    let gutter = if term.shell().integrated { 10.0 } else { 0.0 };
    let inner = Rect::from_min_max(all.min + Vec2::new(8.0 + gutter, 6.0), all.max - Vec2::new(8.0, 6.0));
    let cols = (inner.width() / wide.max(1.0)).floor().clamp(8.0, 400.0) as u16;
    let rows = (inner.height() / tall.max(1.0)).floor().clamp(2.0, 300.0) as u16;
    term.resize(rows, cols);
    mouse.place = Some((inner.min, Vec2::new(wide, tall)));
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Text);
    }
    let picked = resp.contains_pointer() && ui.input(|input| input.pointer.any_pressed());

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
            let with_ctrl = |want: Key| {
                matches!(event, Event::Key { key, modifiers, .. }
                    if *key == want && modifiers.ctrl && !modifiers.shift && !modifiers.alt)
            };
            if with_ctrl(Key::PageUp) || with_ctrl(Key::PageDown) {
                continue;
            }
            let pressed = matches!(event, Event::Key { pressed: true, .. });
            if term.shell().integrated && (with_ctrl(Key::ArrowUp) || with_ctrl(Key::ArrowDown)) {
                if pressed {
                    term.scroll_to_command(with_ctrl(Key::ArrowUp));
                }
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
    if gutter > 0.0 && !term.screen().alternate_screen() {
        for mark in term.shell().marks.iter().filter(|mark| (top..top + usize::from(rows)).contains(&mark.line)) {
            let row = (mark.line - top) as f32;
            let at = Pos2::new(inner.left() - 7.0, inner.top() + (row + 0.5) * tall);
            match (mark.done, mark.failed(), mark.exit) {
                (true, true, _) => painter.circle_filled(at, 3.5, red()),
                (true, false, Some(0)) => painter.circle_filled(at, 3.0, pal().text_faint),
                _ => painter.circle_stroke(at, 3.0, Stroke::new(1.0, pal().text_faint)),
            };
        }
    }
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
    picked
}

fn says(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).size(12.5).color(pal().text_dim)).wrap());
}

fn no_shell(ui: &mut egui::Ui, st: &mut UiState, actions: &mut Vec<Action>) {
    let console = &mut st.console;
    Frame::NONE.inner_margin(Margin::same(12)).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 8.0;
        ui.label(RichText::new("No shell was found on this computer").size(14.0).color(pal().text));
        match (cfg!(windows), console.installing()) {
            (true, true) => says(ui, "Installing Git for Windows, which brings bash. Windows may ask for permission."),
            (true, false) => {
                says(ui, "The console runs bash, PowerShell or the Command Prompt. Git for Windows brings bash.");
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
}

/// A small button of the header; its place, to open a menu under it.
fn head_button(ui: &mut egui::Ui, glyph: &str, tip: &str, width: f32, on: bool) -> (bool, Rect) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 24.0), Sense::click());
    if resp.hovered() || on {
        ui.painter().rect_filled(rect, 6.0, pal().hover);
    }
    let color = if resp.hovered() || on { pal().text } else { pal().text_dim };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(14.0), color);
    (resp.on_hover_text(tip).clicked(), rect)
}

fn open_menu(st: &mut UiState, card: u64, kind: MenuKind, page: MenuPage, at: Pos2) {
    let again = st.console.menu.as_ref().is_some_and(|menu| menu.kind == kind && menu.card == card);
    st.console.menu = (!again).then_some(Menu { card, kind, page, at, place: None });
}

fn rename_field(ui: &mut egui::Ui, st: &mut UiState, actions: &mut Vec<Action>, id: u64, width: f32) {
    let first = !std::mem::replace(&mut st.console.rename_focused, true);
    let Some((_, text)) = st.console.renaming.as_mut() else { return };
    let edit = egui::TextEdit::singleline(text).desired_width(width).font(FontId::proportional(12.5));
    let resp = ui.add(edit);
    if first {
        resp.request_focus();
    }
    if resp.lost_focus() || (!first && !resp.has_focus()) {
        let escaped = ui.input(|input| input.key_pressed(egui::Key::Escape));
        if let (false, Some((_, name))) = (escaped, st.console.renaming.clone()) {
            actions.push(Action::Console(ConsoleOp::Rename(id, Some(name))));
        }
        st.console.renaming = None;
    }
}

fn title_tab(
    ui: &mut egui::Ui,
    st: &mut UiState,
    ext: &Extensions,
    look: &RailLook,
    title: &str,
    on: bool,
) -> egui::Response {
    let font = FontId::proportional(13.0);
    let text = ui.painter().layout_no_wrap(title.to_string(), font.clone(), pal().text).size().x.min(160.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(text + 34.0, 26.0), Sense::click());
    if resp.hovered() && !on {
        ui.painter().rect_filled(rect.shrink2(Vec2::new(0.0, 1.0)), 6.0, pal().hover);
    }
    let color = if on || resp.hovered() { pal().text } else { pal().text_dim };
    paint_look(ui, st, ext, look, Pos2::new(rect.left() + 14.0, rect.center().y), 13.0, color);
    text_fit(ui.painter(), Pos2::new(rect.left() + 26.0, rect.center().y), title, font, color, text + 2.0);
    if on {
        let mark = Rect::from_min_max(
            Pos2::new(rect.left() + 6.0, rect.bottom() - 2.0),
            Pos2::new(rect.right() - 6.0, rect.bottom()),
        );
        ui.painter().rect_filled(mark, 1.0, pal().accent);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn view_tabs(ui: &mut egui::Ui, st: &mut UiState, ext: &Extensions, showing: Option<&Panel>) {
    let console = RailLook::Glyph(codicon("terminal"));
    if title_tab(ui, st, ext, &console, "Console", showing.is_none()).clicked() {
        st.console_tab = None;
    }
    for tab in st.console_tabs.clone() {
        let resp = title_tab(ui, st, ext, &tab.look, &tab.tip, showing == Some(&tab.panel));
        resp.context_menu(|ui| {
            ui.set_width(210.0);
            for side in Side::BOTH {
                let label = match (side == Side::Right) != st.prefs.dock_right {
                    true => "Move to the right bar",
                    false => "Move to the left bar",
                };
                if menu_row(ui, icons::PANEL_LEFT, label) {
                    st.rail_move = Some((tab.panel.clone(), side, None));
                    ui.close();
                }
            }
        });
        if resp.on_hover_text(format!("{}\nRight click it to move it to a bar", tab.tip)).clicked() {
            st.console_tab = Some(tab.panel.clone());
        }
    }
}

fn header(ui: &mut egui::Ui, st: &mut UiState, ext: &Extensions, actions: &mut Vec<Action>, card: u64) {
    let front = st.console.front(card);
    let filled = st.console.filled == Some(card);
    let keys = st.keys.label(Command::SplitConsole);
    let crossed = ui.ctx().data(|data| data.get_temp::<bool>(egui::Id::unique(super::CARD_X))).unwrap_or(false);
    let in_panel = ui.ctx().data(|data| data.get_temp::<bool>(egui::Id::unique(super::IN_PANEL))).unwrap_or(false);
    let tabbed = card == MAIN && !st.console_tabs.is_empty();
    let showing = st.console_tab.clone().filter(|tab| tabbed && st.tabbed(tab));
    let place = Frame::NONE.inner_margin(Margin { left: 12, right: 8, top: 8, bottom: 2 }).show(ui, |ui| {
        ui.horizontal(|ui| {
            match tabbed {
                true => {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    view_tabs(ui, st, ext, showing.as_ref());
                }
                false => {
                    ui.add(egui::Label::new(RichText::new("Console").size(14.0).color(pal().text)).selectable(false));
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                if crossed {
                    ui.add_space(26.0);
                }
                let (glyph, tip) = match filled {
                    true => ("screen-normal", "Restore the card"),
                    false => ("screen-full", "Fill the canvas with the card"),
                };
                if !in_panel && head_button(ui, codicon(glyph), tip, 24.0, false).0 {
                    actions.push(Action::Console(ConsoleOp::Fill(Some(card))));
                }
                if showing.is_some() {
                    return;
                }
                let more =
                    st.console.menu.as_ref().is_some_and(|menu| menu.card == card && menu.kind == MenuKind::More);
                let (clicked, rect) = head_button(ui, codicon("ellipsis"), "More", 24.0, more);
                if clicked {
                    open_menu(
                        st,
                        card,
                        MenuKind::More,
                        MenuPage::Main,
                        Pos2::new(rect.right() - 240.0, rect.bottom() + 4.0),
                    );
                }
                if head_button(ui, codicon("trash"), "Kill the console", 24.0, false).0 {
                    actions.push(Action::Console(ConsoleOp::Kill(front)));
                }
                let tip = format!("Split the console ({keys})");
                if head_button(ui, codicon("split-horizontal"), &tip, 24.0, false).0 {
                    actions.push(Action::Console(ConsoleOp::Split { tab: front, profile: None }));
                }
            });
        });
    });
    st.console.heads.insert(card, place.response.rect);
}

struct Tab {
    id: u64,
    group: u64,
    title: String,
    icon: &'static str,
    color: Option<u8>,
    news: bool,
    broke: bool,
    ended: bool,
}

/// The consoles of the card as tabs under its title, each a chip like the folder's in the files panel, with the
/// button for a new console and the menu of its kinds after them.
fn tabs(ui: &mut egui::Ui, st: &mut UiState, actions: &mut Vec<Action>, card: u64) {
    const HIGH: f32 = 26.0;
    const GAP: f32 = 4.0;
    const WIDEST: f32 = 190.0;
    const NARROWEST: f32 = 46.0;
    let shown = st.console.shown(card);
    let all: Vec<Tab> = st
        .console
        .of_card(card)
        .map(|tab| {
            let icon = tab.icon.as_deref().map(codicon).unwrap_or_else(|| {
                codicon(st.console.profile(&tab.profile).map_or("terminal", |profile| profile.icon()))
            });
            let (title, ended) = (tab.title(), tab.ended());
            Tab { id: tab.id, group: tab.group, title, icon, color: tab.color, news: tab.news, broke: tab.broke, ended }
        })
        .collect();
    let row = Frame::NONE.inner_margin(Margin { left: 10, right: 8, top: 2, bottom: 2 }).show(ui, |ui| {
        let (strip, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), HIGH), Sense::hover());
        strip
    });
    let strip = row.inner;
    let font = FontId::proportional(12.5);
    let wants: Vec<f32> = all
        .iter()
        .map(|tab| {
            let text = ui.painter().layout_no_wrap(tab.title.clone(), font.clone(), pal().text).size().x;
            (text + 52.0).clamp(NARROWEST, WIDEST)
        })
        .collect();
    let buttons = 50.0;
    let room = (strip.width() - buttons - GAP * all.len() as f32).max(NARROWEST);
    let total: f32 = wants.iter().sum();
    let scale = if total > room { room / total } else { 1.0 };
    let renaming = st.console.renaming.as_ref().map(|(id, _)| *id);
    let quiet = ui.memory(|memory| memory.focused().is_none());
    let (f2, delete) = ui.input(|input| (input.key_pressed(egui::Key::F2), input.key_pressed(egui::Key::Delete)));
    let mut x = strip.left();
    for (i, (tab, want)) in all.iter().zip(&wants).enumerate() {
        let width = (want * scale).max(NARROWEST);
        if x + width > strip.right() - buttons {
            break;
        }
        let rect = Rect::from_min_size(Pos2::new(x, strip.top()), Vec2::new(width, HIGH));
        let mate = all.get(i + 1).is_some_and(|next| next.group == tab.group);
        x += width + if mate { 1.0 } else { GAP };
        let resp = ui.interact(rect, egui::Id::unique(("console tab", tab.id)), Sense::click());
        let front = shown.contains(&tab.id);
        let hovered = resp.hovered();
        let fill = if front {
            pal().raised
        } else if hovered {
            pal().hover
        } else {
            Color32::TRANSPARENT
        };
        let edge = if front { pal().border_strong } else { Color32::TRANSPARENT };
        ui.painter().rect(rect, 7.0, fill, Stroke::new(1.0, edge), egui::StrokeKind::Inside);
        let painter = ui.painter().with_clip_rect(rect.shrink(1.0));
        let mid = rect.center().y;
        let glyph_color = if tab.ended { pal().text_faint } else { tint(tab.color) };
        let icon_at = Pos2::new(rect.left() + 15.0, mid);
        painter.text(icon_at, Align2::CENTER_CENTER, tab.icon, FontId::proportional(13.5), glyph_color);
        let dot = if tab.broke {
            Some(red())
        } else if tab.news {
            Some(pal().accent)
        } else {
            None
        };
        let close = Rect::from_center_size(Pos2::new(rect.right() - 13.0, mid), Vec2::splat(18.0));
        let closing = (hovered || front) && width > 70.0;
        let left = rect.left() + 28.0;
        let right = if closing || dot.is_some() { close.left() - 2.0 } else { rect.right() - 8.0 };
        if renaming == Some(tab.id) {
            let field = Rect::from_min_max(
                Pos2::new(left - 2.0, rect.top() + 2.0),
                Pos2::new(rect.right() - 4.0, rect.bottom() - 2.0),
            );
            ui.scope_builder(egui::UiBuilder::new().max_rect(field), |ui| {
                rename_field(ui, st, actions, tab.id, field.width() - 4.0);
            });
        } else if width > 60.0 {
            let color = if tab.ended {
                pal().text_faint
            } else if front {
                pal().text
            } else {
                pal().text_dim
            };
            text_fit(&painter, Pos2::new(left, mid), &tab.title, font.clone(), color, right - left);
        }
        let mut closed = false;
        if closing && renaming != Some(tab.id) {
            let cross = ui.interact(close, egui::Id::unique(("console tab close", tab.id)), Sense::click());
            if cross.hovered() {
                ui.painter().rect_filled(close, 5.0, pal().border_strong);
            }
            let color = if cross.hovered() { pal().text } else { pal().text_faint };
            painter.text(close.center(), Align2::CENTER_CENTER, icons::X, FontId::proportional(12.0), color);
            closed = cross.on_hover_text("End this console").clicked();
        } else if let Some(dot) = dot {
            painter.circle_filled(close.center(), 3.5, dot);
        }
        if closing && !closed {
            if let Some(dot) = dot {
                painter.circle_filled(Pos2::new(rect.right() - 4.0, rect.top() + 4.0), 3.0, dot);
            }
        }
        if closed {
            actions.push(Action::Console(ConsoleOp::Kill(Some(tab.id))));
            continue;
        }
        let tip = match (tab.broke, tab.news) {
            (true, _) => format!("{}\nSomething in it ended with an error", tab.title),
            (false, true) => format!("{}\nIt printed something new", tab.title),
            _ => format!("{}\nDouble-click to rename, right click for more", tab.title),
        };
        let resp = resp.on_hover_text(tip);
        if resp.double_clicked() {
            actions.push(Action::Console(ConsoleOp::Renaming(Some(tab.id))));
        } else if resp.clicked() {
            actions.push(Action::Console(ConsoleOp::Front(tab.id)));
        }
        if resp.secondary_clicked() {
            let at = resp.interact_pointer_pos().unwrap_or(rect.left_bottom());
            st.console.menu = Some(Menu { card, kind: MenuKind::Tab(tab.id), page: MenuPage::Main, at, place: None });
        }
        if hovered && quiet && f2 {
            actions.push(Action::Console(ConsoleOp::Renaming(Some(tab.id))));
        }
        if hovered && quiet && delete {
            actions.push(Action::Console(ConsoleOp::Kill(Some(tab.id))));
        }
    }
    let keys = st.keys.label(Command::NewConsole);
    let mut at = x.max(strip.left());
    let add = Rect::from_min_size(Pos2::new(at, strip.top() + 1.0), Vec2::new(24.0, 24.0));
    at += 24.0;
    let pick = Rect::from_min_size(Pos2::new(at, strip.top() + 1.0), Vec2::new(18.0, 24.0));
    let profiles = st.console.menu.as_ref().is_some_and(|menu| menu.card == card && menu.kind == MenuKind::New);
    let new = ui.interact(add, egui::Id::unique(("console new", card)), Sense::click());
    let kinds = ui.interact(pick, egui::Id::unique(("console kinds", card)), Sense::click());
    for (rect, resp, glyph, on) in [(add, &new, "add", false), (pick, &kinds, "chevron-down", profiles)] {
        if resp.hovered() || on {
            ui.painter().rect_filled(rect, 6.0, pal().hover);
        }
        let color = if resp.hovered() || on { pal().text } else { pal().text_dim };
        let size = if glyph == "add" { 14.0 } else { 12.0 };
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, codicon(glyph), FontId::proportional(size), color);
    }
    if new.on_hover_text(format!("New console ({keys})")).clicked() {
        actions.push(Action::Console(ConsoleOp::New { card: Some(card), profile: None, dir: None }));
    }
    if kinds.on_hover_text("Pick the kind of console").clicked() {
        open_menu(st, card, MenuKind::New, MenuPage::Main, Pos2::new(pick.left(), pick.bottom() + 4.0));
    }
}

fn message(ui: &mut egui::Ui, rect: Rect, text: &str, button: &str) -> bool {
    let mut clicked = false;
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
        Frame::NONE.inner_margin(Margin::same(12)).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            says(ui, text);
            clicked = ask_button(ui, button, ChoiceLook::Main);
        });
    });
    clicked
}

fn pane(ui: &mut egui::Ui, st: &mut UiState, actions: &mut Vec<Action>, id: u64, rect: Rect, ring: bool) {
    let Some(tab) = st.console.tab_mut(id) else { return };
    if let Some(why) = tab.failed.clone() {
        if message(ui, rect, &format!("The shell did not start: {why}"), "Try again") {
            actions.push(Action::Console(ConsoleOp::Start(id)));
        }
        return;
    }
    let Some(term) = tab.term.as_mut() else {
        actions.push(Action::Console(ConsoleOp::Start(id)));
        return;
    };
    let mut area = rect;
    if term.ended {
        let bar = Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - 34.0), rect.max);
        area.max.y = bar.top() - 4.0;
        let said = match term.exit {
            Some(0) | None => "The shell ended.".to_string(),
            Some(code) => format!("The shell ended with code {code}."),
        };
        ui.scope_builder(egui::UiBuilder::new().max_rect(bar.shrink2(Vec2::new(10.0, 0.0))), |ui| {
            ui.horizontal_centered(|ui| {
                says(ui, &said);
                if ask_button(ui, "Start a new one", ChoiceLook::Plain) {
                    actions.push(Action::Console(ConsoleOp::Start(id)));
                }
            });
        });
    }
    let picked = screen(ui, id, area, term, &mut tab.focus, &mut tab.wheel, &mut tab.mouse, ring);
    st.console.size = st.console.tab(id).and_then(|tab| tab.term.as_ref()).map(Terminal::size);
    if picked && st.console.current != Some(id) {
        actions.push(Action::Console(ConsoleOp::Front(id)));
    }
}

pub fn console_panel(ui: &mut egui::Ui, st: &mut UiState, ext: &mut Extensions, actions: &mut Vec<Action>, card: u64) {
    if !st.console.looked {
        st.console.look();
    }
    header(ui, st, ext, actions, card);
    let view = st.console_tab.clone().filter(|tab| card == MAIN && st.tabbed(tab));
    match view {
        Some(Panel::Webview(view)) => return ext_panels::webview_panel(ui, st, ext, &view),
        Some(Panel::Tree(id)) => return ext_panels::tree_panel(ui, st, ext, &id),
        _ => {}
    }
    if st.console.profiles.is_empty() {
        no_shell(ui, st, actions);
        return;
    }
    let ids: Vec<u64> = st.console.of_card(card).map(|tab| tab.id).collect();
    if ids.is_empty() {
        match card {
            MAIN => actions.push(Action::Console(ConsoleOp::New { card: Some(MAIN), profile: None, dir: None })),
            _ => st.console.emptied.push(card),
        }
        return;
    }
    st.console.seen(card);
    tabs(ui, st, actions, card);
    let body = ui.available_rect_before_wrap();
    ui.allocate_rect(body, Sense::hover());
    let room = Rect::from_min_max(body.min + Vec2::new(8.0, 4.0), Pos2::new(body.right() - 8.0, body.bottom() - 8.0));
    let shown = st.console.shown(card);
    let current = st.console.current;
    const GAP: f32 = 6.0;
    let count = shown.len().max(1) as f32;
    let width = ((room.width() - GAP * (count - 1.0)) / count).max(40.0);
    for (i, id) in shown.iter().enumerate() {
        let left = room.left() + i as f32 * (width + GAP);
        let rect = Rect::from_min_size(Pos2::new(left, room.top()), Vec2::new(width, room.height()));
        pane(ui, st, actions, *id, rect, shown.len() > 1 && current == Some(*id));
    }
    menu(ui.ctx(), st, actions, card);
}

fn back_row(ui: &mut egui::Ui, page: &mut MenuPage) {
    if menu_row(ui, icons::ARROW_LEFT, "Back") {
        *page = MenuPage::Main;
    }
    menu_gap(ui);
}

fn more_row(ui: &mut egui::Ui, glyph: &str, label: &str) -> bool {
    menu_row_key(ui, glyph, label, "›")
}

fn menu(ctx: &egui::Context, st: &mut UiState, actions: &mut Vec<Action>, card: u64) {
    const WIDTH: f32 = 248.0;
    if st.console.menu.as_ref().is_none_or(|menu| menu.card != card) {
        return;
    }
    let profiles = st.console.profiles.clone();
    let default = st.console.default_profile(&st.prefs.console_profile).map(|profile| profile.id.clone());
    let folders: Vec<(std::path::PathBuf, String)> =
        st.workspace.iter().filter(|row| row.page.is_some()).map(|row| (row.path.clone(), row.title.clone())).collect();
    let (history, dirs) = (st.console.history.clone(), st.console.dirs.clone());
    let keys = [Command::SplitConsole].map(|command| st.keys.label(command));
    let current = st.console.current;
    let Some(menu) = st.console.menu.as_ref() else { return };
    let pressed = ctx.input(|i| i.pointer.any_pressed().then(|| i.pointer.interact_pos()).flatten());
    let outside = pressed.is_some_and(|at| menu.place.is_some_and(|place| !place.contains(at)));
    if outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        st.console.menu = None;
        return;
    }
    let screen = ctx.content_rect();
    let tall = menu.place.map_or(260.0, |place| place.height());
    let at = Pos2::new(
        menu.at.x.min(screen.right() - WIDTH - 20.0).max(screen.left() + 4.0),
        menu.at.y.min(screen.bottom() - tall - 8.0).max(screen.top() + 4.0),
    );
    let (kind, mut page) = (menu.kind, menu.page);
    let mut ops: Vec<ConsoleOp> = Vec::new();
    let mut close = false;
    let group_size = |id: Option<u64>| {
        let tab = id.and_then(|id| st.console.tab(id));
        tab.map_or(0, |tab| st.console.of_card(tab.card).filter(|other| other.group == tab.group).count())
    };
    let (current_mates, menu_tab) = match kind {
        MenuKind::Tab(id) => (group_size(Some(id)), st.console.tab(id)),
        _ => (group_size(current), None),
    };
    let (chosen_color, chosen_icon) = menu_tab.map_or((None, None), |tab| (tab.color, tab.icon.clone()));
    let area = egui::Area::new(egui::Id::unique(("console menu", card))).fixed_pos(at).order(egui::Order::Foreground);
    let resp = area.show(ctx, |ui| {
        let frame = Frame::NONE
            .fill(pal().card)
            .stroke(Stroke::new(1.0, pal().border))
            .corner_radius(egui::CornerRadius::same(10))
            .inner_margin(Margin::same(6));
        frame.show(ui, |ui| {
            ui.set_width(WIDTH);
            ui.spacing_mut().item_spacing.y = 1.0;
            let profile_rows = |ui: &mut egui::Ui, ops: &mut Vec<ConsoleOp>, make: &dyn Fn(String) -> ConsoleOp| {
                for profile in &profiles {
                    let label = match default.as_deref() == Some(profile.id.as_str()) {
                        true => format!("{} (Default)", profile.name),
                        false => profile.name.clone(),
                    };
                    if menu_row(ui, codicon(profile.icon()), &label) {
                        ops.push(make(profile.id.clone()));
                    }
                }
            };
            let showing = page;
            match (kind, showing) {
                (MenuKind::New, MenuPage::Main) => {
                    profile_rows(ui, &mut ops, &|profile| ConsoleOp::New {
                        card: Some(card),
                        profile: Some(profile),
                        dir: None,
                    });
                    let no_bash = cfg!(windows) && !profiles.iter().any(|profile| profile.id == "bash");
                    if no_bash && menu_row(ui, icons::DOWNLOAD, "Git Bash: install Git for Windows") {
                        ops.push(ConsoleOp::Install);
                    }
                    menu_gap(ui);
                    if more_row(ui, codicon("split-horizontal"), "Split Console with Profile") {
                        page = MenuPage::SplitWith;
                    }
                    if more_row(ui, icons::CHECK, "Select Default Profile") {
                        page = MenuPage::Default;
                    }
                    if !folders.is_empty() && more_row(ui, icons::FOLDER, "New Console in Folder") {
                        page = MenuPage::Folders;
                    }
                }
                (MenuKind::New, MenuPage::SplitWith) => {
                    back_row(ui, &mut page);
                    profile_rows(ui, &mut ops, &|profile| ConsoleOp::Split { tab: current, profile: Some(profile) });
                }
                (MenuKind::New, MenuPage::Default) => {
                    back_row(ui, &mut page);
                    profile_rows(ui, &mut ops, &|profile| ConsoleOp::Default(profile));
                }
                (MenuKind::New, MenuPage::Folders) => {
                    back_row(ui, &mut page);
                    for (path, title) in &folders {
                        if menu_row(ui, icons::FOLDER, title) {
                            ops.push(ConsoleOp::New { card: Some(card), profile: None, dir: Some(path.clone()) });
                        }
                    }
                }
                (MenuKind::More, MenuPage::Main) => {
                    if menu_row(ui, codicon("clear-all"), "Clear Console") {
                        ops.push(ConsoleOp::Clear(None));
                    }
                    if menu_row(ui, codicon("play"), "Run Selected Text") {
                        ops.push(ConsoleOp::RunSelection);
                    }
                    if menu_row(ui, codicon("run-all"), "Run Active File") {
                        ops.push(ConsoleOp::RunFile);
                    }
                    if more_row(ui, icons::FOLDER_OPEN, "Go to Recent Directory") {
                        page = MenuPage::Dirs;
                    }
                    if more_row(ui, icons::HISTORY, "Run Recent Command") {
                        page = MenuPage::Commands;
                    }
                    menu_gap(ui);
                    if menu_row_key(ui, icons::ARROW_UP, "Scroll to Previous Command", "Ctrl+Up") {
                        ops.push(ConsoleOp::ToCommand(true));
                    }
                    if menu_row_key(ui, icons::ARROW_DOWN, "Scroll to Next Command", "Ctrl+Down") {
                        ops.push(ConsoleOp::ToCommand(false));
                    }
                    menu_gap(ui);
                    if menu_row_key(ui, icons::PENCIL, "Rename", "F2") {
                        ops.push(ConsoleOp::Renaming(None));
                    }
                    if menu_row(ui, codicon("empty-window"), "Move to a New Card") {
                        ops.push(ConsoleOp::ToCard(None));
                    }
                    if current_mates > 1 && menu_row(ui, codicon("split-horizontal"), "Unsplit") {
                        ops.push(ConsoleOp::Unsplit(None));
                    }
                }
                (MenuKind::More, MenuPage::Dirs) => {
                    back_row(ui, &mut page);
                    if dirs.is_empty() {
                        ui.label(RichText::new("No folder yet").size(12.0).color(pal().text_faint));
                    }
                    for dir in dirs.iter().take(14) {
                        if menu_row(ui, icons::FOLDER, &dir.display().to_string()) {
                            ops.push(ConsoleOp::Cd(dir.clone()));
                        }
                    }
                }
                (MenuKind::More, MenuPage::Commands) => {
                    back_row(ui, &mut page);
                    if history.is_empty() {
                        ui.label(RichText::new("No command yet").size(12.0).color(pal().text_faint));
                    }
                    for command in history.iter().take(14) {
                        let (rect, resp) =
                            ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::click());
                        let run = Rect::from_min_max(Pos2::new(rect.right() - 28.0, rect.top()), rect.max);
                        let run_resp = ui.interact(run, egui::Id::unique(("run recent", command)), Sense::click());
                        if resp.hovered() || run_resp.hovered() {
                            ui.painter().rect_filled(rect, 6.0, pal().hover);
                        }
                        if run_resp.hovered() {
                            ui.painter().rect_filled(run.shrink(2.0), 5.0, pal().raised);
                        }
                        let painter = ui.painter();
                        let mid = rect.center().y;
                        painter.text(
                            Pos2::new(rect.left() + 10.0, mid),
                            Align2::LEFT_CENTER,
                            icons::HISTORY,
                            FontId::proportional(13.0),
                            pal().text_dim,
                        );
                        text_fit(
                            painter,
                            Pos2::new(rect.left() + 32.0, mid),
                            command,
                            FontId::monospace(12.0),
                            pal().text,
                            rect.width() - 64.0,
                        );
                        painter.text(
                            run.center(),
                            Align2::CENTER_CENTER,
                            codicon("play"),
                            FontId::proportional(14.0),
                            pal().text_dim,
                        );
                        if run_resp.on_hover_text("Run it").clicked() {
                            ops.push(ConsoleOp::Type { text: command.clone(), run: true });
                        } else if resp.on_hover_text("Type it into the console").clicked() {
                            ops.push(ConsoleOp::Type { text: command.clone(), run: false });
                        }
                    }
                }
                (MenuKind::Tab(id), MenuPage::Main) => {
                    if menu_row_key(ui, codicon("split-horizontal"), "Split", &keys[0]) {
                        ops.push(ConsoleOp::Split { tab: Some(id), profile: None });
                    }
                    if menu_row(ui, codicon("empty-window"), "Move to a New Card") {
                        ops.push(ConsoleOp::ToCard(Some(id)));
                    }
                    if more_row(ui, icons::PALETTE, "Change Colour") {
                        page = MenuPage::Colour;
                    }
                    if more_row(ui, codicon("symbol-misc"), "Change Icon") {
                        page = MenuPage::Icon;
                    }
                    if menu_row_key(ui, icons::PENCIL, "Rename", "F2") {
                        ops.push(ConsoleOp::Renaming(Some(id)));
                    }
                    if current_mates > 1 && menu_row(ui, codicon("split-horizontal"), "Unsplit") {
                        ops.push(ConsoleOp::Unsplit(Some(id)));
                    }
                    menu_gap(ui);
                    if menu_row_key(ui, codicon("trash"), "Kill", "Del") {
                        ops.push(ConsoleOp::Kill(Some(id)));
                    }
                    let (f2, delete) = ui.input(|i| (i.key_pressed(egui::Key::F2), i.key_pressed(egui::Key::Delete)));
                    if f2 {
                        ops.push(ConsoleOp::Renaming(Some(id)));
                    }
                    if delete {
                        ops.push(ConsoleOp::Kill(Some(id)));
                    }
                }
                (MenuKind::Tab(id), MenuPage::Colour) => {
                    back_row(ui, &mut page);
                    let chosen = chosen_color;
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        for color in std::iter::once(None).chain(COLOURS.map(Some)) {
                            let (rect, resp) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
                            if resp.hovered() || chosen == color {
                                ui.painter().rect_filled(rect, 6.0, pal().hover);
                            }
                            match color {
                                Some(_) => ui.painter().circle_filled(rect.center(), 7.0, tint(color)),
                                None => {
                                    ui.painter().circle_stroke(rect.center(), 7.0, Stroke::new(1.5, pal().text_dim))
                                }
                            };
                            let tip = if color.is_none() { "As it was" } else { "This colour" };
                            if resp.on_hover_text(tip).clicked() {
                                ops.push(ConsoleOp::Colour(id, color));
                            }
                        }
                    });
                }
                (MenuKind::Tab(id), MenuPage::Icon) => {
                    back_row(ui, &mut page);
                    let chosen = chosen_icon.clone();
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        for name in std::iter::once(None).chain(ICONS.map(Some)) {
                            let (rect, resp) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
                            if resp.hovered() || chosen.as_deref() == name {
                                ui.painter().rect_filled(rect, 6.0, pal().hover);
                            }
                            let glyph = codicon(name.unwrap_or("debug-restart"));
                            ui.painter().text(
                                rect.center(),
                                Align2::CENTER_CENTER,
                                glyph,
                                FontId::proportional(15.0),
                                pal().text,
                            );
                            if resp.on_hover_text(name.unwrap_or("The shell's own")).clicked() {
                                ops.push(ConsoleOp::Icon(id, name.map(str::to_string)));
                            }
                        }
                    });
                }
                _ => close = true,
            }
        });
    });
    if let Some(menu) = st.console.menu.as_mut() {
        if page != menu.page {
            menu.page = page;
            menu.place = None;
        } else {
            menu.place = Some(resp.response.rect);
        }
    }
    if close || !ops.is_empty() {
        st.console.menu = None;
    }
    actions.extend(ops.into_iter().map(Action::Console));
}

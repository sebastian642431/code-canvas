use super::*;

impl App {
    /// The `term` step of a script: the mouse and the copy key on the console's screen, aimed at text shown on it, and
    /// a report of what is selected, what the clipboard holds and the line the cursor is on.
    pub(super) fn term_step(&mut self, what: &str) {
        let mut words = what.split_whitespace();
        let verb = words.next().unwrap_or_default();
        let args: Vec<&str> = words.collect();
        match verb {
            "report" => return self.term_report(),
            "copy" => return self.ui_events.push_back(vec![egui::Event::Copy]),
            _ => {}
        }
        let console = &self.ui.console;
        let (Some(term), Some((origin, cell))) = (console.term.as_ref(), console.mouse.place) else {
            return log::warn!("script: term {what}: the console is not on screen");
        };
        let (rows, cols) = term.size();
        // The row and column where a text starts, the lowest row it is on: output comes after the line typed.
        let find = |text: &str| {
            let rows = term.screen().rows(0, cols).enumerate();
            let found = rows.filter_map(|(row, line)| line.find(text).map(|i| (row as u16, line[..i].chars().count())));
            let found = found.last();
            if found.is_none() {
                log::warn!("script: term {what}: '{text}' is not on the console's screen");
            }
            found
        };
        let point = |row: u16, col: f32| origin + egui::vec2(col * cell.x, (f32::from(row) + 0.5) * cell.y);
        let press = |pos, button, pressed| egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let (left, right) = (egui::PointerButton::Primary, egui::PointerButton::Secondary);
        let frames: Vec<Vec<egui::Event>> = match (verb, args.as_slice()) {
            ("drag", [from, to]) => {
                let (Some(start), Some(end)) = (find(from), find(to)) else { return };
                let from = point(start.0, start.1 as f32 + 0.2);
                let to = point(end.0, (end.1 + to.chars().count()) as f32 - 0.2);
                let mut frames = vec![vec![egui::Event::PointerMoved(from)], vec![press(from, left, true)]];
                frames.extend((1..=8).map(|i| vec![egui::Event::PointerMoved(from.lerp(to, i as f32 / 8.0))]));
                frames.push(vec![press(to, left, false)]);
                frames
            }
            ("click", [text, times @ ..]) => {
                let Some((row, col)) = find(text) else { return };
                let times = times.first().and_then(|n| n.parse().ok()).unwrap_or(1usize);
                let at = point(row, col as f32 + 0.5);
                let mut frames = vec![vec![egui::Event::PointerMoved(at)]];
                for _ in 0..times {
                    frames.extend([vec![press(at, left, true)], vec![press(at, left, false)]]);
                }
                frames
            }
            ("right", rest) if rest.len() <= 1 => {
                let at = match rest.first() {
                    Some(text) => {
                        let Some((row, col)) = find(text) else { return };
                        point(row, col as f32 + 0.5)
                    }
                    None => point(rows / 2, f32::from(cols) / 2.0),
                };
                let moved = vec![egui::Event::PointerMoved(at)];
                vec![moved, vec![press(at, right, true)], vec![press(at, right, false)]]
            }
            _ => {
                log::warn!("script: term needs drag <text> <text>, click <text> [times], right [text], copy or report");
                Vec::new()
            }
        };
        self.ui_events.extend(frames);
    }

    fn term_report(&mut self) {
        let Some(term) = self.ui.console.term.as_mut() else {
            return log::warn!("script: term report: the console has no shell");
        };
        let selected = term.selected_text();
        let (row, _) = term.screen().cursor_position();
        let cols = term.size().1;
        let line = term.screen().rows(0, cols).nth(usize::from(row)).unwrap_or_default();
        log::info!("script: console selection {selected:?}");
        log::info!("script: console clipboard {:?}", crate::term::clipboard().unwrap_or_default());
        log::info!("script: console line at the cursor {:?}", line.trim_end());
    }
}

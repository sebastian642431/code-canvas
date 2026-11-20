use super::*;
use crate::churn::{Cause, Count, Warning};

pub(super) const ASKED: &str = "churn:";
const STOP: &str = "Stop following this file";
const HIDE: &str = "Hide it from the canvas";

impl App {
    pub(super) fn is_unfollowed(&self, path: &Path) -> bool {
        !self.unfollowed.is_empty() && self.unfollowed.contains_key(&ext::path_key(&path.to_string_lossy()))
    }

    pub(super) fn watch_churn(&mut self) {
        let now = Instant::now();
        for page in &mut self.pages {
            let (git, scans) = (std::mem::take(&mut page.git_loads), std::mem::take(&mut page.scans));
            self.churn.git_reloaded(now, &page.root, git);
            self.churn.rescanned(now, &page.root, scans);
        }
        let Some(verdict) = self.churn.check(now) else { return };
        let busy: Vec<String> = verdict.top.iter().map(|count| self.churn_line(count)).collect();
        if verdict.log {
            let worst: Vec<&str> = busy.iter().take(3).map(String::as_str).collect();
            log::warn!("busy in the last minute: {}", worst.join("; "));
        }
        if busy != self.ui.busy {
            self.ui.busy = busy;
            self.request_redraw();
        }
        if let Some(warning) = verdict.warning {
            self.warn_churn(warning);
        }
    }

    fn folder_title(&self, folder: &Path) -> String {
        match self.pages.iter().find(|page| page.root == folder) {
            Some(page) => page.title.clone(),
            None => {
                folder.file_name().map_or_else(|| folder.display().to_string(), |n| n.to_string_lossy().into_owned())
            }
        }
    }

    fn churn_line(&self, count: &Count) -> String {
        let (title, n) = (self.folder_title(count.cause.folder()), count.count);
        match &count.cause {
            Cause::Writes { rel, .. } => format!("{rel} in {title}: written {n} times"),
            Cause::GitReloads(_) => format!("{title}: git read {n} times"),
            Cause::Rescans(_) => format!("{title}: scanned {n} times"),
        }
    }

    fn warn_churn(&mut self, warning: Warning) {
        let Warning { folder, path, git, scans } = warning;
        let title = self.folder_title(&folder);
        let mut then = Vec::new();
        if git > 0 {
            then.push(format!("read git again {git} times"));
        }
        if scans > 0 {
            then.push(format!("scanned the folder again {scans} times"));
        }
        let then = then.join(" and ");
        let Some((rel, n)) = path else {
            let text = format!(
                "In the last minute Hawk Canvas {then} for {title}. Something keeps changing that folder (a build, a server?)."
            );
            log::warn!("{text}");
            self.ext.notice(ext::Level::Warn, "Hawk Canvas", text);
            self.request_redraw();
            return;
        };
        let mut text = format!("{rel} in {title} was written {n} times in the last minute");
        if !then.is_empty() {
            text.push_str(&format!(", and Hawk Canvas {then}"));
        }
        text.push_str(
            ". Something keeps writing to it (a server's log?). Stop following it and its changes cost nothing here.",
        );
        log::warn!("{text}");
        let hideable = self.pages.iter().find(|page| page.root == folder).is_some_and(|page| {
            !page.filter.hidden.contains(&rel) && page.ready().is_some_and(|r| r.repo.file_index(&rel).is_some())
        });
        let choices: &[&str] = if hideable { &[STOP, HIDE, "Dismiss"] } else { &[STOP, "Dismiss"] };
        let about = format!("{ASKED}{}\n{rel}", folder.display());
        self.ext.ask_at(ext::Level::Warn, "Hawk Canvas", text, choices, &about);
        self.request_redraw();
    }

    pub(super) fn churn_answered(&mut self, about: &str, choice: &str) {
        let Some((folder, rel)) = about.split_once('\n') else { return };
        let folder = PathBuf::from(folder);
        match choice {
            STOP => {
                let key = ext::path_key(&folder.join(rel).to_string_lossy());
                self.unfollowed.insert(key, (folder.clone(), rel.to_string()));
                self.churn.forget(&folder, rel);
                self.ui.unfollowed = self.unfollowed_names();
                log::info!("{rel} in {} is not followed any more", folder.display());
                let text = format!(
                    "Changes to {rel} are not followed any more. \"Not followed\" in the status bar follows it again."
                );
                self.ext.notice(ext::Level::Info, "Hawk Canvas", text);
            }
            HIDE => {
                if let Some(i) = self.pages.iter().position(|page| page.root == folder) {
                    self.pages[i].filter.hidden.insert(rel.to_string());
                    if let Some(g) = &self.gfx {
                        self.pages[i].relayout(&g.renderer, &g.device, &self.ui.filters);
                    }
                }
            }
            _ => {}
        }
        self.request_redraw();
    }

    fn unfollowed_names(&self) -> Vec<String> {
        let mut names: Vec<String> =
            self.unfollowed.values().map(|(folder, rel)| format!("{rel} in {}", self.folder_title(folder))).collect();
        names.sort();
        names
    }

    pub(super) fn follow_again(&mut self) {
        let files: Vec<PathBuf> = self.unfollowed.drain().map(|(_, (folder, rel))| folder.join(rel)).collect();
        self.ui.unfollowed.clear();
        if files.is_empty() {
            return;
        }
        log::info!("{} files are followed again", files.len());
        self.ext.notice(ext::Level::Info, "Hawk Canvas", "Changes to the files not followed are followed again.");
        self.on_disk_changes(crate::watch::Batch { files, git: Vec::new() });
    }

    pub(super) fn show_churn(&mut self) {
        if let Some(warning) = self.churn.warning_now(Instant::now()) {
            self.warn_churn(warning);
        }
    }

    pub(super) fn script_churn(&mut self) {
        let top = self.churn.top(Instant::now(), 5);
        let lines: Vec<String> = top.iter().map(|count| self.churn_line(count)).collect();
        match lines.is_empty() {
            true => log::info!("script: churn: nothing repeats"),
            false => log::info!("script: churn: {}", lines.join("; ")),
        }
    }
}

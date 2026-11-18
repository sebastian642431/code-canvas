use super::*;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Worked {
    pub root: PathBuf,
    pub shared: bool,
    pub decided: Option<bool>,
}

pub(super) fn host_folders(worked: &[Worked]) -> (Vec<PathBuf>, bool) {
    let mut trusted = true;
    let mut folders = Vec::new();
    for folder in worked {
        let trusts = folder.decided == Some(true);
        if folder.shared && !trusts {
            continue;
        }
        trusted &= trusts;
        folders.push(folder.root.clone());
    }
    (folders, trusted)
}

pub(super) fn to_ask<'a>(worked: &'a [Worked], asked: &HashSet<PathBuf>, held_back: bool) -> Option<&'a Worked> {
    if !held_back {
        return None;
    }
    worked.iter().find(|folder| folder.decided.is_none() && !asked.contains(&folder.root))
}

impl App {
    fn worked_in(&self) -> Vec<Worked> {
        self.pages
            .iter()
            .filter(|p| !p.is_waiting() && !p.only)
            .map(|p| Worked {
                root: p.root.clone(),
                shared: p.shared.is_some(),
                decided: self.trust.decided(&p.root, p.shared.is_some()),
            })
            .collect()
    }

    pub(super) fn host_folders(&self) -> (Vec<PathBuf>, bool) {
        host_folders(&self.worked_in())
    }

    pub(super) fn ask_trust(&mut self) {
        let worked = self.worked_in();
        let held_back = !self.ext.held_back().is_empty();
        let shared_out = worked.iter().any(|f| f.shared && f.decided != Some(true));
        let restricted = held_back && (self.ext.restricted_mode() || shared_out);
        if restricted != self.ui.restricted {
            log::info!("the status bar {} restricted mode", if restricted { "shows" } else { "no longer shows" });
            self.ui.restricted = restricted;
            self.request_redraw();
        }
        if self.ui.ask.is_some() {
            return;
        }
        if let Some(folder) = to_ask(&worked, &self.trust_asked, held_back).cloned() {
            self.trust_asked.insert(folder.root.clone());
            self.show_trust_ask(&folder.root, folder.shared);
        }
    }

    pub(super) fn trust_from_status(&mut self) {
        let worked = self.worked_in();
        if let Some(folder) = worked.iter().find(|f| f.decided != Some(true)) {
            self.show_trust_ask(&folder.root, folder.shared);
        }
    }

    fn show_trust_ask(&mut self, root: &Path, shared: bool) {
        let name =
            root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| root.display().to_string());
        let text = match shared {
            true => "It comes from the person sharing this session. Extensions are not given it until you trust it; trusted, they can run what it brings (its settings, its own TypeScript, ESLint or Prettier and their configuration files) on this computer, for as long as the session lasts.",
            false => "Until it is trusted, extensions that could run what the folder brings (its settings, its own TypeScript, ESLint or Prettier and their configuration files) stay off or limited in it. Trust it only if you trust where it comes from.",
        };
        let mut items = vec![root.display().to_string()];
        for e in ext::held_back(&self.ext.installed) {
            let how = match e.untrusted {
                ext::Untrusted::Limited(_) => "limited",
                _ => "off",
            };
            items.push(format!("{how} until trusted: {}", e.display_name));
        }
        log::info!("asking whether to trust {}", root.display());
        let ask = ui::Ask::new(
            ui::Asked::Trust { folder: root.to_path_buf(), shared },
            &format!("Trust {name}?"),
            text.to_string(),
        )
        .choice("Trust the folder", ui::ChoiceLook::Main, ui::Answer::Yes)
        .choice("Stay in restricted mode", ui::ChoiceLook::Plain, ui::Answer::Other);
        self.ui.ask = Some(ui::Ask { items, danger: false, ..ask });
        self.request_redraw();
    }

    pub(super) fn trust_answered(&mut self, folder: &Path, shared: bool, answer: ui::Answer) {
        let trusted = answer == ui::Answer::Yes;
        log::info!("{} is {}", folder.display(), if trusted { "trusted" } else { "kept in restricted mode" });
        self.trust.set(folder, shared, trusted, answer != ui::Answer::Cancel);
        self.trust_asked.insert(folder.to_path_buf());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(root: &str, shared: bool, decided: Option<bool>) -> Worked {
        Worked { root: PathBuf::from(root), shared, decided }
    }

    #[test]
    fn one_folder_not_trusted_restricts_the_host_and_shared_ones_stay_out_until_trusted() {
        let mine = folder("/a", false, Some(true));
        let new = folder("/b", false, None);
        let declined = folder("/c", false, Some(false));
        let guest = folder("/g", true, None);
        let trusted_guest = folder("/t", true, Some(true));
        assert_eq!(host_folders(std::slice::from_ref(&mine)), (vec![PathBuf::from("/a")], true));
        assert_eq!(host_folders(&[mine.clone(), new]), (vec![PathBuf::from("/a"), PathBuf::from("/b")], false));
        assert!(!host_folders(&[declined]).1);
        assert_eq!(host_folders(&[mine, guest.clone()]), (vec![PathBuf::from("/a")], true), "never given");
        assert_eq!(host_folders(&[guest]), (Vec::new(), true));
        assert_eq!(host_folders(&[trusted_guest]), (vec![PathBuf::from("/t")], true));
    }

    #[test]
    fn a_folder_is_asked_about_once_and_only_when_an_extension_is_held_back() {
        let worked = [folder("/a", false, Some(true)), folder("/b", false, None), folder("/g", true, None)];
        let mut asked = HashSet::new();
        assert_eq!(to_ask(&worked, &asked, false), None, "nothing would change");
        assert_eq!(to_ask(&worked, &asked, true).map(|f| f.root.clone()), Some(PathBuf::from("/b")));
        asked.insert(PathBuf::from("/b"));
        assert_eq!(to_ask(&worked, &asked, true).map(|f| f.root.clone()), Some(PathBuf::from("/g")));
        asked.insert(PathBuf::from("/g"));
        assert_eq!(to_ask(&worked, &asked, true), None);
        let declined = [folder("/c", false, Some(false))];
        assert_eq!(to_ask(&declined, &HashSet::new(), true), None, "a kept answer is not asked again");
    }
}

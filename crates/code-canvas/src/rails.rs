use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    pub const BOTH: [Side; 2] = [Side::Left, Side::Right];

    pub fn index(self) -> usize {
        match self {
            Side::Left => 0,
            Side::Right => 1,
        }
    }
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Rails {
    pub left: Vec<String>,
    pub right: Vec<String>,
}

impl Rails {
    fn list(&self, side: Side) -> &Vec<String> {
        match side {
            Side::Left => &self.left,
            Side::Right => &self.right,
        }
    }

    fn list_mut(&mut self, side: Side) -> &mut Vec<String> {
        match side {
            Side::Left => &mut self.left,
            Side::Right => &mut self.right,
        }
    }

    fn placed(&self, key: &str) -> bool {
        Side::BOTH.into_iter().any(|side| self.list(side).iter().any(|placed| placed == key))
    }

    pub fn arrange(&self, available: &[(String, Side)]) -> [Vec<String>; 2] {
        let mut order = [Vec::new(), Vec::new()];
        let home = |key: &str| available.iter().find(|(known, _)| known == key).map(|(_, home)| *home);
        for key in Side::BOTH.into_iter().flat_map(|side| self.list(side)) {
            let Some(home) = home(key) else { continue };
            if !order[home.index()].contains(key) {
                order[home.index()].push(key.clone());
            }
        }
        for (key, home) in available {
            if !order[home.index()].contains(key) {
                order[home.index()].push(key.clone());
            }
        }
        order
    }

    pub fn place(&mut self, available: &[(String, Side)], key: &str, before: Option<&str>) {
        let Some(home) = available.iter().find(|(known, _)| known == key).map(|(_, home)| *home) else { return };
        for (known, home) in available {
            if !self.placed(known) {
                self.list_mut(*home).push(known.clone());
            }
        }
        let order = self.arrange(available);
        for side in Side::BOTH {
            let kept: Vec<String> = self
                .list(side)
                .iter()
                .filter(|placed| !order.iter().flatten().any(|key| key == *placed))
                .cloned()
                .collect();
            *self.list_mut(side) = order[side.index()].iter().cloned().chain(kept).collect();
        }
        self.left.retain(|placed| placed != key);
        self.right.retain(|placed| placed != key);
        let list = self.list_mut(home);
        let at = before.and_then(|next| list.iter().position(|placed| placed == next)).unwrap_or(list.len());
        list.insert(at, key.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn available() -> Vec<(String, Side)> {
        [("files", Side::Left), ("search", Side::Left), ("symbols", Side::Left), ("view:chat", Side::Right)]
            .into_iter()
            .map(|(key, side)| (key.to_string(), side))
            .collect()
    }

    fn keys(list: &[&str]) -> Vec<String> {
        list.iter().map(|key| key.to_string()).collect()
    }

    #[test]
    fn what_is_the_apps_is_on_the_left_and_what_is_an_extensions_on_the_right() {
        let order = Rails::default().arrange(&available());
        assert_eq!(order, [keys(&["files", "search", "symbols"]), keys(&["view:chat"])]);
    }

    #[test]
    fn an_arrangement_saved_when_icons_could_change_sides_is_put_right() {
        let rails = Rails { left: keys(&["view:chat", "files"]), right: keys(&["symbols", "search"]) };
        assert_eq!(rails.arrange(&available()), [keys(&["files", "symbols", "search"]), keys(&["view:chat"])]);
    }

    #[test]
    fn an_icon_is_reordered_within_its_side_and_never_leaves_it() {
        let mut rails = Rails::default();
        rails.place(&available(), "search", Some("files"));
        assert_eq!(rails.arrange(&available())[0], keys(&["search", "files", "symbols"]));
        rails.place(&available(), "view:chat", Some("files"));
        assert_eq!(rails.arrange(&available()), [keys(&["search", "files", "symbols"]), keys(&["view:chat"])]);
        rails.place(&available(), "files", None);
        assert_eq!(rails.arrange(&available())[0], keys(&["search", "symbols", "files"]));
        rails.place(&available(), "unknown", None);
        assert_eq!(rails.arrange(&available())[0], keys(&["search", "symbols", "files"]));
    }

    #[test]
    fn an_icon_that_is_gone_keeps_its_place_for_when_it_returns() {
        let mut more = available();
        more.push(("view:graph".to_string(), Side::Right));
        let mut rails = Rails::default();
        rails.place(&more, "view:graph", Some("view:chat"));
        let without_graph: Vec<(String, Side)> = more.iter().filter(|(key, _)| key != "view:graph").cloned().collect();
        assert_eq!(rails.arrange(&without_graph)[1], keys(&["view:chat"]));
        assert_eq!(rails.arrange(&more)[1], keys(&["view:graph", "view:chat"]));
    }

    #[test]
    fn a_new_icon_joins_the_end_of_its_side() {
        let mut rails = Rails::default();
        rails.place(&available(), "symbols", Some("files"));
        let mut more = available();
        more.push(("deps".to_string(), Side::Left));
        assert_eq!(rails.arrange(&more)[0], keys(&["symbols", "files", "search", "deps"]));
    }

    #[test]
    fn a_damaged_file_never_shows_an_icon_twice() {
        let rails = Rails { left: keys(&["files", "symbols", "files"]), right: keys(&["symbols", "unknown"]) };
        assert_eq!(rails.arrange(&available()), [keys(&["files", "symbols", "search"]), keys(&["view:chat"])]);
    }
}

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    pub const BOTH: [Side; 2] = [Side::Left, Side::Right];

    pub fn other(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }

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
    pub homes: u32,
}

pub const HOMES: u32 = 2;

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

    pub fn at_home(&mut self, home: impl Fn(&str) -> Side) {
        if self.homes >= HOMES {
            return;
        }
        self.homes = HOMES;
        let all: Vec<String> = self.left.drain(..).chain(self.right.drain(..)).collect();
        for key in all {
            let list = self.list_mut(home(&key));
            if !list.contains(&key) {
                list.push(key);
            }
        }
    }

    pub fn side_of(&self, key: &str) -> Option<Side> {
        Side::BOTH.into_iter().find(|side| self.list(*side).iter().any(|placed| placed == key))
    }

    pub fn arrange(&self, available: &[(String, Side)]) -> [Vec<String>; 2] {
        let mut order = [Vec::new(), Vec::new()];
        for side in Side::BOTH {
            for key in self.list(side) {
                let shown = available.iter().any(|(known, _)| known == key);
                let repeated = order.iter().any(|placed: &Vec<String>| placed.contains(key));
                if shown && !repeated {
                    order[side.index()].push(key.clone());
                }
            }
        }
        for (key, home) in available {
            if self.side_of(key).is_none() && !order[home.index()].contains(key) {
                order[home.index()].push(key.clone());
            }
        }
        order
    }

    pub fn place(&mut self, available: &[(String, Side)], key: &str, side: Side, before: Option<&str>) {
        for (known, home) in available {
            if self.side_of(known).is_none() {
                self.list_mut(*home).push(known.clone());
            }
        }
        self.left.retain(|placed| placed != key);
        self.right.retain(|placed| placed != key);
        let list = self.list_mut(side);
        let at = before.and_then(|next| list.iter().position(|placed| placed == next)).unwrap_or(list.len());
        list.insert(at, key.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn available() -> Vec<(String, Side)> {
        [("files", Side::Left), ("search", Side::Left), ("symbols", Side::Right), ("view:chat", Side::Right)]
            .into_iter()
            .map(|(key, side)| (key.to_string(), side))
            .collect()
    }

    fn keys(list: &[&str]) -> Vec<String> {
        list.iter().map(|key| key.to_string()).collect()
    }

    #[test]
    fn nothing_moved_keeps_every_icon_at_home() {
        let order = Rails::default().arrange(&available());
        assert_eq!(order, [keys(&["files", "search"]), keys(&["symbols", "view:chat"])]);
    }

    #[test]
    fn an_icon_moves_to_the_other_side_and_back() {
        let mut rails = Rails::default();
        rails.place(&available(), "view:chat", Side::Left, None);
        assert_eq!(rails.arrange(&available()), [keys(&["files", "search", "view:chat"]), keys(&["symbols"])]);
        assert_eq!(rails.side_of("view:chat"), Some(Side::Left));
        rails.place(&available(), "files", Side::Right, Some("symbols"));
        assert_eq!(rails.arrange(&available()), [keys(&["search", "view:chat"]), keys(&["files", "symbols"])]);
        rails.place(&available(), "view:chat", Side::Right, None);
        assert_eq!(rails.arrange(&available()), [keys(&["search"]), keys(&["files", "symbols", "view:chat"])]);
    }

    #[test]
    fn an_icon_is_reordered_within_its_side() {
        let mut rails = Rails::default();
        rails.place(&available(), "search", Side::Left, Some("files"));
        assert_eq!(rails.arrange(&available())[0], keys(&["search", "files"]));
    }

    #[test]
    fn an_icon_that_is_gone_keeps_its_place_for_when_it_returns() {
        let mut rails = Rails::default();
        rails.place(&available(), "view:chat", Side::Left, Some("search"));
        let without_chat: Vec<(String, Side)> = available().into_iter().filter(|(key, _)| key != "view:chat").collect();
        assert_eq!(rails.arrange(&without_chat), [keys(&["files", "search"]), keys(&["symbols"])]);
        assert_eq!(rails.arrange(&available())[0], keys(&["files", "view:chat", "search"]));
    }

    #[test]
    fn a_new_icon_joins_the_end_of_its_home_side() {
        let mut rails = Rails::default();
        rails.place(&available(), "symbols", Side::Left, None);
        let mut more = available();
        more.push(("deps".to_string(), Side::Right));
        assert_eq!(rails.arrange(&more)[1], keys(&["view:chat", "deps"]));
    }

    #[test]
    fn an_arrangement_of_before_the_sides_changed_goes_home_once() {
        let home = |key: &str| if key.starts_with("view:") { Side::Right } else { Side::Left };
        let mut rails = Rails { left: keys(&["view:chat", "files"]), right: keys(&["symbols", "search"]), homes: 0 };
        rails.at_home(home);
        assert_eq!(
            (rails.left.clone(), rails.right.clone()),
            (keys(&["files", "symbols", "search"]), keys(&["view:chat"]))
        );
        rails.place(&available(), "symbols", Side::Right, None);
        rails.at_home(home);
        assert_eq!(rails.side_of("symbols"), Some(Side::Right), "what is moved after that stays where it was put");
    }

    #[test]
    fn a_damaged_file_never_shows_an_icon_twice() {
        let rails =
            Rails { left: keys(&["files", "symbols", "files"]), right: keys(&["symbols", "unknown"]), homes: 0 };
        assert_eq!(rails.arrange(&available()), [keys(&["files", "symbols", "search"]), keys(&["view:chat"])]);
    }
}

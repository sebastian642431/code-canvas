use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Part {
    #[serde(rename = "r")]
    Retain(usize),
    #[serde(rename = "i")]
    Insert(String),
    #[serde(rename = "d")]
    Delete(usize),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TextOp {
    pub parts: Vec<Part>,
}

impl TextOp {
    pub fn retain(&mut self, n: usize) {
        match self.parts.last_mut() {
            _ if n == 0 => {}
            Some(Part::Retain(last)) => *last += n,
            _ => self.parts.push(Part::Retain(n)),
        }
    }

    pub fn insert(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let at = match self.parts.last() {
            Some(Part::Delete(_)) => self.parts.len() - 1,
            _ => self.parts.len(),
        };
        match at.checked_sub(1).and_then(|before| self.parts.get_mut(before)) {
            Some(Part::Insert(last)) => last.push_str(text),
            _ => self.parts.insert(at, Part::Insert(text.to_string())),
        }
    }

    pub fn delete(&mut self, n: usize) {
        match self.parts.last_mut() {
            _ if n == 0 => {}
            Some(Part::Delete(last)) => *last += n,
            _ => self.parts.push(Part::Delete(n)),
        }
    }

    pub fn before(&self) -> usize {
        self.parts
            .iter()
            .map(|part| match part {
                Part::Retain(n) | Part::Delete(n) => *n,
                Part::Insert(_) => 0,
            })
            .sum()
    }

    #[cfg(test)]
    pub fn changes_nothing(&self) -> bool {
        self.parts.iter().all(|part| matches!(part, Part::Retain(_)))
    }

    pub fn between(old: &str, new: &str) -> TextOp {
        let (a, b): (Vec<char>, Vec<char>) = (old.chars().collect(), new.chars().collect());
        let most = a.len().min(b.len());
        let head = (0..most).take_while(|i| a[*i] == b[*i]).count();
        let tail = (0..most - head).take_while(|i| a[a.len() - 1 - i] == b[b.len() - 1 - i]).count();
        let mut op = TextOp::default();
        let (was, is) = (&a[head..a.len() - tail], &b[head..b.len() - tail]);
        if was.contains(&'\n') && is.contains(&'\n') {
            for change in similar::TextDiff::from_lines(old, new).iter_all_changes() {
                let length = change.value().chars().count();
                match change.tag() {
                    similar::ChangeTag::Equal => op.retain(length),
                    similar::ChangeTag::Delete => op.delete(length),
                    similar::ChangeTag::Insert => op.insert(change.value()),
                }
            }
            return op;
        }
        op.retain(head);
        op.insert(&is.iter().collect::<String>());
        op.delete(was.len());
        op.retain(tail);
        op
    }

    pub fn apply(&self, text: &str) -> Option<String> {
        let mut chars = text.chars();
        let mut out = String::with_capacity(text.len());
        for part in &self.parts {
            match part {
                Part::Retain(n) => {
                    for _ in 0..*n {
                        out.push(chars.next()?);
                    }
                }
                Part::Insert(text) => out.push_str(text),
                Part::Delete(n) => {
                    for _ in 0..*n {
                        chars.next()?;
                    }
                }
            }
        }
        chars.next().is_none().then_some(out)
    }

    pub fn place(&self, at: usize) -> usize {
        let (mut old, mut new) = (0, 0);
        for part in &self.parts {
            match part {
                Part::Retain(n) => {
                    if at < old + n {
                        return new + (at - old);
                    }
                    old += n;
                    new += n;
                }
                Part::Insert(text) => new += text.chars().count(),
                Part::Delete(n) => {
                    if at < old + n {
                        return new;
                    }
                    old += n;
                }
            }
        }
        new + at.saturating_sub(old)
    }

    pub fn transform(first: &TextOp, second: &TextOp) -> Option<(TextOp, TextOp)> {
        let (mut a, mut b) = (first.parts.iter().cloned(), second.parts.iter().cloned());
        let (mut one, mut two) = (TextOp::default(), TextOp::default());
        let (mut left, mut right) = (a.next(), b.next());
        loop {
            if let Some(Part::Insert(text)) = &left {
                one.insert(text);
                two.retain(text.chars().count());
                left = a.next();
                continue;
            }
            if let Some(Part::Insert(text)) = &right {
                one.retain(text.chars().count());
                two.insert(text);
                right = b.next();
                continue;
            }
            let (x, y) = match (&left, &right) {
                (None, None) => break,
                (Some(x), Some(y)) => (x.clone(), y.clone()),
                _ => return None,
            };
            let len = |part: &Part| match part {
                Part::Retain(n) | Part::Delete(n) => *n,
                Part::Insert(_) => 0,
            };
            let both = len(&x).min(len(&y));
            match (&x, &y) {
                (Part::Retain(_), Part::Retain(_)) => {
                    one.retain(both);
                    two.retain(both);
                }
                (Part::Delete(_), Part::Retain(_)) => one.delete(both),
                (Part::Retain(_), Part::Delete(_)) => two.delete(both),
                _ => {}
            }
            let rest = |part: Part, more: &mut dyn Iterator<Item = Part>| match part {
                Part::Retain(n) if n > both => Some(Part::Retain(n - both)),
                Part::Delete(n) if n > both => Some(Part::Delete(n - both)),
                _ => more.next(),
            };
            left = rest(x, &mut a);
            right = rest(y, &mut b);
        }
        Some((one, two))
    }
}

#[derive(Default)]
pub struct Link {
    pub epoch: u32,
    sent: u64,
    received: u64,
    waiting: Vec<(u64, TextOp)>,
}

impl Link {
    pub fn fresh(epoch: u32) -> Link {
        Link { epoch, ..Default::default() }
    }

    pub fn send(&mut self, op: &TextOp) -> (u64, u64) {
        self.waiting.push((self.sent, op.clone()));
        self.sent += 1;
        (self.sent - 1, self.received)
    }

    pub fn receive(&mut self, mut op: TextOp, theirs: u64, mine_seen: u64, i_go_first: bool) -> Option<TextOp> {
        if theirs != self.received {
            return None;
        }
        self.waiting.retain(|(number, _)| *number >= mine_seen);
        for (_, waiting) in &mut self.waiting {
            let (ours, incoming) = match i_go_first {
                true => TextOp::transform(waiting, &op)?,
                false => {
                    let (incoming, ours) = TextOp::transform(&op, waiting)?;
                    (ours, incoming)
                }
            };
            *waiting = ours;
            op = incoming;
        }
        self.received += 1;
        Some(op)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(parts: &[Part]) -> TextOp {
        TextOp { parts: parts.to_vec() }
    }

    #[test]
    fn what_changed_between_two_texts_is_one_edit() {
        let made = TextOp::between("let a = 1;\nlet b = 2;", "let a = 10;\nlet b = 2;");
        assert_eq!(made, op(&[Part::Retain(9), Part::Insert("0".into()), Part::Retain(12)]));
        assert_eq!(made.apply("let a = 1;\nlet b = 2;").as_deref(), Some("let a = 10;\nlet b = 2;"));
        assert!(TextOp::between("same", "same").changes_nothing());
        let swap = TextOp::between("añb", "añ€b");
        assert_eq!(swap.apply("añb").as_deref(), Some("añ€b"), "counted in characters, not bytes");
        assert_eq!(TextOp::between("abc", "").apply("abc").as_deref(), Some(""));
        assert!(made.apply("short").is_none(), "an edit for another text is refused");
        let (was, is) = ("a\nb\nc\nd\ne\n", "A\nb\nc\nd\nE\n");
        let far = TextOp::between(was, is);
        assert_eq!(far.apply(was).as_deref(), Some(is));
        assert_eq!(far.place(4), 4, "changes at both ends of a file leave its middle alone");
        assert!(far.parts.len() >= 5, "{far:?}");
    }

    #[test]
    fn a_place_moves_with_the_edits_before_it() {
        let made = TextOp::between("hello world", "hello big world");
        assert_eq!(made.place(0), 0);
        assert_eq!(made.place(8), 12);
        let cut = TextOp::between("hello big world", "hello world");
        assert_eq!(cut.place(8), 6, "a place inside what was removed falls to where it was");
        assert_eq!(cut.place(15), 11);
    }

    #[test]
    fn two_edits_made_apart_end_in_the_same_text() {
        let base = "fn main() {\n    run();\n}\n";
        let mine = TextOp::between(base, "fn main() {\n    run(1);\n}\n");
        let theirs = TextOp::between(base, "// start\nfn main() {\n    run();\n}\n");
        let (mine_after, theirs_after) = TextOp::transform(&mine, &theirs).unwrap();
        let one = theirs_after.apply(&mine.apply(base).unwrap()).unwrap();
        let two = mine_after.apply(&theirs.apply(base).unwrap()).unwrap();
        assert_eq!(one, two);
        assert_eq!(one, "// start\nfn main() {\n    run(1);\n}\n");

        let a = TextOp::between("abc", "aXbc");
        let b = TextOp::between("abc", "aYbc");
        let (a2, b2) = TextOp::transform(&a, &b).unwrap();
        assert_eq!(b2.apply(&a.apply("abc").unwrap()).unwrap(), "aXYbc", "at the same place the first one goes first");
        assert_eq!(a2.apply(&b.apply("abc").unwrap()).unwrap(), "aXYbc");

        let cut = TextOp::between("abcdef", "af");
        let inside = TextOp::between("abcdef", "abcXdef");
        let (cut2, inside2) = TextOp::transform(&cut, &inside).unwrap();
        assert_eq!(inside2.apply(&cut.apply("abcdef").unwrap()), cut2.apply(&inside.apply("abcdef").unwrap()));
        assert!(TextOp::transform(&a, &TextOp::between("other text", "x")).is_none());
    }

    fn next(seed: &mut u64) -> usize {
        *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (*seed >> 33) as usize
    }

    fn scribble(text: &str, seed: &mut u64) -> String {
        let mut chars: Vec<char> = text.chars().collect();
        let at = next(seed) % (chars.len() + 1);
        match next(seed) % 3 {
            0 => {
                let cut = (next(seed) % 4).min(chars.len() - at);
                chars.drain(at..at + cut);
            }
            _ => {
                let word = ["x", "yz", "\n", "ñ€", "fn "][next(seed) % 5];
                chars.splice(at..at, word.chars());
            }
        }
        chars.into_iter().collect()
    }

    #[test]
    fn two_people_typing_at_once_agree_whatever_the_order_of_arrival() {
        for round in 0..300u64 {
            let mut seed = round * 7919 + 1;
            let mut host = String::from("fn main() {\n    println!(\"hi\");\n}\n");
            let mut guest = host.clone();
            let (mut at_host, mut at_guest) = (Link::default(), Link::default());
            let (mut to_guest, mut to_host): (Vec<(TextOp, u64, u64)>, Vec<(TextOp, u64, u64)>) =
                (Vec::new(), Vec::new());
            for _ in 0..40 {
                match next(&mut seed) % 4 {
                    0 => {
                        let after = scribble(&host, &mut seed);
                        let made = TextOp::between(&host, &after);
                        host = after;
                        let (mine, theirs) = at_host.send(&made);
                        to_guest.push((made, mine, theirs));
                    }
                    1 => {
                        let after = scribble(&guest, &mut seed);
                        let made = TextOp::between(&guest, &after);
                        guest = after;
                        let (mine, theirs) = at_guest.send(&made);
                        to_host.push((made, mine, theirs));
                    }
                    2 if !to_guest.is_empty() => {
                        let (made, number, seen) = to_guest.remove(0);
                        let made = at_guest.receive(made, number, seen, false).expect("in order");
                        guest = made.apply(&guest).expect("fits the guest's text");
                    }
                    _ if !to_host.is_empty() => {
                        let (made, number, seen) = to_host.remove(0);
                        let made = at_host.receive(made, number, seen, true).expect("in order");
                        host = made.apply(&host).expect("fits the host's text");
                    }
                    _ => {}
                }
            }
            for (made, number, seen) in to_guest.drain(..) {
                guest = at_guest.receive(made, number, seen, false).unwrap().apply(&guest).unwrap();
            }
            for (made, number, seen) in to_host.drain(..) {
                host = at_host.receive(made, number, seen, true).unwrap().apply(&host).unwrap();
            }
            assert_eq!(host, guest, "round {round}");
        }
    }
}

//! What the registry tells about extensions beyond a search: icons, the page of one
//! extension, its README, changelog and manifest. Each is asked for once a session, on a
//! few threads at a time, and kept; nothing here waits on the network.

use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

use serde_json::Value;

use super::compat::{self, Gap};
use super::market::{self, MarketDetails};
use super::Waker;

/// How many answers are awaited at once.
const AT_ONCE: usize = 6;

#[derive(Clone, Debug, PartialEq)]
pub enum Got<T> {
    Loading,
    Ready(T),
    Failed(String),
}

impl<T> Got<T> {
    pub fn ready(&self) -> Option<&T> {
        match self {
            Got::Ready(t) => Some(t),
            _ => None,
        }
    }

    /// A short word for what state it is in, to tell when a page made of it must be made again.
    pub fn stage(&self) -> &'static str {
        match self {
            Got::Loading => "loading",
            Got::Ready(_) => "ready",
            Got::Failed(_) => "failed",
        }
    }
}

/// A manifest and what of it does not work in Hawk Canvas.
#[derive(Debug, PartialEq)]
pub struct Manifest {
    pub json: Value,
    pub gaps: Vec<Gap>,
}

impl Manifest {
    pub fn new(json: Value) -> Self {
        let gaps = compat::gaps(&json);
        Manifest { json, gaps }
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum Key {
    Icon(String),
    Manifest(String),
    Details(String),
    Text(String),
    Released(String),
}

enum Payload {
    Icon(egui::ColorImage),
    Manifest(Value),
    Details(Box<MarketDetails>),
    Text(String),
    Released(String),
}

type Work = Box<dyn FnOnce() -> Result<Payload, String> + Send>;

pub struct Gallery {
    icons: HashMap<String, Got<Arc<egui::ColorImage>>>,
    manifests: HashMap<String, Got<Arc<Manifest>>>,
    details: HashMap<String, Got<Arc<MarketDetails>>>,
    texts: HashMap<String, Got<Arc<String>>>,
    released: HashMap<String, Got<String>>,
    queue: VecDeque<(Key, Work)>,
    running: usize,
    tx: Sender<(Key, Result<Payload, String>)>,
    rx: Receiver<(Key, Result<Payload, String>)>,
    waker: Waker,
}

fn id_of(namespace: &str, name: &str) -> String {
    format!("{namespace}.{name}").to_lowercase()
}

impl Gallery {
    pub fn new(waker: Waker) -> Self {
        let (tx, rx) = mpsc::channel();
        Gallery {
            icons: HashMap::new(),
            manifests: HashMap::new(),
            details: HashMap::new(),
            texts: HashMap::new(),
            released: HashMap::new(),
            queue: VecDeque::new(),
            running: 0,
            tx,
            rx,
            waker,
        }
    }

    fn ask(&mut self, key: Key, work: Work) {
        self.queue.push_back((key, work));
        self.pump();
    }

    fn pump(&mut self) {
        while self.running < AT_ONCE {
            let Some((key, work)) = self.queue.pop_front() else { break };
            self.running += 1;
            let (tx, waker) = (self.tx.clone(), self.waker.clone());
            std::thread::spawn(move || {
                let _ = tx.send((key, work()));
                waker.wake();
            });
        }
    }

    /// Takes what has arrived; true when anything did.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok((key, got)) = self.rx.try_recv() {
            changed = true;
            self.running = self.running.saturating_sub(1);
            match (key, got) {
                (Key::Icon(url), Ok(Payload::Icon(picture))) => {
                    self.icons.insert(url, Got::Ready(Arc::new(picture)));
                }
                (Key::Manifest(url), Ok(Payload::Manifest(json))) => {
                    self.manifests.insert(url, Got::Ready(Arc::new(Manifest::new(json))));
                }
                (Key::Details(id), Ok(Payload::Details(d))) => {
                    self.details.insert(id, Got::Ready(Arc::new(*d)));
                }
                (Key::Text(url), Ok(Payload::Text(text))) => {
                    self.texts.insert(url, Got::Ready(Arc::new(text)));
                }
                (Key::Released(id), Ok(Payload::Released(at))) => {
                    self.released.insert(id, Got::Ready(at));
                }
                (key, Err(why)) => self.failed(key, why),
                (key, Ok(_)) => self.failed(key, "an answer of the wrong kind".into()),
            }
        }
        self.pump();
        changed
    }

    fn failed(&mut self, key: Key, why: String) {
        match key {
            Key::Icon(url) => {
                self.icons.insert(url, Got::Failed(why));
            }
            Key::Manifest(url) => {
                self.manifests.insert(url, Got::Failed(why));
            }
            Key::Details(id) => {
                self.details.insert(id, Got::Failed(why));
            }
            Key::Text(url) => {
                self.texts.insert(url, Got::Failed(why));
            }
            Key::Released(id) => {
                self.released.insert(id, Got::Failed(why));
            }
        }
    }

    /// Lets what failed be asked for again (a new search: the network may be back).
    pub fn forget_failures(&mut self) {
        self.icons.retain(|_, got| !matches!(got, Got::Failed(_)));
        self.manifests.retain(|_, got| !matches!(got, Got::Failed(_)));
        self.details.retain(|_, got| !matches!(got, Got::Failed(_)));
        self.texts.retain(|_, got| !matches!(got, Got::Failed(_)));
        self.released.retain(|_, got| !matches!(got, Got::Failed(_)));
    }

    /// The icon at that address, once it has arrived.
    pub fn icon(&mut self, url: &str) -> Option<Arc<egui::ColorImage>> {
        if url.is_empty() {
            return None;
        }
        if !self.icons.contains_key(url) {
            self.icons.insert(url.to_string(), Got::Loading);
            let wanted = url.to_string();
            self.ask(Key::Icon(url.to_string()), Box::new(move || market::fetch_icon(&wanted).map(Payload::Icon)));
        }
        self.icons.get(url).and_then(Got::ready).cloned()
    }

    /// The manifest at that address, and what of it does not work.
    pub fn manifest(&mut self, url: &str) -> Got<Arc<Manifest>> {
        if url.is_empty() {
            return Got::Failed("the registry keeps no manifest of it".into());
        }
        if !self.manifests.contains_key(url) {
            self.manifests.insert(url.to_string(), Got::Loading);
            let wanted = url.to_string();
            let work = move || market::fetch_json(&wanted).map(Payload::Manifest);
            self.ask(Key::Manifest(url.to_string()), Box::new(work));
        }
        self.manifests.get(url).cloned().unwrap_or(Got::Loading)
    }

    /// The manifest of a version of an extension: `manifest` at the registry's usual address.
    pub fn manifest_of(&mut self, namespace: &str, name: &str, version: &str) -> Got<Arc<Manifest>> {
        self.manifest(&market::manifest_url(namespace, name, version))
    }

    /// The registry's page of an extension.
    pub fn details(&mut self, namespace: &str, name: &str) -> Got<Arc<MarketDetails>> {
        let id = id_of(namespace, name);
        if !self.details.contains_key(&id) {
            self.details.insert(id.clone(), Got::Loading);
            let (namespace, name) = (namespace.to_string(), name.to_string());
            let work = move || market::details(&namespace, &name).map(|d| Payload::Details(Box::new(d)));
            self.ask(Key::Details(id.clone()), Box::new(work));
        }
        self.details.get(&id).cloned().unwrap_or(Got::Loading)
    }

    /// A text file of the registry (a README, a changelog).
    pub fn text(&mut self, url: &str) -> Got<Arc<String>> {
        if url.is_empty() {
            return Got::Failed("none".into());
        }
        if !self.texts.contains_key(url) {
            self.texts.insert(url.to_string(), Got::Loading);
            let wanted = url.to_string();
            self.ask(Key::Text(url.to_string()), Box::new(move || market::fetch_text(&wanted).map(Payload::Text)));
        }
        self.texts.get(url).cloned().unwrap_or(Got::Loading)
    }

    /// When its first version on the registry came out.
    pub fn first_released(&mut self, namespace: &str, name: &str) -> Got<String> {
        let id = id_of(namespace, name);
        if !self.released.contains_key(&id) {
            self.released.insert(id.clone(), Got::Loading);
            let (namespace, name) = (namespace.to_string(), name.to_string());
            let work = move || market::first_released(&namespace, &name).map(Payload::Released);
            self.ask(Key::Released(id.clone()), Box::new(work));
        }
        self.released.get(&id).cloned().unwrap_or(Got::Loading)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asks_a_few_at_a_time_and_keeps_answers() {
        let mut g = Gallery::new(Waker::new(|| {}));
        let (gate_tx, gate_rx) = mpsc::channel::<()>();
        let gate = Arc::new(std::sync::Mutex::new(gate_rx));
        for i in 0..(AT_ONCE + 3) {
            let gate = gate.clone();
            let work: Work = Box::new(move || {
                let _ = gate.lock().unwrap().recv();
                Ok(Payload::Text(format!("text {i}")))
            });
            g.texts.insert(format!("u{i}"), Got::Loading);
            g.ask(Key::Text(format!("u{i}")), work);
        }
        assert_eq!((g.running, g.queue.len()), (AT_ONCE, 3));
        for _ in 0..(AT_ONCE + 3) {
            gate_tx.send(()).unwrap();
        }
        let started = std::time::Instant::now();
        while g.texts.values().any(|t| *t == Got::Loading) && started.elapsed().as_secs() < 10 {
            g.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(g.running, 0);
        assert_eq!(g.text("u2"), Got::Ready(Arc::new("text 2".to_string())));
        g.failed(Key::Text("u1".into()), "offline".into());
        g.forget_failures();
        assert!(!g.texts.contains_key("u1"));
    }
}

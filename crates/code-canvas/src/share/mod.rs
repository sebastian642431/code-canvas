pub mod ot;
pub mod wire;

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ot::{Link, TextOp};
use wire::Msg;
pub use wire::{Caret, Invite, Person, Seen};

pub const HOST: u32 = 0;
pub const COLORS: [u32; 8] =
    [0xff6b9dff, 0x4fd6beff, 0xffb454ff, 0xa78bfaff, 0x5ab0ffff, 0xf7768eff, 0x9ece6aff, 0xe0af68ff];
const GREETING: Duration = Duration::from_secs(10);
const STUCK: Duration = Duration::from_secs(20);
pub const MOST_TEXT: usize = 96 << 20;
pub const LARGEST_FILE: usize = 2 << 20;
pub const MOST_FILES: usize = 20_000;
const MOST_WAITING: usize = MOST_TEXT * 2 + (32 << 20);
const MOST_KNOCKING: usize = 24;

pub fn body(text: &str) -> String {
    let text = text.strip_suffix('\n').unwrap_or(text);
    let text = text.strip_suffix('\r').unwrap_or(text);
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn weight(msg: &Msg) -> usize {
    let texts = match msg {
        Msg::File { text, path, .. } | Msg::Make { text, path, .. } => text.len() + path.len(),
        Msg::Edit { op, .. } => {
            let inserted = |part: &ot::Part| match part {
                ot::Part::Insert(text) => text.len(),
                _ => 0,
            };
            op.parts.iter().map(inserted).sum::<usize>() + op.parts.len() * 16
        }
        _ => 0,
    };
    texts + 128
}

struct Hurried<'a> {
    stream: &'a mut TcpStream,
    until: Instant,
}

impl Read for Hurried<'_> {
    fn read(&mut self, into: &mut [u8]) -> io::Result<usize> {
        let left = self.until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "it took too long to say hello"));
        }
        self.stream.set_read_timeout(Some(left))?;
        self.stream.read(into)
    }
}

impl Write for Hurried<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.stream.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

pub fn local_address() -> String {
    let asked = UdpSocket::bind("0.0.0.0:0").and_then(|socket| {
        socket.connect("192.0.2.1:9")?;
        socket.local_addr()
    });
    asked.map_or("127.0.0.1".to_string(), |address| address.ip().to_string())
}

enum Net {
    Open(u32, Sender<Option<Msg>>, Arc<AtomicUsize>),
    Got(u32, Msg),
    Closed(u32, String),
}

struct Conn {
    id: u32,
    out: Sender<Option<Msg>>,
    waiting: Arc<AtomicUsize>,
    greeted: bool,
}

impl Conn {
    fn send(&self, msg: Msg) {
        let size = weight(&msg);
        if self.waiting.fetch_add(size, Ordering::Relaxed) + size > MOST_WAITING {
            log::warn!("share: someone does not take what is sent: their place is closed");
            let _ = self.out.send(None);
            return;
        }
        let _ = self.out.send(Some(msg));
    }
}

struct Doc {
    text: String,
    epoch: u32,
    links: HashMap<u32, Link>,
}

#[derive(Debug, PartialEq)]
pub enum Event {
    Joined(u32),
    Left(String),
    Folder { id: u32, name: String },
    File { folder: u32, path: String, text: String, again: bool },
    FolderReady(u32),
    FolderGone(u32),
    Gone { folder: u32, path: String },
    Edit { folder: u32, path: String, op: TextOp, by: u32 },
    Save { folder: u32, path: String, by: u32 },
    Make { folder: u32, path: String, text: String, by: u32 },
    Drop { folder: u32, path: String, by: u32 },
    Role(bool),
    Come(u32),
    Ended(String),
}

pub struct Session {
    pub host: bool,
    pub me: u32,
    pub people: Vec<Person>,
    pub seen: HashMap<u32, Seen>,
    pub invite: Option<Invite>,
    pub guests_write: bool,
    pub connected: bool,
    pub ended: Option<String>,
    conns: Vec<Conn>,
    net: Receiver<Net>,
    stop: Arc<AtomicBool>,
    docs: HashMap<(u32, String), Doc>,
    folders: Vec<(u32, String)>,
    colors: u8,
    epochs: u32,
    taken: usize,
    secret: Arc<Mutex<[u8; 16]>>,
}

fn serve(stream: TcpStream, id: u32, secret: [u8; 16], hello: Option<Msg>, net: Sender<Net>) {
    let tell = net.clone();
    let run = move || -> io::Result<()> {
        let mut stream = stream;
        stream.set_nodelay(true)?;
        stream.set_write_timeout(Some(STUCK))?;
        let (first, mut seal_out, mut seal_in) = {
            let mut hurried = Hurried { stream: &mut stream, until: Instant::now() + GREETING };
            match hello {
                Some(hello) => {
                    let (mut seal_out, seal_in) = wire::greet_as_guest(&mut hurried, &secret)?;
                    wire::write_msg(&mut hurried, &mut seal_out, &hello)?;
                    (None, seal_out, seal_in)
                }
                None => {
                    let (seal_out, mut seal_in) = wire::greet_as_host(&mut hurried, &secret)?;
                    let first = wire::read_msg_within(&mut hurried, &mut seal_in, wire::FIRST)?;
                    (Some(first), seal_out, seal_in)
                }
            }
        };
        stream.set_read_timeout(None)?;
        let (out, queued) = mpsc::channel::<Option<Msg>>();
        let waiting = Arc::new(AtomicUsize::new(0));
        let _ = net.send(Net::Open(id, out, waiting.clone()));
        if let Some(first) = first {
            let _ = net.send(Net::Got(id, first));
        }
        crate::ext::wake_ui();
        let mut writing = stream.try_clone()?;
        std::thread::spawn(move || {
            while let Ok(Some(msg)) = queued.recv() {
                let size = weight(&msg);
                let sent = wire::write_msg(&mut writing, &mut seal_out, &msg);
                waiting.fetch_sub(size.min(waiting.load(Ordering::Relaxed)), Ordering::Relaxed);
                if sent.is_err() {
                    break;
                }
            }
            let _ = writing.shutdown(Shutdown::Both);
        });
        loop {
            let msg = wire::read_msg(&mut stream, &mut seal_in)?;
            if net.send(Net::Got(id, msg)).is_err() {
                return Ok(());
            }
            crate::ext::wake_ui();
        }
    };
    let why = match run() {
        Ok(()) => String::new(),
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => "the connection was closed".to_string(),
        Err(e) => e.to_string(),
    };
    let _ = tell.send(Net::Closed(id, why));
    crate::ext::wake_ui();
}

impl Session {
    fn new(host: bool, net: Receiver<Net>, stop: Arc<AtomicBool>) -> Session {
        Session {
            host,
            me: HOST,
            people: Vec::new(),
            seen: HashMap::new(),
            invite: None,
            guests_write: false,
            connected: host,
            ended: None,
            conns: Vec::new(),
            net,
            stop,
            docs: HashMap::new(),
            folders: Vec::new(),
            colors: 1,
            epochs: 0,
            taken: 0,
            secret: Arc::new(Mutex::new([0; 16])),
        }
    }

    fn epoch(&mut self) -> u32 {
        self.epochs += 1;
        self.epochs
    }

    pub fn open(name: &str, guests_write: bool) -> io::Result<Session> {
        let only = std::env::var("CODE_CANVAS_SHARE_ON").unwrap_or_else(|_| "0.0.0.0".to_string());
        Session::open_on(&only, name, guests_write)
    }

    pub fn open_on(address: &str, name: &str, guests_write: bool) -> io::Result<Session> {
        let listener = TcpListener::bind((address, wire::DEFAULT_PORT)).or_else(|_| TcpListener::bind((address, 0)))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let secret = wire::random::<16>();
        let shared = Arc::new(Mutex::new(secret));
        let (tx, net) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let (stopped, ids, asked) = (stop.clone(), AtomicU32::new(1), shared.clone());
        let knocking = Arc::new(AtomicUsize::new(0));
        std::thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        if knocking.load(Ordering::Relaxed) >= MOST_KNOCKING {
                            continue;
                        }
                        knocking.fetch_add(1, Ordering::Relaxed);
                        let (id, tx, here) = (ids.fetch_add(1, Ordering::Relaxed), tx.clone(), knocking.clone());
                        let secret = *asked.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                        let _ = stream.set_nonblocking(false);
                        std::thread::spawn(move || {
                            serve(stream, id, secret, None, tx);
                            here.fetch_sub(1, Ordering::Relaxed);
                        });
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(60)),
                }
            }
        });
        let mut session = Session::new(true, net, stop);
        session.guests_write = guests_write;
        session.secret = shared;
        let host = if address == "0.0.0.0" { local_address() } else { address.to_string() };
        session.invite = Some(Invite { host, port, secret });
        session.people = vec![Person { id: HOST, name: name.to_string(), color: 0, writer: true, host: true }];
        Ok(session)
    }

    pub fn join(invite: &Invite, name: &str) -> Session {
        let (tx, net) = mpsc::channel();
        let (invite, hello) = (invite.clone(), Msg::Hello { name: name.to_string(), version: wire::VERSION });
        std::thread::spawn(move || {
            let found = (invite.host.as_str(), invite.port).to_socket_addrs().ok().and_then(|mut all| all.next());
            let stream = match found {
                Some(address) => TcpStream::connect_timeout(&address, GREETING),
                None => Err(io::Error::new(io::ErrorKind::NotFound, format!("{} is not an address", invite.host))),
            };
            match stream {
                Ok(stream) => serve(stream, HOST, invite.secret, Some(hello), tx),
                Err(e) => {
                    let _ = tx.send(Net::Closed(HOST, format!("it could not be reached: {e}")));
                    crate::ext::wake_ui();
                }
            }
        });
        Session::new(false, net, Arc::new(AtomicBool::new(false)))
    }

    pub fn person(&self, id: u32) -> Option<&Person> {
        self.people.iter().find(|person| person.id == id)
    }

    pub fn writer(&self) -> bool {
        self.host || self.person(self.me).is_some_and(|me| me.writer)
    }

    pub fn others(&self) -> impl Iterator<Item = &Person> {
        self.people.iter().filter(|person| person.id != self.me)
    }

    #[cfg(test)]
    pub fn has(&self, folder: u32, path: &str) -> bool {
        self.docs.contains_key(&(folder, path.to_string()))
    }

    pub fn shares(&self, folder: u32) -> bool {
        self.folders.iter().any(|(id, _)| *id == folder)
    }

    pub fn text(&self, folder: u32, path: &str) -> Option<&str> {
        self.docs.get(&(folder, path.to_string())).map(|doc| doc.text.as_str())
    }

    pub fn folder_ids(&self) -> Vec<u32> {
        self.folders.iter().map(|(id, _)| *id).collect()
    }

    pub fn paths(&self, folder: u32) -> Vec<String> {
        self.docs.keys().filter(|(of, _)| *of == folder).map(|(_, path)| path.clone()).collect()
    }

    pub fn text_kept(&self) -> usize {
        self.docs.values().map(|doc| doc.text.len()).sum()
    }

    fn to(&self, id: u32, msg: Msg) {
        if let Some(conn) = self.conns.iter().find(|conn| conn.id == id) {
            conn.send(msg);
        }
    }

    fn to_all(&self, msg: &Msg, but: Option<u32>) {
        for conn in self.conns.iter().filter(|conn| conn.greeted && Some(conn.id) != but) {
            conn.send(msg.clone());
        }
    }

    fn tell_people(&self) {
        self.to_all(&Msg::People { list: self.people.clone() }, None);
    }

    pub fn share_folder(&mut self, id: u32, name: &str, files: Vec<(String, String)>) {
        self.folders.push((id, name.to_string()));
        self.to_all(&Msg::Folder { id, name: name.to_string(), files: files.len() as u32 }, None);
        for (path, text) in files {
            let epoch = self.epoch();
            self.to_all(&Msg::File { folder: id, path: path.clone(), text: text.clone(), epoch }, None);
            self.docs.insert((id, path), Doc { text, epoch, links: HashMap::new() });
        }
        self.to_all(&Msg::FolderReady { id }, None);
    }

    pub fn unshare_folder(&mut self, id: u32) {
        self.folders.retain(|(folder, _)| *folder != id);
        self.docs.retain(|(folder, _), _| *folder != id);
        self.to_all(&Msg::FolderGone { id }, None);
    }

    fn welcome(&mut self, id: u32) {
        self.to(id, Msg::Welcome { you: id });
        for (folder, name) in self.folders.clone() {
            let mut files: Vec<(&String, &mut Doc)> =
                self.docs.iter_mut().filter(|((of, _), _)| *of == folder).map(|((_, path), doc)| (path, doc)).collect();
            files.sort_by(|a, b| a.0.cmp(b.0));
            let mut said = vec![Msg::Folder { id: folder, name, files: files.len() as u32 }];
            for (path, doc) in files {
                doc.links.insert(id, Link::fresh(doc.epoch));
                said.push(Msg::File { folder, path: path.clone(), text: doc.text.clone(), epoch: doc.epoch });
            }
            said.push(Msg::FolderReady { id: folder });
            for msg in said {
                self.to(id, msg);
            }
        }
    }

    pub fn changed(&mut self, folder: u32, path: &str, text: &str) {
        let key = (folder, path.to_string());
        let Some(doc) = self.docs.get_mut(&key) else {
            if self.host && self.shares(folder) {
                let epoch = self.epoch();
                self.docs.insert(key, Doc { text: text.to_string(), epoch, links: HashMap::new() });
                let (path, text) = (path.to_string(), text.to_string());
                self.to_all(&Msg::File { folder, path, text, epoch }, None);
            }
            return;
        };
        if doc.text == text {
            return;
        }
        let op = TextOp::between(&doc.text, text);
        doc.text = text.to_string();
        let mut said = Vec::new();
        for conn in self.conns.iter().filter(|conn| conn.greeted) {
            let link = doc.links.entry(conn.id).or_insert_with(|| Link::fresh(doc.epoch));
            let (number, seen) = link.send(&op);
            said.push((
                conn.id,
                Msg::Edit { folder, path: path.to_string(), epoch: link.epoch, number, seen, op: op.clone() },
            ));
        }
        for (id, msg) in said {
            self.to(id, msg);
        }
    }

    pub fn removed(&mut self, folder: u32, path: &str) {
        if self.host && self.docs.remove(&(folder, path.to_string())).is_some() {
            self.to_all(&Msg::Gone { folder, path: path.to_string() }, None);
        }
    }

    pub fn saved(&mut self, folder: u32, path: &str) {
        if !self.host {
            self.to(HOST, Msg::Save { folder, path: path.to_string() });
        }
    }

    pub fn made(&mut self, folder: u32, path: &str, text: &str) {
        if !self.host && self.writer() {
            self.to(HOST, Msg::Make { folder, path: path.to_string(), text: text.to_string() });
        }
    }

    pub fn dropped(&mut self, folder: u32, path: &str) {
        if !self.host && self.writer() {
            self.to(HOST, Msg::Drop { folder, path: path.to_string() });
        }
    }

    fn again(&mut self, id: u32, folder: u32, path: &str) {
        let Some(doc) = self.docs.get_mut(&(folder, path.to_string())) else { return };
        let epoch = doc.links.get(&id).map_or(doc.epoch, |link| link.epoch) + 1;
        doc.links.insert(id, Link::fresh(epoch));
        let msg = Msg::File { folder, path: path.to_string(), text: doc.text.clone(), epoch };
        self.to(id, msg);
    }

    fn edit(&mut self, from: u32, folder: u32, path: String, at: (u32, u64, u64), op: TextOp) -> Option<Event> {
        let (epoch, number, seen) = at;
        let (host, allowed) = (self.host, !self.host || self.person(from).is_some_and(|person| person.writer));
        let key = (folder, path.clone());
        let doc = self.docs.get_mut(&key)?;
        let link = doc.links.entry(from).or_insert_with(|| Link::fresh(doc.epoch));
        if link.epoch != epoch {
            return None;
        }
        let carries = |part: &ot::Part| matches!(part, ot::Part::Insert(text) if text.contains('\r'));
        let allowed = allowed && !op.parts.iter().any(carries);
        let fitted = if allowed { link.receive(op, number, seen, host) } else { None };
        let applied = fitted.and_then(|op| Some((op.apply(&doc.text)?, op)));
        let Some((text, op)) = applied else {
            match host {
                true => self.again(from, folder, &path),
                false => self.to(HOST, Msg::Again { folder, path }),
            }
            return None;
        };
        doc.text = text;
        let said = link.owed().map(|seen| Msg::Said { folder, path: path.clone(), epoch, seen });
        if let Some(said) = said {
            self.to(from, said);
        }
        let doc = self.docs.get_mut(&key)?;
        let mut said = Vec::new();
        for conn in self.conns.iter().filter(|conn| conn.greeted && conn.id != from) {
            let link = doc.links.entry(conn.id).or_insert_with(|| Link::fresh(doc.epoch));
            let (number, seen) = link.send(&op);
            said.push((
                conn.id,
                Msg::Edit { folder, path: path.clone(), epoch: link.epoch, number, seen, op: op.clone() },
            ));
        }
        for (id, msg) in said {
            self.to(id, msg);
        }
        Some(Event::Edit { folder, path, op, by: from })
    }

    pub fn see(&mut self, at: Seen) {
        let msg = Msg::Seen { who: self.me, at };
        match self.host {
            true => self.to_all(&msg, None),
            false => self.to(HOST, msg),
        }
    }

    pub fn call_everyone(&mut self) {
        let msg = Msg::Come { who: self.me };
        match self.host {
            true => self.to_all(&msg, None),
            false => self.to(HOST, msg),
        }
    }

    pub fn set_writer(&mut self, id: u32, writer: bool) {
        if let Some(person) = self.people.iter_mut().find(|person| person.id == id && !person.host) {
            person.writer = writer;
            self.tell_people();
        }
    }

    pub fn send_away(&mut self, id: u32) {
        if let Some(conn) = self.conns.iter().find(|conn| conn.id == id) {
            let _ = conn.out.send(Some(Msg::Bye { why: "the host closed your place in the session".to_string() }));
            let _ = conn.out.send(None);
        }
        if let Some(invite) = self.invite.as_mut().filter(|_| self.host) {
            invite.secret = wire::random::<16>();
            *self.secret.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = invite.secret;
        }
    }

    fn give_up(&mut self, why: &str, events: &mut Vec<Event>) {
        for conn in self.conns.drain(..) {
            let _ = conn.out.send(None);
        }
        if self.ended.is_none() {
            self.ended = Some(why.to_string());
            events.push(Event::Ended(why.to_string()));
        }
    }

    pub fn close(&mut self, why: &str) {
        self.stop.store(true, Ordering::Relaxed);
        for conn in self.conns.drain(..) {
            let _ = conn.out.send(Some(Msg::Bye { why: why.to_string() }));
            let _ = conn.out.send(None);
        }
        if self.ended.is_none() {
            self.ended = Some(String::new());
        }
    }

    fn gone(&mut self, id: u32, why: String, events: &mut Vec<Event>) {
        let known = self.conns.iter().any(|conn| conn.id == id);
        self.conns.retain(|conn| conn.id != id);
        if !self.host {
            if self.ended.is_none() {
                let why = if why.is_empty() { "the session ended".to_string() } else { why };
                self.ended = Some(why.clone());
                events.push(Event::Ended(why));
            }
            return;
        }
        let Some(at) = self.people.iter().position(|person| person.id == id) else {
            if known && !why.is_empty() {
                log::info!("share: someone did not get in: {why}");
            }
            return;
        };
        let person = self.people.remove(at);
        self.seen.remove(&id);
        for doc in self.docs.values_mut() {
            doc.links.remove(&id);
        }
        self.tell_people();
        events.push(Event::Left(person.name));
    }

    pub fn poll(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        while let Ok(net) = self.net.try_recv() {
            match net {
                Net::Open(id, out, waiting) => {
                    self.conns.push(Conn { id, out, waiting, greeted: !self.host });
                }
                Net::Closed(id, why) => self.gone(id, why, &mut events),
                Net::Got(id, msg) => self.got(id, msg, &mut events),
            }
        }
        events
    }

    fn got(&mut self, from: u32, msg: Msg, events: &mut Vec<Event>) {
        let greeted = self.conns.iter().any(|conn| conn.id == from && conn.greeted);
        match msg {
            Msg::Hello { name, version } if self.host && !greeted => {
                if version != wire::VERSION {
                    let why = "this session is of another version of Code Canvas".to_string();
                    self.to(from, Msg::Bye { why });
                    return self.send_away(from);
                }
                let name = match name.trim() {
                    "" => format!("Guest {from}"),
                    name => name.chars().take(40).collect(),
                };
                let taken = self.people.iter().filter(|person| person.name == name).count();
                let name = if taken > 0 { format!("{name} ({})", taken + 1) } else { name };
                let color = self.colors % COLORS.len() as u8;
                self.colors = self.colors.wrapping_add(1);
                self.people.push(Person { id: from, name, color, writer: self.guests_write, host: false });
                if let Some(conn) = self.conns.iter_mut().find(|conn| conn.id == from) {
                    conn.greeted = true;
                }
                self.welcome(from);
                self.tell_people();
                events.push(Event::Joined(from));
            }
            _ if self.host && !greeted => {}
            Msg::Welcome { you } if !self.host => {
                self.me = you;
                self.connected = true;
            }
            Msg::File { path, .. } | Msg::Gone { path, .. } | Msg::Edit { path, .. }
                if !self.host && !wire::safe_path(&path) =>
            {
                log::warn!("share: a file with a name that is not taken was sent: {path:?}");
            }
            Msg::File { text, .. }
                if !self.host
                    && (text.len() > LARGEST_FILE * 2
                        || self.docs.len() >= MOST_FILES * 2
                        || self.taken + text.len() > MOST_TEXT * 4) =>
            {
                self.give_up("the one sharing sent more than a session carries", events);
            }
            Msg::People { list } if !self.host => {
                let was = self.writer();
                self.seen.retain(|id, _| list.iter().any(|person| person.id == *id));
                self.people = list;
                if self.writer() != was {
                    events.push(Event::Role(self.writer()));
                }
            }
            Msg::Folder { id, name, .. } if !self.host => {
                self.folders.push((id, name.clone()));
                events.push(Event::Folder { id, name });
            }
            Msg::File { folder, path, text, epoch } if !self.host => {
                self.taken += text.len();
                let text = body(&text);
                let mut links = HashMap::new();
                links.insert(HOST, Link::fresh(epoch));
                let again =
                    self.docs.insert((folder, path.clone()), Doc { text: text.clone(), epoch, links }).is_some();
                events.push(Event::File { folder, path, text, again });
            }
            Msg::FolderReady { id } if !self.host => events.push(Event::FolderReady(id)),
            Msg::FolderGone { id } if !self.host => {
                self.folders.retain(|(folder, _)| *folder != id);
                self.docs.retain(|(folder, _), _| *folder != id);
                events.push(Event::FolderGone(id));
            }
            Msg::Gone { folder, path } if !self.host => {
                self.docs.remove(&(folder, path.clone()));
                events.push(Event::Gone { folder, path });
            }
            Msg::Edit { folder, path, epoch, number, seen, op } => {
                events.extend(self.edit(from, folder, path, (epoch, number, seen), op));
            }
            Msg::Save { folder, path } if self.host => {
                let known = self.docs.contains_key(&(folder, path.clone()));
                if known && self.person(from).is_some_and(|person| person.writer) {
                    events.push(Event::Save { folder, path, by: from });
                }
            }
            Msg::Make { folder, path, text } if self.host => {
                let fits = wire::safe_path(&path) && self.shares(folder) && text.len() <= LARGEST_FILE;
                let fresh = !self.docs.contains_key(&(folder, path.clone()));
                if fits && fresh && self.person(from).is_some_and(|person| person.writer) {
                    events.push(Event::Make { folder, path, text: body(&text), by: from });
                }
            }
            Msg::Drop { folder, path } if self.host => {
                let known = self.docs.contains_key(&(folder, path.clone()));
                if known && wire::safe_path(&path) && self.person(from).is_some_and(|person| person.writer) {
                    events.push(Event::Drop { folder, path, by: from });
                }
            }
            Msg::Said { folder, path, epoch, seen } => {
                let link = self.docs.get_mut(&(folder, path)).and_then(|doc| doc.links.get_mut(&from));
                if let Some(link) = link.filter(|link| link.epoch == epoch) {
                    link.heard(seen);
                }
            }
            Msg::Again { folder, path } if self.host => self.again(from, folder, &path),
            Msg::Seen { at, .. } if !at.sane() => {}
            Msg::Seen { who, at } => {
                let who = if self.host { from } else { who };
                if self.host {
                    self.to_all(&Msg::Seen { who, at: at.clone() }, Some(from));
                }
                self.seen.insert(who, at);
            }
            Msg::Come { who } => {
                let who = if self.host { from } else { who };
                if self.host {
                    self.to_all(&Msg::Come { who }, Some(from));
                }
                events.push(Event::Come(who));
            }
            Msg::Bye { why } => self.gone(from, why, events),
            _ => {}
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.close("the session ended");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn until(
        sessions: &mut [&mut Session],
        mut done: impl FnMut(&[&mut Session], &[Vec<Event>]) -> bool,
    ) -> Vec<Vec<Event>> {
        let started = Instant::now();
        let mut all: Vec<Vec<Event>> = sessions.iter().map(|_| Vec::new()).collect();
        loop {
            for (i, session) in sessions.iter_mut().enumerate() {
                all[i].extend(session.poll());
            }
            if done(sessions, &all) {
                return all;
            }
            assert!(started.elapsed() < Duration::from_secs(10), "nothing arrived: {all:?}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn the_text_of_a_file_has_one_form_whatever_its_line_ends() {
        assert_eq!(body("a\r\nb\r\n"), "a\nb");
        assert_eq!(body("a\nb"), "a\nb");
        assert_eq!(body(""), "");
    }

    #[test]
    fn a_guest_gets_the_folder_and_edits_travel_both_ways() {
        let mut host = Session::open_on("127.0.0.1", "Ana", true).unwrap();
        host.share_folder(7, "demo", vec![("src/a.rs".into(), "fn a() {}".into()), ("b.txt".into(), "one".into())]);
        let mut invite = host.invite.clone().unwrap();
        invite.host = "127.0.0.1".into();
        let mut guest = Session::join(&invite, "Beto");

        let events = until(&mut [&mut host, &mut guest], |_, all| all[1].contains(&Event::FolderReady(7)));
        assert!(events[0].contains(&Event::Joined(1)));
        let files: Vec<&str> = events[1]
            .iter()
            .filter_map(|event| match event {
                Event::File { path, .. } => Some(path.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(files, ["b.txt", "src/a.rs"]);
        until(&mut [&mut host, &mut guest], |sessions, _| sessions[1].people.len() == 2);
        assert_eq!((guest.me, guest.writer()), (1, true));
        assert_eq!(guest.others().map(|person| person.name.as_str()).collect::<Vec<_>>(), ["Ana"]);

        host.changed(7, "src/a.rs", "fn a() { 1 }");
        guest.changed(7, "src/a.rs", "// mine\nfn a() {}");
        let events = until(&mut [&mut host, &mut guest], |_, all| {
            let edits = |events: &Vec<Event>| events.iter().filter(|event| matches!(event, Event::Edit { .. })).count();
            edits(&all[0]) == 1 && edits(&all[1]) == 1
        });
        let text = |session: &Session| session.docs[&(7, "src/a.rs".to_string())].text.clone();
        assert_eq!(text(&host), "// mine\nfn a() { 1 }");
        assert_eq!(text(&host), text(&guest));
        assert!(matches!(&events[0][..], [Event::Edit { by: 1, .. }]));

        host.set_writer(1, false);
        until(&mut [&mut host, &mut guest], |sessions, _| !sessions[1].writer());
        guest.changed(7, "b.txt", "one two");
        let events = until(&mut [&mut host, &mut guest], |_, all| {
            all[1].iter().any(|event| matches!(event, Event::File { again: true, .. }))
        });
        assert!(events[0].is_empty(), "what a reader types changes nothing at the host");
        assert_eq!(guest.docs[&(7, "b.txt".to_string())].text, "one", "and the reader gets the text back");

        host.changed(7, "new.md", "# new");
        host.removed(7, "b.txt");
        until(&mut [&mut host, &mut guest], |sessions, _| sessions[1].has(7, "new.md") && !sessions[1].has(7, "b.txt"));

        host.see(Seen { folder: 7, anchor: "src/a.rs".into(), view: [800.0, 600.0], ..Default::default() });
        until(&mut [&mut host, &mut guest], |sessions, _| sessions[1].seen.contains_key(&HOST));
        guest.call_everyone();
        let events = until(&mut [&mut host, &mut guest], |_, all| !all[0].is_empty());
        assert_eq!(events[0], [Event::Come(1)], "the one who shares hears who calls, whatever the caller says it is");

        host.close("done for today");
        let events = until(&mut [&mut guest], |_, all| !all[0].is_empty());
        assert_eq!(events[0], [Event::Ended("done for today".to_string())]);
    }

    #[test]
    fn a_guest_takes_no_file_that_would_land_outside_or_in_gits_own_folder() {
        let (_tx, net) = mpsc::channel();
        let mut guest = Session::new(false, net, Arc::new(AtomicBool::new(false)));
        let mut events = Vec::new();
        for path in [".git/config", "../../up.txt", "sub/.GIT/hooks/pre-commit", "C:/Windows/x"] {
            guest.got(HOST, Msg::File { folder: 1, path: path.into(), text: "x".into(), epoch: 1 }, &mut events);
            guest.got(HOST, Msg::Gone { folder: 1, path: path.into() }, &mut events);
        }
        assert!(events.is_empty() && guest.docs.is_empty(), "{events:?}");
        guest.got(
            HOST,
            Msg::File { folder: 1, path: "src/a.rs".into(), text: "a\r\nb\rc".into(), epoch: 1 },
            &mut events,
        );
        assert_eq!(guest.text(1, "src/a.rs"), Some("a\nb\nc"), "and no carriage return comes in with a text");
        let far = Seen { offset: [f32::INFINITY, 0.0], view: [1.0, 1.0], ..Default::default() };
        guest.got(HOST, Msg::Seen { who: 0, at: far }, &mut events);
        assert!(guest.seen.is_empty());
        guest.got(
            HOST,
            Msg::File { folder: 1, path: "big".into(), text: "x".repeat(LARGEST_FILE * 2 + 1), epoch: 2 },
            &mut events,
        );
        assert!(matches!(events.last(), Some(Event::Ended(_))), "a file past every limit ends the session");
    }

    #[test]
    fn a_save_is_only_for_a_file_that_is_shared() {
        let mut host = Session::open_on("127.0.0.1", "Ana", true).unwrap();
        host.share_folder(7, "demo", vec![("a.txt".into(), "one".into())]);
        host.people.push(Person { id: 5, name: "Beto".into(), color: 1, writer: true, host: false });
        let (out, _kept) = mpsc::channel();
        host.conns.push(Conn { id: 5, out, waiting: Arc::new(AtomicUsize::new(0)), greeted: true });
        let mut events = Vec::new();
        host.got(5, Msg::Save { folder: 7, path: "a.txt".into() }, &mut events);
        host.got(5, Msg::Save { folder: 8, path: "a.txt".into() }, &mut events);
        host.got(5, Msg::Save { folder: 7, path: "not shared.txt".into() }, &mut events);
        assert_eq!(events, [Event::Save { folder: 7, path: "a.txt".into(), by: 5 }]);
        let before = host.invite.clone().unwrap().secret;
        host.send_away(5);
        assert_ne!(
            host.invite.clone().unwrap().secret,
            before,
            "who was sent away cannot come back with the same invitation"
        );
    }

    #[test]
    fn an_invitation_with_another_secret_does_not_get_in() {
        let mut host = Session::open_on("127.0.0.1", "Ana", false).unwrap();
        host.share_folder(1, "demo", vec![("a.txt".into(), "secret text".into())]);
        let mut invite = host.invite.clone().unwrap();
        invite.host = "127.0.0.1".into();
        invite.secret[0] ^= 1;
        let mut stranger = Session::join(&invite, "Eve");
        let events = until(&mut [&mut host, &mut stranger], |_, all| !all[1].is_empty());
        assert!(matches!(&events[1][..], [Event::Ended(_)]));
        assert!(events[0].is_empty() && host.people.len() == 1);
    }
}

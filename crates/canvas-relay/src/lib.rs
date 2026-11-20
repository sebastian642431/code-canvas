//! A relay for shared sessions of Hawk Canvas.
//!
//! Who shares and who joins both connect *out* to the relay, so neither needs an
//! address the other can reach. The one who shares opens a room and keeps that
//! connection (the room's control line); each guest asks for the room, the relay
//! tells the host on the control line that someone knocks, the host opens a fresh
//! connection that takes that knock, and from then on the relay passes the bytes
//! of the two connections to each other.
//!
//! The relay never has the secret of the invitation. What crosses it is the
//! session's own greeting (`CCSH1` and a random salt each way) and frames sealed
//! with ChaCha20-Poly1305 under keys made from that secret: it can tell how
//! large each frame is, and nothing else. It checks only that much (the greeting
//! starts as a session's does, no frame is larger than a session sends), so it
//! cannot be used to pass along anything that is not a session.
//!
//! The lines before the bytes start to flow, each ending in `\n`:
//!
//! ```text
//! host  -> relay   CCRELAY1 open
//! relay -> host    room <room> <token>          (or: no busy)
//! relay -> host    knock <n>                    (a guest waits; on the control line)
//! both  ->         alive                        (every ALIVE_EVERY on the control line)
//! guest -> relay   CCRELAY1 join <room>
//! host  -> relay   CCRELAY1 take <room> <token> <n>   (a new connection)
//! relay -> both    ok                           (or: no room | no full | no answer | no knock)
//! ```

use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Write};
use std::net::{IpAddr, Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ring::rand::{SecureRandom, SystemRandom};

pub const MAGIC: &str = "CCRELAY1";
pub const DEFAULT_PORT: u16 = 7878;
pub const ALIVE_EVERY: Duration = Duration::from_secs(20);
const LONGEST_LINE: usize = 200;
const SESSION_MAGIC: &[u8; 5] = b"CCSH1";
const GREETING: usize = 21;
const SMALLEST_FRAME: usize = 17;
const ROOM_LETTERS: &[u8; 32] = b"abcdefghijkmnpqrstuvwxyz23456789";
const ROOM_LENGTH: usize = 12;
const STACK: usize = 256 << 10;
const BUFFER: usize = 16 << 10;

/// How much one relay takes on.
#[derive(Clone, Debug)]
pub struct Limits {
    /// Rooms open at once.
    pub rooms: usize,
    /// Connections open at once, of every kind.
    pub connections: usize,
    /// Connections open at once from one address.
    pub per_address: usize,
    /// Rooms open at once by one address.
    pub rooms_per_address: usize,
    /// Guests connected through one room.
    pub guests: usize,
    /// Guests of one room waiting for the host to take them.
    pub waiting: usize,
    /// The largest frame passed on, in bytes (a session sends at most 16 MiB).
    pub largest: usize,
    /// A guest and a host between whom nothing crossed for this long are parted.
    pub idle: Duration,
    /// A room ends this long after it was opened, whatever happens in it.
    pub room_life: Duration,
    /// The first line of a connection must come within this.
    pub greeting: Duration,
    /// How long a guest waits for the host to take it.
    pub pair_wait: Duration,
    /// A room whose host was not heard for this long ends.
    pub quiet: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            rooms: 256,
            connections: 1024,
            per_address: 64,
            rooms_per_address: 8,
            guests: 32,
            waiting: 8,
            largest: 16 << 20,
            idle: Duration::from_secs(5 * 60),
            room_life: Duration::from_secs(24 * 3600),
            greeting: Duration::from_secs(10),
            pair_wait: Duration::from_secs(20),
            quiet: Duration::from_secs(75),
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn waited(e: &io::Error) -> bool {
    matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted)
}

fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    SystemRandom::new().fill(&mut bytes).expect("the system gives random bytes");
    bytes
}

fn new_room() -> String {
    random::<ROOM_LENGTH>().iter().map(|byte| ROOM_LETTERS[*byte as usize % ROOM_LETTERS.len()] as char).collect()
}

fn new_token() -> String {
    random::<16>().iter().map(|byte| format!("{byte:02x}")).collect()
}

fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |all, (x, y)| all | (x ^ y)) == 0
}

pub fn room_name_fits(room: &str) -> bool {
    (4..=32).contains(&room.len()) && room.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

/// Reads one line, a byte at a time so nothing after it is taken, by `until`.
pub fn read_line(stream: &mut TcpStream, until: Instant) -> io::Result<String> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "no answer came in time"));
        }
        stream.set_read_timeout(Some(left))?;
        match stream.read(&mut byte) {
            Ok(0) => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the connection was closed")),
            Ok(_) if byte[0] == b'\n' => break,
            Ok(_) if line.len() >= LONGEST_LINE => {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "a line that does not end"))
            }
            Ok(_) => line.push(byte[0]),
            Err(e) if waited(&e) => continue,
            Err(e) => return Err(e),
        }
    }
    let line =
        String::from_utf8(line).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "a line not in UTF-8"))?;
    Ok(line.trim_end_matches('\r').to_string())
}

fn say(mut stream: &TcpStream, line: &str) -> io::Result<()> {
    stream.write_all(format!("{line}\n").as_bytes())
}

/// Checks the bytes of one direction of a session as they pass: the greeting
/// first, then frames no larger than `largest`. It never sees inside a frame.
pub struct Frames {
    greeting: usize,
    head: [u8; 4],
    have: usize,
    left: usize,
    largest: usize,
}

impl Frames {
    pub fn new(largest: usize) -> Frames {
        Frames { greeting: 0, head: [0; 4], have: 0, left: 0, largest }
    }

    pub fn pass(&mut self, mut bytes: &[u8]) -> Result<(), &'static str> {
        while !bytes.is_empty() {
            if self.greeting < GREETING {
                let take = (GREETING - self.greeting).min(bytes.len());
                for (i, byte) in bytes[..take].iter().enumerate() {
                    let at = self.greeting + i;
                    if at < SESSION_MAGIC.len() && *byte != SESSION_MAGIC[at] {
                        return Err("that is not a Hawk Canvas session");
                    }
                }
                self.greeting += take;
                bytes = &bytes[take..];
            } else if self.left > 0 {
                let take = self.left.min(bytes.len());
                self.left -= take;
                bytes = &bytes[take..];
            } else {
                let take = (4 - self.have).min(bytes.len());
                self.head[self.have..self.have + take].copy_from_slice(&bytes[..take]);
                self.have += take;
                bytes = &bytes[take..];
                if self.have == 4 {
                    let length = u32::from_be_bytes(self.head) as usize;
                    if !(SMALLEST_FRAME..=self.largest).contains(&length) {
                        return Err("a message of an impossible size");
                    }
                    self.left = length;
                    self.have = 0;
                }
            }
        }
        Ok(())
    }
}

struct Waiting {
    stream: TcpStream,
    _slot: Slot,
    paired: mpsc::Sender<()>,
}

struct Room {
    token: String,
    owner: IpAddr,
    control: TcpStream,
    born: Instant,
    waiting: HashMap<u64, Waiting>,
    links: HashMap<u64, [TcpStream; 2]>,
}

impl Room {
    fn end(self) {
        let _ = self.control.shutdown(Shutdown::Both);
        for waiting in self.waiting.values() {
            let _ = waiting.stream.shutdown(Shutdown::Both);
        }
        for link in self.links.values().flatten() {
            let _ = link.shutdown(Shutdown::Both);
        }
    }
}

#[derive(Default)]
struct Counts {
    open: usize,
    from: HashMap<IpAddr, usize>,
}

#[derive(Default)]
struct State {
    rooms: HashMap<String, Room>,
    knocks: u64,
}

struct Shared {
    limits: Limits,
    state: Mutex<State>,
    counts: Mutex<Counts>,
    stop: AtomicBool,
    quiet: bool,
    born: Instant,
}

impl Shared {
    fn note(&self, text: &str) {
        if !self.quiet {
            eprintln!("canvas-relay: {text}");
        }
    }

    fn now(&self) -> u64 {
        self.born.elapsed().as_millis() as u64
    }

    fn admit(self: &Arc<Shared>, from: IpAddr) -> Option<Slot> {
        let mut counts = lock(&self.counts);
        let mine = counts.from.get(&from).copied().unwrap_or(0);
        if counts.open >= self.limits.connections || mine >= self.limits.per_address {
            return None;
        }
        counts.open += 1;
        counts.from.insert(from, mine + 1);
        Some(Slot { shared: self.clone(), from })
    }

    fn close_room(&self, room: &str, token: &str) -> bool {
        let gone = {
            let mut state = lock(&self.state);
            match state.rooms.get(room) {
                Some(open) if same(&open.token, token) => state.rooms.remove(room),
                _ => None,
            }
        };
        let ended = gone.is_some();
        gone.into_iter().for_each(Room::end);
        ended
    }

    fn expire(&self) {
        let old: Vec<(String, Room)> = {
            let mut state = lock(&self.state);
            let ids: Vec<String> = state
                .rooms
                .iter()
                .filter(|(_, room)| room.born.elapsed() >= self.limits.room_life)
                .map(|(id, _)| id.clone())
                .collect();
            ids.into_iter().filter_map(|id| state.rooms.remove(&id).map(|room| (id, room))).collect()
        };
        for (id, room) in old {
            self.note(&format!("room {id} ended: it was open as long as a room may be"));
            room.end();
        }
    }
}

/// One connection counted while it lives.
struct Slot {
    shared: Arc<Shared>,
    from: IpAddr,
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut counts = lock(&self.shared.counts);
        counts.open = counts.open.saturating_sub(1);
        if let Some(mine) = counts.from.get_mut(&self.from) {
            *mine = mine.saturating_sub(1);
            if *mine == 0 {
                counts.from.remove(&self.from);
            }
        }
    }
}

fn spawn(name: &str, work: impl FnOnce() + Send + 'static) -> io::Result<JoinHandle<()>> {
    std::thread::Builder::new().name(name.to_string()).stack_size(STACK).spawn(work)
}

/// A relay at work. It stops when dropped.
pub struct Relay {
    shared: Arc<Shared>,
    address: SocketAddr,
    accepting: Option<JoinHandle<()>>,
}

impl Relay {
    pub fn start(listen: impl ToSocketAddrs, limits: Limits, quiet: bool) -> io::Result<Relay> {
        let listener = TcpListener::bind(listen)?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let shared = Arc::new(Shared {
            limits,
            state: Mutex::new(State::default()),
            counts: Mutex::new(Counts::default()),
            stop: AtomicBool::new(false),
            quiet,
            born: Instant::now(),
        });
        let serving = shared.clone();
        let accepting = spawn("relay accept", move || accept(serving, listener))?;
        Ok(Relay { shared, address, accepting: Some(accepting) })
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    pub fn rooms(&self) -> usize {
        lock(&self.shared.state).rooms.len()
    }

    pub fn connections(&self) -> usize {
        lock(&self.shared.counts).open
    }

    /// Serves until the process ends.
    pub fn run(mut self) {
        if let Some(accepting) = self.accepting.take() {
            let _ = accepting.join();
        }
    }

    pub fn stop(&self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        let rooms: Vec<Room> = lock(&self.shared.state).rooms.drain().map(|(_, room)| room).collect();
        rooms.into_iter().for_each(Room::end);
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.stop();
    }
}

fn accept(shared: Arc<Shared>, listener: TcpListener) {
    let mut looked = Instant::now();
    while !shared.stop.load(Ordering::Relaxed) {
        if looked.elapsed() >= Duration::from_millis(500) {
            looked = Instant::now();
            shared.expire();
        }
        match listener.accept() {
            Ok((stream, peer)) => {
                let _ = stream.set_nonblocking(false);
                let Some(slot) = shared.admit(peer.ip()) else {
                    let _ = stream.set_write_timeout(Some(Duration::from_millis(200)));
                    let _ = say(&stream, "no busy");
                    continue;
                };
                let serving = shared.clone();
                if spawn("relay connection", move || welcome(serving, stream, slot)).is_err() {
                    shared.note("a thread could not be started for a connection");
                }
            }
            Err(e) if waited(&e) => std::thread::sleep(Duration::from_millis(25)),
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

fn welcome(shared: Arc<Shared>, mut stream: TcpStream, slot: Slot) {
    let _ = stream.set_nodelay(true);
    let _ = stream.set_write_timeout(Some(shared.limits.greeting));
    let Ok(line) = read_line(&mut stream, Instant::now() + shared.limits.greeting) else { return };
    let words: Vec<&str> = line.split(' ').collect();
    match words.as_slice() {
        [MAGIC, "open"] => room_opens(&shared, stream, slot),
        [MAGIC, "join", room] => guest_waits(&shared, stream, slot, room),
        [MAGIC, "take", room, token, knock] => match knock.parse() {
            Ok(knock) => host_takes(&shared, stream, slot, room, token, knock),
            Err(_) => {
                let _ = say(&stream, "no ask");
            }
        },
        _ => {
            let _ = say(&stream, "no ask");
        }
    }
}

fn room_opens(shared: &Arc<Shared>, mut stream: TcpStream, slot: Slot) {
    let limits = &shared.limits;
    let Ok(control) = stream.try_clone() else { return };
    let (room, token) = (new_room(), new_token());
    let rooms = {
        let mut state = lock(&shared.state);
        let mine = state.rooms.values().filter(|open| open.owner == slot.from).count();
        if state.rooms.len() >= limits.rooms || mine >= limits.rooms_per_address || state.rooms.contains_key(&room) {
            None
        } else {
            let made = Room {
                token: token.clone(),
                owner: slot.from,
                control,
                born: Instant::now(),
                waiting: HashMap::new(),
                links: HashMap::new(),
            };
            state.rooms.insert(room.clone(), made);
            Some(state.rooms.len())
        }
    };
    let Some(rooms) = rooms else {
        let _ = say(&stream, "no busy");
        return;
    };
    if say(&stream, &format!("room {room} {token}")).is_err() {
        shared.close_room(&room, &token);
        return;
    }
    shared.note(&format!("room {room} opened ({rooms} open)"));
    let (mut heard, mut said) = (Instant::now(), Instant::now());
    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    let mut buffer = [0u8; 64];
    let mut unended = 0usize;
    let why = loop {
        if shared.stop.load(Ordering::Relaxed) {
            break "the relay stopped";
        }
        match stream.read(&mut buffer) {
            Ok(0) => break "the host closed it",
            Ok(n) => {
                heard = Instant::now();
                for byte in &buffer[..n] {
                    unended = if *byte == b'\n' { 0 } else { unended + 1 };
                }
                if unended > LONGEST_LINE {
                    break "the host said what a host does not say";
                }
            }
            Err(e) if waited(&e) => {}
            Err(_) => break "the host's connection broke",
        }
        if heard.elapsed() > limits.quiet {
            break "the host was not heard for too long";
        }
        if said.elapsed() >= ALIVE_EVERY {
            said = Instant::now();
            let alive = match lock(&shared.state).rooms.get(&room) {
                Some(open) if same(&open.token, &token) => say(&open.control, "alive").is_ok(),
                _ => false,
            };
            if !alive {
                break "it ended";
            }
        }
    };
    if shared.close_room(&room, &token) {
        shared.note(&format!("room {room} ended: {why}"));
    }
    drop(slot);
}

fn guest_waits(shared: &Arc<Shared>, stream: TcpStream, slot: Slot, room: &str) {
    let limits = &shared.limits;
    let (paired, wait) = mpsc::channel();
    let mut held = Some((stream, slot));
    let (knock, refused) = {
        let mut state = lock(&shared.state);
        state.knocks += 1;
        let knock = state.knocks;
        let refused = match state.rooms.get_mut(room) {
            None => Some("no room"),
            Some(open) if open.waiting.len() >= limits.waiting || open.links.len() >= limits.guests => Some("no full"),
            Some(open) => match say(&open.control, &format!("knock {knock}")) {
                Err(_) => Some("no room"),
                Ok(()) => {
                    let (stream, slot) = held.take().expect("the guest is still here");
                    open.waiting.insert(knock, Waiting { stream, _slot: slot, paired });
                    None
                }
            },
        };
        (knock, refused)
    };
    if let (Some(why), Some((stream, _))) = (refused, held) {
        let _ = say(&stream, why);
        return;
    }
    if wait.recv_timeout(limits.pair_wait).is_ok() {
        return;
    }
    let back = lock(&shared.state).rooms.get_mut(room).and_then(|open| open.waiting.remove(&knock));
    if let Some(waiting) = back {
        let _ = say(&waiting.stream, "no answer");
    }
}

fn host_takes(shared: &Arc<Shared>, host: TcpStream, slot: Slot, room: &str, token: &str, knock: u64) {
    let found = {
        let mut state = lock(&shared.state);
        match state.rooms.get_mut(room) {
            Some(open) if same(&open.token, token) => open.waiting.remove(&knock).map(|waiting| {
                if let (Ok(guest), Ok(own)) = (waiting.stream.try_clone(), host.try_clone()) {
                    open.links.insert(knock, [guest, own]);
                }
                (waiting, open.links.len())
            }),
            _ => None,
        }
    };
    let Some((Waiting { stream: guest, _slot: guest_slot, paired }, guests)) = found else {
        let _ = say(&host, "no knock");
        return;
    };
    let unlink = || {
        if let Some(open) = lock(&shared.state).rooms.get_mut(room) {
            open.links.remove(&knock);
        }
    };
    let ready = say(&guest, "ok").and_then(|_| say(&host, "ok"));
    let _ = paired.send(());
    let (Ok(()), Ok(guest_out), Ok(host_out)) = (ready, guest.try_clone(), host.try_clone()) else {
        let _ = guest.shutdown(Shutdown::Both);
        let _ = host.shutdown(Shutdown::Both);
        unlink();
        return;
    };
    shared.note(&format!("a guest joined room {room} ({guests} in it)"));
    let last = Arc::new(AtomicU64::new(shared.now()));
    let (along, heard) = (shared.clone(), last.clone());
    let other = spawn("relay pass", move || {
        if let Err(why) = pass(&along, guest, host_out, &heard) {
            along.note(&format!("what a guest sent was not passed on: {why}"));
        }
        drop(guest_slot);
    });
    let why = pass(shared, host, guest_out, &last);
    if let Ok(other) = other {
        let _ = other.join();
    }
    unlink();
    if let Err(why) = why {
        shared.note(&format!("a guest of room {room} was parted: {why}"));
    }
    drop(slot);
}

fn pass(shared: &Shared, mut from: TcpStream, mut to: TcpStream, last: &AtomicU64) -> Result<(), &'static str> {
    let limits = &shared.limits;
    let idle = limits.idle.as_millis() as u64;
    let looks = limits.idle.clamp(Duration::from_millis(20), Duration::from_secs(1));
    let passed = (|| {
        from.set_read_timeout(Some(looks)).map_err(|_| "the connection broke")?;
        to.set_write_timeout(Some(limits.idle.max(Duration::from_millis(20)))).map_err(|_| "the connection broke")?;
        let mut frames = Frames::new(limits.largest);
        let mut buffer = vec![0u8; BUFFER];
        loop {
            if shared.stop.load(Ordering::Relaxed) {
                return Err("the relay stopped");
            }
            match from.read(&mut buffer) {
                Ok(0) => return Ok(()),
                Ok(n) => {
                    frames.pass(&buffer[..n])?;
                    to.write_all(&buffer[..n]).map_err(|_| "what was sent could not be passed on")?;
                    last.store(shared.now(), Ordering::Relaxed);
                }
                Err(e) if waited(&e) => {
                    if shared.now().saturating_sub(last.load(Ordering::Relaxed)) > idle {
                        return Err("nothing crossed for too long");
                    }
                }
                Err(_) => return Ok(()),
            }
        }
    })();
    let _ = from.shutdown(Shutdown::Both);
    let _ = to.shutdown(Shutdown::Both);
    passed
}

/// What the app uses to reach a relay.
pub mod client {
    use super::*;

    /// `host`, `host:port`, `[v6]:port` or a bare IPv6 address.
    pub fn address(text: &str) -> Option<(String, u16)> {
        let text = text.trim();
        let (host, port) = if let Some(rest) = text.strip_prefix('[') {
            let (host, after) = rest.split_once(']')?;
            match after {
                "" => (host, None),
                after => (host, Some(after.strip_prefix(':')?)),
            }
        } else if text.matches(':').count() == 1 {
            let (host, port) = text.split_once(':')?;
            (host, Some(port))
        } else {
            (text, None)
        };
        let port = match port {
            Some(port) => port.parse::<u16>().ok().filter(|port| *port != 0)?,
            None => DEFAULT_PORT,
        };
        let fits = !host.is_empty() && host.len() <= 253 && !host.chars().any(|c| c.is_whitespace() || c == '/');
        fits.then(|| (host.to_string(), port))
    }

    /// How an address is written back: `[v6]:port` keeps the colons apart.
    pub fn written(host: &str, port: u16) -> String {
        if host.contains(':') {
            format!("[{host}]:{port}")
        } else {
            format!("{host}:{port}")
        }
    }

    pub fn connect(relay: &str, wait: Duration) -> io::Result<TcpStream> {
        let bad = || io::Error::new(io::ErrorKind::InvalidInput, format!("{relay} is not the address of a relay"));
        let (host, port) = address(relay).ok_or_else(bad)?;
        let mut last = io::Error::new(io::ErrorKind::NotFound, format!("{host} was not found"));
        for found in (host.as_str(), port).to_socket_addrs()? {
            match TcpStream::connect_timeout(&found, wait) {
                Ok(stream) => {
                    stream.set_nodelay(true)?;
                    stream.set_write_timeout(Some(wait))?;
                    return Ok(stream);
                }
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    fn meaning(said: &str) -> String {
        match said {
            "no room" => "the relay has no such room: the session was stopped, or the invitation is old".into(),
            "no full" => "the session has as many guests as the relay lets in".into(),
            "no busy" => "the relay is busy (too many rooms or connections)".into(),
            "no answer" => "the one sharing did not answer through the relay".into(),
            "no knock" => "nobody was waiting at the relay".into(),
            "no ask" => "the relay did not understand the question".into(),
            other => format!("the relay said \"{}\"", other.chars().take(60).collect::<String>()),
        }
    }

    fn answer(stream: &mut TcpStream, wait: Duration) -> io::Result<String> {
        match read_line(stream, Instant::now() + wait) {
            Ok(line) if line == "ok" || line.starts_with("room ") => Ok(line),
            Ok(line) => Err(io::Error::new(io::ErrorKind::ConnectionRefused, meaning(&line))),
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                Err(io::Error::new(e.kind(), "the relay closed the connection"))
            }
            Err(e) => Err(e),
        }
    }

    /// Opens a room: the control line, the room and the token that takes its guests.
    pub fn open(relay: &str, wait: Duration) -> io::Result<(Control, String, String)> {
        let mut stream = connect(relay, wait)?;
        say(&stream, &format!("{MAGIC} open"))?;
        let line = answer(&mut stream, wait)?;
        let mut words = line.split(' ').skip(1);
        let (Some(room), Some(token), None) = (words.next(), words.next(), words.next()) else {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "the relay gave no room"));
        };
        if !room_name_fits(room) || !room_name_fits(token) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "the relay gave a room that is not one"));
        }
        let control =
            Control { stream, kept: Vec::new(), knocks: VecDeque::new(), heard: Instant::now(), said: Instant::now() };
        Ok((control, room.to_string(), token.to_string()))
    }

    /// Takes the guest that knocked: the connection to it, through the relay.
    pub fn take(relay: &str, room: &str, token: &str, knock: u64, wait: Duration) -> io::Result<TcpStream> {
        let mut stream = connect(relay, wait)?;
        say(&stream, &format!("{MAGIC} take {room} {token} {knock}"))?;
        answer(&mut stream, wait)?;
        stream.set_read_timeout(None)?;
        Ok(stream)
    }

    /// Joins a room: the connection to its host, through the relay.
    pub fn join(relay: &str, room: &str, wait: Duration) -> io::Result<TcpStream> {
        if !room_name_fits(room) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "that is not the name of a room"));
        }
        let mut stream = connect(relay, wait)?;
        say(&stream, &format!("{MAGIC} join {room}"))?;
        answer(&mut stream, wait + Limits::default().pair_wait)?;
        stream.set_read_timeout(None)?;
        Ok(stream)
    }

    /// The control line of a room, kept by the one who shares.
    pub struct Control {
        stream: TcpStream,
        kept: Vec<u8>,
        knocks: VecDeque<u64>,
        heard: Instant,
        said: Instant,
    }

    impl Control {
        /// A handle that can close the line from another thread.
        pub fn handle(&self) -> io::Result<TcpStream> {
            self.stream.try_clone()
        }

        /// Waits up to about half a second: the number of a guest who knocks,
        /// or nothing. Says it is alive when due; an error is the room's end.
        pub fn listen(&mut self) -> io::Result<Option<u64>> {
            if let Some(knock) = self.knocks.pop_front() {
                return Ok(Some(knock));
            }
            if self.said.elapsed() >= ALIVE_EVERY {
                say(&self.stream, "alive")?;
                self.said = Instant::now();
            }
            if self.heard.elapsed() > ALIVE_EVERY * 3 {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "the relay was not heard for a minute"));
            }
            self.stream.set_read_timeout(Some(Duration::from_millis(500)))?;
            let mut buffer = [0u8; 256];
            match self.stream.read(&mut buffer) {
                Ok(0) => Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the relay closed the room")),
                Ok(n) => {
                    self.heard = Instant::now();
                    self.kept.extend_from_slice(&buffer[..n]);
                    while let Some(at) = self.kept.iter().position(|byte| *byte == b'\n') {
                        let line: Vec<u8> = self.kept.drain(..=at).collect();
                        let line = String::from_utf8_lossy(&line);
                        if let Some(knock) = line.trim().strip_prefix("knock ").and_then(|n| n.parse().ok()) {
                            self.knocks.push_back(knock);
                        }
                    }
                    if self.kept.len() > LONGEST_LINE {
                        return Err(io::Error::new(io::ErrorKind::InvalidData, "the relay said something odd"));
                    }
                    Ok(self.knocks.pop_front())
                }
                Err(e) if waited(&e) => Ok(None),
                Err(e) => Err(e),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::client::*;
    use super::*;

    const WAIT: Duration = Duration::from_secs(5);

    fn started(limits: Limits) -> (Relay, String) {
        let relay = Relay::start("127.0.0.1:0", limits, true).unwrap();
        let address = relay.address().to_string();
        (relay, address)
    }

    fn knock(control: &mut Control) -> u64 {
        let until = Instant::now() + WAIT;
        loop {
            if let Some(knock) = control.listen().unwrap() {
                return knock;
            }
            assert!(Instant::now() < until, "nobody knocked");
        }
    }

    fn greeting(salt: u8) -> Vec<u8> {
        [b"CCSH1".as_slice(), &[salt; 16]].concat()
    }

    fn frame(body: &[u8]) -> Vec<u8> {
        [(body.len() as u32).to_be_bytes().as_slice(), body].concat()
    }

    fn pair(address: &str) -> (Control, TcpStream, TcpStream) {
        let (mut control, room, token) = open(address, WAIT).unwrap();
        let joining = {
            let (address, room) = (address.to_string(), room.clone());
            std::thread::spawn(move || join(&address, &room, WAIT))
        };
        let knock = knock(&mut control);
        let host = take(address, &room, &token, knock, WAIT).unwrap();
        let guest = joining.join().unwrap().unwrap();
        (control, host, guest)
    }

    fn until(what: &str, mut done: impl FnMut() -> bool) {
        let until = Instant::now() + WAIT;
        while !done() {
            assert!(Instant::now() < until, "{what} never happened");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn closed(stream: &mut TcpStream) -> bool {
        stream.set_read_timeout(Some(WAIT)).unwrap();
        let mut byte = [0u8; 1];
        matches!(stream.read(&mut byte), Ok(0) | Err(_))
    }

    #[test]
    fn an_address_of_a_relay_is_read_with_or_without_its_port() {
        assert_eq!(address("relay.example.org"), Some(("relay.example.org".into(), DEFAULT_PORT)));
        assert_eq!(address(" relay.example.org:9000 "), Some(("relay.example.org".into(), 9000)));
        assert_eq!(address("[::1]:7000"), Some(("::1".into(), 7000)));
        assert_eq!(address("::1"), Some(("::1".into(), DEFAULT_PORT)));
        for bad in ["", ":7878", "host:0", "host:port", "a b:1", "[::1]x", "http://x"] {
            assert_eq!(address(bad), None, "{bad}");
        }
        assert_eq!(written("::1", 7000), "[::1]:7000");
        assert_eq!(written("10.0.0.2", 7878), "10.0.0.2:7878");
    }

    #[test]
    fn frames_pass_only_after_a_sessions_greeting_and_never_larger_than_allowed() {
        let mut frames = Frames::new(1000);
        let mut all = greeting(1);
        all.extend(frame(&[9u8; 40]));
        all.extend(frame(&[9u8; 17]));
        for chunk in all.chunks(3) {
            frames.pass(chunk).unwrap();
        }
        assert!(Frames::new(1000).pass(b"GET / HTTP/1.1\r\n").is_err());
        let mut frames = Frames::new(1000);
        frames.pass(&greeting(1)).unwrap();
        assert!(frames.pass(&1001u32.to_be_bytes()).is_err(), "too large");
        let mut frames = Frames::new(1000);
        frames.pass(&greeting(1)).unwrap();
        assert!(frames.pass(&3u32.to_be_bytes()).is_err(), "too small to be sealed");
    }

    #[test]
    fn a_guest_and_a_host_are_paired_and_their_bytes_cross_unchanged() {
        let (relay, address) = started(Limits::default());
        let (_control, mut host, mut guest) = pair(&address);
        assert_eq!(relay.rooms(), 1);
        let said = [greeting(1), frame(&[7u8; 30])].concat();
        guest.write_all(&said).unwrap();
        let mut got = vec![0u8; said.len()];
        host.read_exact(&mut got).unwrap();
        assert_eq!(got, said);
        let back = [greeting(2), frame(&[8u8; 5000])].concat();
        host.write_all(&back).unwrap();
        let mut got = vec![0u8; back.len()];
        guest.read_exact(&mut got).unwrap();
        assert_eq!(got, back);

        drop(host);
        assert!(closed(&mut guest), "when one side leaves the other is let go");
        until("both connections counted out", || relay.connections() == 1);
    }

    #[test]
    fn a_room_that_does_not_exist_or_a_wrong_token_pairs_nobody() {
        let (_relay, address) = started(Limits { pair_wait: Duration::from_millis(600), ..Limits::default() });
        let error = join(&address, "nosuchroom", WAIT).unwrap_err();
        assert!(error.to_string().contains("no such room"), "{error}");
        let (mut control, room, token) = open(&address, WAIT).unwrap();
        let joining = {
            let (address, room) = (address.clone(), room.clone());
            std::thread::spawn(move || join(&address, &room, WAIT))
        };
        let knock = knock(&mut control);
        assert!(take(&address, &room, &"0".repeat(32), knock, WAIT).is_err(), "only the host takes its guests");
        assert!(take(&address, &room, &token, knock + 1, WAIT).is_err(), "and only one who knocked");
        let error = joining.join().unwrap().unwrap_err();
        assert!(error.to_string().contains("did not answer"), "{error}");
    }

    #[test]
    fn what_is_not_a_session_or_too_large_is_cut() {
        let (_relay, address) = started(Limits { largest: 4096, ..Limits::default() });
        let (_control, mut host, mut guest) = pair(&address);
        guest.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
        assert!(closed(&mut host), "the relay passes on nothing but a session");

        let (_control, mut host, mut guest) = pair(&address);
        guest.write_all(&greeting(1)).unwrap();
        let mut got = [0u8; GREETING];
        host.read_exact(&mut got).unwrap();
        guest.write_all(&4097u32.to_be_bytes()).unwrap();
        assert!(closed(&mut host), "a frame larger than the limit ends the pair");
    }

    #[test]
    fn rooms_and_connections_are_limited() {
        let limits = Limits { rooms: 2, rooms_per_address: 1, ..Limits::default() };
        let (_relay, address) = started(limits);
        let first = open(&address, WAIT).unwrap();
        let error = open(&address, WAIT).err().unwrap();
        assert!(error.to_string().contains("busy"), "one address opens one room: {error}");
        drop(first);

        let limits = Limits { connections: 2, ..Limits::default() };
        let (relay, address) = started(limits);
        let _a = open(&address, WAIT).unwrap();
        let _b = open(&address, WAIT).unwrap();
        until("two open", || relay.connections() == 2);
        assert!(open(&address, WAIT).is_err(), "a third connection is refused");

        let limits = Limits { waiting: 1, pair_wait: Duration::from_secs(3), ..Limits::default() };
        let (_relay, address) = started(limits);
        let (mut control, room, _) = open(&address, WAIT).unwrap();
        let first = {
            let (address, room) = (address.clone(), room.clone());
            std::thread::spawn(move || join(&address, &room, WAIT))
        };
        knock(&mut control);
        let error = join(&address, &room, WAIT).unwrap_err();
        assert!(error.to_string().contains("as many guests"), "{error}");
        drop(control);
        assert!(first.join().unwrap().is_err(), "the room ended with its host");
    }

    #[test]
    fn a_quiet_pair_is_parted_and_an_old_room_ends() {
        let limits = Limits { idle: Duration::from_millis(300), ..Limits::default() };
        let (_relay, address) = started(limits);
        let (_control, mut host, mut guest) = pair(&address);
        guest.write_all(&greeting(1)).unwrap();
        let mut got = [0u8; GREETING];
        host.read_exact(&mut got).unwrap();
        let since = Instant::now();
        assert!(closed(&mut host));
        assert!(since.elapsed() < Duration::from_secs(3));

        let limits = Limits { room_life: Duration::from_millis(400), ..Limits::default() };
        let (relay, address) = started(limits);
        let (mut control, _host, mut guest) = pair(&address);
        assert!(closed(&mut guest), "the guests of a room go with it");
        until("the room gone", || relay.rooms() == 0);
        let until = Instant::now() + WAIT;
        loop {
            match control.listen() {
                Err(_) => break,
                Ok(_) => assert!(Instant::now() < until, "the host hears that its room ended"),
            }
        }
    }

    #[test]
    fn a_host_that_leaves_ends_its_room() {
        let (relay, address) = started(Limits::default());
        let (control, room, _) = open(&address, WAIT).unwrap();
        assert_eq!(relay.rooms(), 1);
        drop(control);
        until("the room gone", || relay.rooms() == 0);
        assert!(join(&address, &room, WAIT).is_err());
    }
}

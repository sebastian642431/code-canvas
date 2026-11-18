use std::io::{self, Read, Write};

use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, CHACHA20_POLY1305};
use ring::hkdf;
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};

use super::ot::TextOp;

pub const VERSION: u32 = 5;
pub const DEFAULT_PORT: u16 = 47800;
const MAGIC: &[u8; 5] = b"CCSH1";
const LARGEST: usize = 16 << 20;
pub const FIRST: usize = 64 << 10;
const LONGEST_NAME: usize = 1024;
const FARTHEST: f32 = 1.0e9;
const PACKED_FROM: usize = 512;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Person {
    pub id: u32,
    pub name: String,
    pub color: u8,
    pub writer: bool,
    pub host: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Caret {
    pub path: String,
    pub line: u32,
    pub col: u32,
    pub from: Option<(u32, u32)>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Seen {
    pub folder: u32,
    pub anchor: String,
    pub offset: [f32; 2],
    pub view: [f32; 2],
    pub pointer: Option<(String, [f32; 2])>,
    pub caret: Option<Caret>,
}

impl Seen {
    pub fn sane(&self) -> bool {
        let near = |v: &[f32; 2]| v.iter().all(|n| n.is_finite() && n.abs() <= FARTHEST);
        let named = |text: &str| text.len() <= LONGEST_NAME;
        near(&self.offset)
            && near(&self.view)
            && self.view.iter().all(|n| *n > 0.0)
            && named(&self.anchor)
            && self.pointer.as_ref().is_none_or(|(at, offset)| named(at) && near(offset))
            && self.caret.as_ref().is_none_or(|caret| named(&caret.path))
    }
}

pub const MOST_HELPERS: usize = 24;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Place {
    pub folder: u32,
    pub path: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Track {
    pub doing: u8,
    pub at: Place,
    pub lines: Option<(u32, u32)>,
    pub note: String,
    pub ago: u32,
    pub moved: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Helper {
    pub key: String,
    pub avatar: u8,
    pub role: String,
    pub task: String,
    pub doing: u8,
    pub note: String,
    pub at: Option<Place>,
    pub lines: Option<(u32, u32)>,
    pub find: Option<String>,
    pub ago: u32,
    pub moved: u64,
    pub trail: Vec<Track>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
}

impl Place {
    fn sane(&self) -> bool {
        self.path.is_empty() || safe_path(self.path.trim_end_matches('/'))
    }
}

impl Helper {
    pub fn sane(list: &[Helper]) -> bool {
        let short = |text: &str| text.len() <= LONGEST_NAME;
        let few = |lines: &Option<(u32, u32)>| lines.is_none_or(|(_, count)| count <= 400);
        list.len() <= MOST_HELPERS
            && list.iter().all(|helper| {
                [&helper.key, &helper.role, &helper.task, &helper.note].iter().all(|text| short(text))
                    && helper.find.as_deref().is_none_or(short)
                    && helper.at.as_ref().is_none_or(Place::sane)
                    && few(&helper.lines)
                    && helper.trail.len() <= 2 * crate::agents::TRAIL
                    && helper.trail.iter().all(|track| track.at.sane() && short(&track.note) && few(&track.lines))
            })
    }

    pub fn older(&mut self, by: u32) {
        self.ago = self.ago.saturating_add(by);
        for track in &mut self.trail {
            track.ago = track.ago.saturating_add(by);
        }
    }
}

pub fn turns_text(c: char) -> bool {
    matches!(c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

pub fn safe_path(path: &str) -> bool {
    let part = |part: &str| {
        let bare = part.trim_end_matches(['.', ' ']).to_lowercase();
        let odd = part.chars().any(|c| c.is_control() || "<>:\"|?*".contains(c));
        !bare.is_empty() && bare != ".git" && !bare.starts_with("git~") && !odd && part.len() <= 255
    };
    !path.is_empty() && path.len() <= LONGEST_NAME && path.split(['/', '\\']).all(part)
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "t")]
pub enum Msg {
    Hello { name: String, version: u32 },
    Welcome { you: u32 },
    People { list: Vec<Person> },
    Folder { id: u32, name: String, files: u32 },
    File { folder: u32, path: String, text: String, epoch: u32 },
    FolderReady { id: u32 },
    FolderGone { id: u32 },
    Gone { folder: u32, path: String },
    Edit { folder: u32, path: String, epoch: u32, number: u64, seen: u64, op: TextOp },
    Save { folder: u32, path: String, format: bool },
    Make { folder: u32, path: String, text: String },
    Drop { folder: u32, path: String },
    Again { folder: u32, path: String },
    Seen { who: u32, at: Seen },
    Come { who: u32 },
    Agents { who: u32, list: Vec<Helper> },
    Said { folder: u32, path: String, epoch: u32, seen: u64 },
    Still,
    Bye { why: String },
}

/// Where to find a session and the secret that opens it. With a `room`, `host`
/// and `port` are the relay's and the room is the session's place there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invite {
    pub host: String,
    pub port: u16,
    pub secret: [u8; 16],
    pub room: Option<String>,
}

impl Invite {
    pub fn written(&self) -> String {
        let secret: String = self.secret.iter().map(|byte| format!("{byte:02x}")).collect();
        match &self.room {
            Some(room) => format!("cc2:relay:{}:{}:{room}:{secret}", self.host, self.port),
            None => format!("cc1:{}:{}:{secret}", self.host, self.port),
        }
    }

    pub fn read(text: &str) -> Option<Invite> {
        let text = text.trim();
        let (rest, relayed) = match text.strip_prefix("cc2:relay:") {
            Some(rest) => (rest, true),
            None => (text.strip_prefix("cc1:")?, false),
        };
        let (rest, secret) = rest.rsplit_once(':')?;
        let (rest, room) = match relayed {
            true => {
                let (rest, room) = rest.rsplit_once(':')?;
                if !canvas_relay::room_name_fits(room) {
                    return None;
                }
                (rest, Some(room.to_string()))
            }
            false => (rest, None),
        };
        let (host, port) = rest.rsplit_once(':')?;
        if secret.len() != 32 || host.is_empty() || !secret.is_ascii() {
            return None;
        }
        let mut bytes = [0u8; 16];
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&secret[i * 2..i * 2 + 2], 16).ok()?;
        }
        let host = host.trim_start_matches('[').trim_end_matches(']').to_string();
        Some(Invite { host, port: port.parse().ok()?, secret: bytes, room })
    }

    /// The relay this invitation goes through, as an address to connect to.
    pub fn relay(&self) -> Option<String> {
        self.room.as_ref().map(|_| canvas_relay::client::written(&self.host, self.port))
    }
}

pub fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    SystemRandom::new().fill(&mut bytes).expect("the system gives random bytes");
    bytes
}

pub struct Seal {
    key: LessSafeKey,
    count: u64,
}

impl Seal {
    fn nonce(&mut self) -> Nonce {
        let mut bytes = [0u8; 12];
        bytes[4..].copy_from_slice(&self.count.to_be_bytes());
        self.count += 1;
        Nonce::assume_unique_for_key(bytes)
    }

    fn close(&mut self, plain: Vec<u8>) -> io::Result<Vec<u8>> {
        let mut sealed = plain;
        let nonce = self.nonce();
        self.key
            .seal_in_place_append_tag(nonce, Aad::empty(), &mut sealed)
            .map_err(|_| io::Error::other("could not be sealed"))?;
        Ok(sealed)
    }

    fn open(&mut self, mut sealed: Vec<u8>) -> io::Result<Vec<u8>> {
        let nonce = self.nonce();
        let wrong = || io::Error::new(io::ErrorKind::InvalidData, "the invitation is not the one of this session");
        let plain = self.key.open_in_place(nonce, Aad::empty(), &mut sealed).map_err(|_| wrong())?.len();
        sealed.truncate(plain);
        Ok(sealed)
    }
}

fn seals(secret: &[u8; 16], of_guest: &[u8; 16], of_host: &[u8; 16]) -> (Seal, Seal) {
    let salt = [of_guest.as_slice(), of_host.as_slice()].concat();
    let ready = hkdf::Salt::new(hkdf::HKDF_SHA256, &salt).extract(secret);
    let key = |way: &[u8]| {
        let info = [way];
        let made = ready.expand(&info, &CHACHA20_POLY1305).expect("a key of this length can be made");
        Seal { key: LessSafeKey::new(UnboundKey::from(made)), count: 0 }
    };
    (key(b"code canvas, to the host"), key(b"code canvas, to the guest"))
}

fn salt_from(stream: &mut impl Read) -> io::Result<[u8; 16]> {
    let mut greeting = [0u8; 21];
    stream.read_exact(&mut greeting)?;
    if &greeting[..5] != MAGIC {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "that is not a Code Canvas session"));
    }
    let mut salt = [0u8; 16];
    salt.copy_from_slice(&greeting[5..]);
    Ok(salt)
}

fn salt_to(stream: &mut impl Write, salt: &[u8; 16]) -> io::Result<()> {
    stream.write_all(&[MAGIC.as_slice(), salt.as_slice()].concat())?;
    stream.flush()
}

pub fn greet_as_guest<S: Read + Write>(stream: &mut S, secret: &[u8; 16]) -> io::Result<(Seal, Seal)> {
    let mine = random::<16>();
    salt_to(stream, &mine)?;
    let theirs = salt_from(stream)?;
    let (to_host, to_guest) = seals(secret, &mine, &theirs);
    Ok((to_host, to_guest))
}

pub fn greet_as_host<S: Read + Write>(stream: &mut S, secret: &[u8; 16]) -> io::Result<(Seal, Seal)> {
    let theirs = salt_from(stream)?;
    let mine = random::<16>();
    salt_to(stream, &mine)?;
    let (to_host, to_guest) = seals(secret, &theirs, &mine);
    Ok((to_guest, to_host))
}

pub fn write_msg(stream: &mut impl Write, seal: &mut Seal, msg: &Msg) -> io::Result<()> {
    let json = serde_json::to_vec(msg).map_err(io::Error::other)?;
    let mut plain = Vec::with_capacity(json.len() / 3 + 16);
    if json.len() >= PACKED_FROM {
        plain.push(1);
        let mut packer = flate2::write::DeflateEncoder::new(plain, flate2::Compression::fast());
        packer.write_all(&json)?;
        plain = packer.finish()?;
    } else {
        plain.push(0);
        plain.extend_from_slice(&json);
    }
    let sealed = seal.close(plain)?;
    stream.write_all(&(sealed.len() as u32).to_be_bytes())?;
    stream.write_all(&sealed)?;
    stream.flush()
}

pub fn read_msg(stream: &mut impl Read, seal: &mut Seal) -> io::Result<Msg> {
    read_msg_within(stream, seal, LARGEST)
}

pub fn read_msg_within(stream: &mut impl Read, seal: &mut Seal, most: usize) -> io::Result<Msg> {
    let odd = |what: &str| io::Error::new(io::ErrorKind::InvalidData, what.to_string());
    let mut length = [0u8; 4];
    stream.read_exact(&mut length)?;
    let length = u32::from_be_bytes(length) as usize;
    if !(17..=most.min(LARGEST)).contains(&length) {
        return Err(odd("a message of an impossible size"));
    }
    let mut sealed = Vec::new();
    Read::by_ref(stream).take(length as u64).read_to_end(&mut sealed)?;
    if sealed.len() != length {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "a message cut short"));
    }
    let plain = seal.open(sealed)?;
    let json = match plain[0] {
        0 => plain[1..].to_vec(),
        _ => {
            let mut json = Vec::new();
            let room = LARGEST as u64 * 2;
            flate2::read::DeflateDecoder::new(&plain[1..]).take(room + 1).read_to_end(&mut json)?;
            if json.len() as u64 > room {
                return Err(odd("a message that unpacks to too much"));
            }
            json
        }
    };
    serde_json::from_slice(&json).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Pipe {
        incoming: io::Cursor<Vec<u8>>,
        outgoing: Vec<u8>,
    }

    impl Read for Pipe {
        fn read(&mut self, into: &mut [u8]) -> io::Result<usize> {
            self.incoming.read(into)
        }
    }

    impl Write for Pipe {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.outgoing.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn pair(of_guest: &[u8; 16], of_host: &[u8; 16]) -> ((Seal, Seal), (Seal, Seal)) {
        pair_salted(of_guest, of_host, [7u8; 16], [9u8; 16])
    }

    fn pair_salted(
        of_guest: &[u8; 16],
        of_host: &[u8; 16],
        guest_salt: [u8; 16],
        host_salt: [u8; 16],
    ) -> ((Seal, Seal), (Seal, Seal)) {
        let (to_host, to_guest) = seals(of_guest, &guest_salt, &host_salt);
        let (at_host_in, at_host_out) = seals(of_host, &guest_salt, &host_salt);
        ((to_host, to_guest), (at_host_out, at_host_in))
    }

    #[test]
    fn an_invitation_is_read_back_as_it_was_written() {
        let invite = Invite { host: "192.168.0.14".into(), port: 47800, secret: [0xab; 16], room: None };
        let text = invite.written();
        assert_eq!(text, format!("cc1:192.168.0.14:47800:{}", "ab".repeat(16)));
        assert_eq!(Invite::read(&format!("  {text}\n")), Some(invite));
        assert_eq!(Invite::read("cc1:fe80::1:47800:00112233445566778899aabbccddeeff").unwrap().host, "fe80::1");
        let relayed =
            Invite { host: "relay.example.org".into(), port: 7878, secret: [1; 16], room: Some("k3x9ab2cd4ef".into()) };
        let text = relayed.written();
        assert_eq!(text, format!("cc2:relay:relay.example.org:7878:k3x9ab2cd4ef:{}", "01".repeat(16)));
        assert_eq!(Invite::read(&text), Some(relayed.clone()));
        assert_eq!(relayed.relay().as_deref(), Some("relay.example.org:7878"));
        let v6 = Invite::read("cc2:relay:::1:7000:room1:00112233445566778899aabbccddeeff").unwrap();
        assert_eq!((v6.host.as_str(), v6.port, v6.relay().unwrap().as_str()), ("::1", 7000, "[::1]:7000"));
        for bad in [
            "",
            "cc1:host:47800:short",
            "cc2:host:1:00112233445566778899aabbccddeeff",
            "cc1::1:00112233445566778899aabbccddeeff",
            "cc2:relay:host:7878:00112233445566778899aabbccddeeff",
            "cc2:relay:host:7878:no room:00112233445566778899aabbccddeeff",
            "cc2:relay:host:7878:a/b:00112233445566778899aabbccddeeff",
        ] {
            assert!(Invite::read(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn a_frame_changed_replayed_reordered_or_sent_back_is_refused() {
        let secret = [6u8; 16];
        let frames = |count: u32| {
            let ((mut guest_out, _), (_, host_in)) = pair(&secret, &secret);
            let mut all = Vec::new();
            for who in 0..count {
                let mut one = Vec::new();
                write_msg(&mut one, &mut guest_out, &Msg::Come { who }).unwrap();
                all.push(one);
            }
            (all, host_in)
        };
        let (all, mut host_in) = frames(1);
        let mut changed = all[0].clone();
        let last = changed.len() - 1;
        changed[last] ^= 0x01;
        assert!(read_msg(&mut io::Cursor::new(changed), &mut host_in).is_err(), "one bit of the tag changed");
        let (all, mut host_in) = frames(1);
        let mut changed = all[0].clone();
        changed[6] ^= 0x80;
        assert!(read_msg(&mut io::Cursor::new(changed), &mut host_in).is_err(), "one bit of the body changed");

        let (all, mut host_in) = frames(2);
        assert_eq!(read_msg(&mut io::Cursor::new(all[0].clone()), &mut host_in).unwrap(), Msg::Come { who: 0 });
        assert!(read_msg(&mut io::Cursor::new(all[0].clone()), &mut host_in).is_err(), "the same frame again");
        let (all, mut host_in) = frames(2);
        assert!(read_msg(&mut io::Cursor::new(all[1].clone()), &mut host_in).is_err(), "the second before the first");

        let ((mut guest_out, mut guest_in), _) = pair(&secret, &secret);
        let mut mine = Vec::new();
        write_msg(&mut mine, &mut guest_out, &Msg::Still).unwrap();
        assert!(read_msg(&mut io::Cursor::new(mine), &mut guest_in).is_err(), "what one sent, sent back to it");

        let (all, _) = frames(1);
        let (_, (_, mut later_in)) = pair_salted(&secret, &secret, [1u8; 16], [2u8; 16]);
        let played = read_msg(&mut io::Cursor::new(all[0].clone()), &mut later_in);
        assert!(played.is_err(), "a frame of an earlier connection, played again in a new one");
    }

    #[test]
    fn messages_cross_sealed_and_only_with_the_same_secret() {
        let secret = [3u8; 16];
        let ((mut guest_out, mut guest_in), (mut host_out, mut host_in)) = pair(&secret, &secret);
        let long = Msg::File { folder: 1, path: "src/a.rs".into(), text: "fn main() {}\n".repeat(400), epoch: 0 };
        let short = Msg::Save { folder: 1, path: "src/a.rs".into(), format: true };
        let mut wire = Vec::new();
        write_msg(&mut wire, &mut guest_out, &long).unwrap();
        write_msg(&mut wire, &mut guest_out, &short).unwrap();
        assert!(wire.len() < 1000, "a long text travels packed: {}", wire.len());
        assert!(!wire.windows(7).any(|bytes| bytes == b"fn main"), "and nothing of it can be read on the way");
        let mut arriving = io::Cursor::new(wire);
        assert_eq!(read_msg(&mut arriving, &mut host_in).unwrap(), long);
        assert_eq!(read_msg(&mut arriving, &mut host_in).unwrap(), short);

        let mut back = Vec::new();
        write_msg(&mut back, &mut host_out, &Msg::Welcome { you: 4 }).unwrap();
        assert_eq!(read_msg(&mut io::Cursor::new(back), &mut guest_in).unwrap(), Msg::Welcome { you: 4 });

        let ((mut stranger_out, _), (_, mut host_in)) = pair(&[4u8; 16], &secret);
        let mut wire = Vec::new();
        write_msg(&mut wire, &mut stranger_out, &Msg::Hello { name: "x".into(), version: VERSION }).unwrap();
        assert!(read_msg(&mut io::Cursor::new(wire), &mut host_in).is_err(), "another secret opens nothing");
    }

    #[test]
    fn a_path_from_the_other_side_stays_inside_and_is_never_gits_own() {
        for good in ["a.rs", "src/app/main.rs", ".gitignore", ".github/workflows/ci.yml", "with space/ñ €.md", "git.rs"]
        {
            assert!(safe_path(good), "{good}");
        }
        let long = "x".repeat(2000);
        for bad in [
            "",
            "/etc/passwd",
            "../up",
            "a/../../b",
            "a//b",
            ".git/config",
            "sub/.GIT/hooks/x",
            ".git./config",
            ".git /config",
            "GIT~1/config",
            "c:/x",
            "a\\..\\b",
            "nul\0",
            "a/./b",
            "..",
            ".",
            long.as_str(),
        ] {
            assert!(!safe_path(bad), "{bad:?}");
        }
    }

    #[test]
    fn what_someone_sees_is_refused_when_it_is_not_a_place() {
        let seen = Seen { view: [800.0, 600.0], ..Default::default() };
        assert!(seen.sane());
        assert!(!Seen { offset: [f32::INFINITY, 0.0], ..seen.clone() }.sane());
        assert!(!Seen { offset: [f32::NAN, 0.0], ..seen.clone() }.sane());
        assert!(!Seen { view: [0.0, 600.0], ..seen.clone() }.sane());
        assert!(!Seen { anchor: "x".repeat(5000), ..seen.clone() }.sane());
        assert!(!Seen { pointer: Some((String::new(), [1.0e30, 0.0])), ..seen.clone() }.sane());
        let far: Seen = serde_json::from_str(
            r#"{"folder":1,"anchor":"","offset":[1e39,0],"view":[1,1],"pointer":null,"caret":null}"#,
        )
        .unwrap();
        assert!(!far.sane(), "a number too large for the wire's floats is not a place either");
    }

    #[test]
    fn a_first_message_is_small_and_nothing_is_set_aside_for_what_has_not_come() {
        let secret = [3u8; 16];
        let ((mut guest_out, _), (_, mut host_in)) = pair(&secret, &secret);
        let mut wire = Vec::new();
        write_msg(
            &mut wire,
            &mut guest_out,
            &Msg::File { folder: 1, path: "a".into(), text: "x".repeat(200_000), epoch: 0 },
        )
        .unwrap();
        let ((mut guest_out, _), (_, mut again_in)) = pair(&secret, &secret);
        let mut plain = Vec::new();
        write_msg(&mut plain, &mut guest_out, &Msg::Hello { name: "x".into(), version: VERSION }).unwrap();
        assert!(read_msg_within(&mut io::Cursor::new(plain), &mut again_in, FIRST).is_ok());
        let claimed = [(LARGEST as u32).to_be_bytes().as_slice(), &[0u8; 40]].concat();
        let cut = read_msg_within(&mut io::Cursor::new(claimed), &mut host_in, LARGEST).unwrap_err();
        assert_eq!(cut.kind(), io::ErrorKind::UnexpectedEof);
        let huge = [(LARGEST as u32 + 1).to_be_bytes().as_slice(), &[0u8; 40]].concat();
        assert_eq!(read_msg(&mut io::Cursor::new(huge), &mut host_in).unwrap_err().kind(), io::ErrorKind::InvalidData);
        let big_first = [(FIRST as u32 + 1).to_be_bytes().as_slice(), &[0u8; 40]].concat();
        assert!(read_msg_within(&mut io::Cursor::new(big_first), &mut host_in, FIRST).is_err());
        assert!(wire.len() > 100);
    }

    #[test]
    fn both_ends_greet_and_make_the_same_keys() {
        let secret = [5u8; 16];
        let mut guest = Pipe { incoming: io::Cursor::new(Vec::new()), outgoing: Vec::new() };
        let said = {
            let mine = random::<16>();
            salt_to(&mut guest.outgoing, &mine).unwrap();
            mine
        };
        let mut host = Pipe { incoming: io::Cursor::new(guest.outgoing.clone()), outgoing: Vec::new() };
        let (mut host_out, mut host_in) = greet_as_host(&mut host, &secret).unwrap();
        let answered = salt_from(&mut io::Cursor::new(host.outgoing.clone())).unwrap();
        let (mut guest_out, mut guest_in) = seals(&secret, &said, &answered);
        let mut wire = Vec::new();
        write_msg(&mut wire, &mut guest_out, &Msg::Bye { why: "done".into() }).unwrap();
        assert_eq!(read_msg(&mut io::Cursor::new(wire), &mut host_in).unwrap(), Msg::Bye { why: "done".into() });
        let mut wire = Vec::new();
        write_msg(&mut wire, &mut host_out, &Msg::FolderReady { id: 2 }).unwrap();
        assert_eq!(read_msg(&mut io::Cursor::new(wire), &mut guest_in).unwrap(), Msg::FolderReady { id: 2 });
        assert!(salt_from(&mut io::Cursor::new(b"HTTP/1.1 200 OK\r\n\r\nxxxxxxxx".to_vec())).is_err());
    }
}

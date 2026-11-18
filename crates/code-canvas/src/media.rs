use std::path::{Path, PathBuf};

pub const HANDLE: &str = "media-viewer";
pub const PLAYER: &str = "canvas-player";
pub const PEEK: &str = "ask-preview";

const FILES: &str = "https://ccres.localhost/file/";

const WHEEL: &str = r#"
<script>
  window.addEventListener('wheel', (e) => {
    e.preventDefault();
    const wheel = { x: e.deltaX, y: e.deltaY, at: [e.clientX, e.clientY], ctrl: e.ctrlKey || e.metaKey, shift: e.shiftKey };
    window.ipc.postMessage(JSON.stringify({ wheel }));
  }, { passive: false, capture: true });
</script>"#;

pub const ASK_STILL: &str = "window.__ccStill && window.__ccStill()";
pub const ASK_SETTLED: &str = "window.__ccSettled && window.__ccSettled()";

const STILL: &str = r#"
<script>
  const playing = document.getElementById('media');
  const shot = document.createElement('canvas');
  window.__ccStill = () => {
    const [w, h] = [window.innerWidth, window.innerHeight];
    if (!playing.videoWidth || !w || !h) return;
    const k = 1280 / Math.max(w, h);
    shot.width = Math.round(w * k);
    shot.height = Math.round(h * k);
    const fit = Math.min(shot.width / playing.videoWidth, shot.height / playing.videoHeight);
    const [vw, vh] = [playing.videoWidth * fit, playing.videoHeight * fit];
    const pen = shot.getContext('2d');
    pen.fillStyle = '#000';
    pen.fillRect(0, 0, shot.width, shot.height);
    pen.drawImage(playing, (shot.width - vw) / 2, (shot.height - vh) / 2, vw, vh);
    window.ipc.postMessage(JSON.stringify({ still: shot.toDataURL('image/jpeg', 0.85) }));
  };
  window.__ccSettled = () => requestAnimationFrame(() => requestAnimationFrame(() => {
    window.ipc.postMessage(JSON.stringify({ settled: true }));
  }));
</script>"#;

pub fn still(sent: &str) -> Option<(egui::ColorImage, [u32; 2])> {
    let bytes = from_base64(sent.split_once("base64,")?.1)?;
    let picture = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let size = [picture.width(), picture.height()];
    Some((egui::ColorImage::from_rgba_unmultiplied(size.map(|side| side as usize), picture.as_raw()), size))
}

fn from_base64(text: &str) -> Option<Vec<u8>> {
    let mut bytes = Vec::with_capacity(text.len() / 4 * 3);
    let (mut held, mut bits) = (0u32, 0u32);
    for c in text.bytes().filter(|c| *c != b'=') {
        let six = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        held = (held << 6) | six as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            bytes.push((held >> bits) as u8);
        }
    }
    Some(bytes)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Image,
    Video,
    Audio,
    Pdf,
}

pub fn kind(path: &Path) -> Option<Kind> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "avif" | "svg" => Kind::Image,
        "mp4" | "m4v" | "webm" | "mov" | "mkv" | "ogv" => Kind::Video,
        "mp3" | "wav" | "ogg" | "m4a" | "flac" | "opus" | "aac" => Kind::Audio,
        "pdf" => Kind::Pdf,
        _ => return None,
    })
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

pub fn file_url(path: &Path) -> String {
    let unified = path.to_string_lossy().replace('\\', "/");
    let encoded: Vec<String> = unified
        .split('/')
        .map(|part| {
            let mut out = String::new();
            for b in part.bytes() {
                match b {
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
                    _ => out.push_str(&format!("%{b:02X}")),
                }
            }
            out
        })
        .collect();
    format!("{FILES}{}", encoded.join("/"))
}

pub fn file_of(url: &str) -> Option<PathBuf> {
    let encoded = url.strip_prefix(FILES)?.split(['#', '?']).next()?.as_bytes();
    let mut bytes = Vec::with_capacity(encoded.len());
    let mut at = 0;
    while at < encoded.len() {
        if encoded[at] == b'%' {
            let hex = std::str::from_utf8(encoded.get(at + 1..at + 3)?).ok()?;
            bytes.push(u8::from_str_radix(hex, 16).ok()?);
            at += 3;
        } else {
            bytes.push(encoded[at]);
            at += 1;
        }
    }
    Some(PathBuf::from(String::from_utf8(bytes).ok()?))
}

pub fn page(path: &Path, kind: Kind, starts: bool) -> String {
    let url = escape(&file_url(path));
    let how = if starts { r#"controls autoplay preload="metadata""# } else { r#"controls preload="auto""# };
    let body = match kind {
        Kind::Image => format!(
            r#"<img id="media" src="{url}" alt="" title="Click: real size / fit">
<div id="info"></div>
<script>
  const img = document.getElementById('media');
  const info = document.getElementById('info');
  img.addEventListener('load', () => {{ info.textContent = img.naturalWidth + ' × ' + img.naturalHeight + ' px'; }});
  img.addEventListener('error', () => {{ info.textContent = 'This image could not be shown.'; }});
  img.addEventListener('click', () => document.body.classList.toggle('actual'));
</script>"#
        ),
        Kind::Video => {
            format!(r#"<video id="media" src="{url}" {how}></video>{WHEEL}{STILL}"#)
        }
        Kind::Audio => format!(r#"<audio id="media" src="{url}" {how}></audio>{WHEEL}"#),
        Kind::Pdf => format!(r#"<iframe id="media" class="page" src="{url}"></iframe>"#),
    };
    format!(
        r#"<!DOCTYPE html>
<html><head><meta charset="utf-8">
<style>
  html, body {{ height: 100%; margin: 0; background: #0b0c0f; color: #8a8e99; font: 12px "Segoe UI", system-ui, sans-serif; }}
  body {{ display: flex; flex-direction: column; align-items: center; justify-content: center; overflow: auto; }}
  img {{ max-width: 100%; max-height: calc(100% - 28px); object-fit: contain; cursor: zoom-in; image-rendering: auto;
        background: repeating-conic-gradient(#16171c 0% 25%, #101115 0% 50%) 50% / 16px 16px; }}
  body.actual {{ display: block; }}
  body.actual img {{ max-width: none; max-height: none; cursor: zoom-out; }}
  video {{ display: block; width: 100%; height: 100%; object-fit: contain; background: #000; }}
  audio {{ width: min(90%, 520px); }}
  iframe.page {{ width: 100%; height: 100%; border: 0; }}
  #info {{ padding: 6px; }}
</style></head>
<body>{body}</body></html>"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_by_extension() {
        assert_eq!(kind(Path::new("a/b/Shot.PNG")), Some(Kind::Image));
        assert_eq!(kind(Path::new("clip.mp4")), Some(Kind::Video));
        assert_eq!(kind(Path::new("voice.m4a")), Some(Kind::Audio));
        assert_eq!(kind(Path::new("paper.pdf")), Some(Kind::Pdf));
        assert_eq!(kind(Path::new("main.rs")), None);
        assert_eq!(kind(Path::new("Makefile")), None);
    }

    #[test]
    fn pages_point_at_the_file_safely() {
        let url = file_url(Path::new("C:\\My Files\\a&b \"x\".png"));
        assert_eq!(url, "https://ccres.localhost/file/C%3A/My%20Files/a%26b%20%22x%22.png");
        let html = page(Path::new("C:/v/clip one.mp4"), Kind::Video, true);
        assert!(html.contains(r#"<video id="media" src="https://ccres.localhost/file/C%3A/v/clip%20one.mp4" controls"#));
        assert!(page(Path::new("a.png"), Kind::Image, true).contains("<img"));
        assert!(html.contains("autoplay") && !page(Path::new("a.mp4"), Kind::Video, false).contains("autoplay"));
    }

    #[test]
    fn a_still_is_read_from_what_the_page_sends() {
        assert_eq!(from_base64("TWFu").as_deref(), Some(&b"Man"[..]));
        assert_eq!(from_base64("TWE=").as_deref(), Some(&b"Ma"[..]));
        assert_eq!(from_base64("TQ==").as_deref(), Some(&b"M"[..]));
        assert_eq!(from_base64("T W"), None);
        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(3, 2, image::Rgba([10, 20, 30, 255]))
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let sent: String = png.into_inner().chunks(3).map(to_base64).collect();
        let (picture, size) = still(&format!("data:image/png;base64,{sent}")).unwrap();
        assert_eq!((size, picture.pixels[0]), ([3, 2], egui::Color32::from_rgb(10, 20, 30)));
        assert!(still("data:image/jpeg;base64,AAAA").is_none() && still("nothing").is_none());
    }

    fn to_base64(three: &[u8]) -> String {
        const SIGNS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let held = three.iter().fold(0u32, |held, b| (held << 8) | *b as u32) << (8 * (3 - three.len()));
        let signs = (0..4).map(|at| match at <= three.len() {
            true => SIGNS[(held >> (18 - 6 * at)) as usize & 63] as char,
            false => '=',
        });
        signs.collect()
    }

    #[test]
    fn a_link_leads_back_to_its_file() {
        let file = Path::new("C:/My Files/a&b \"x\"/léeme.md");
        assert_eq!(file_of(&file_url(file)).as_deref(), Some(file));
        assert_eq!(file_of(&format!("{}#setup", file_url(file))).as_deref(), Some(file));
        assert_eq!(file_of("https://example.com/file/a.md"), None);
        assert_eq!(file_of("https://ccres.localhost/file/a%zz.md"), None);
    }
}

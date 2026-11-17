use std::path::Path;

pub const HANDLE: &str = "media-viewer";

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
    format!("https://ccres.localhost/file/{}", encoded.join("/"))
}

pub fn page(path: &Path, kind: Kind) -> String {
    let url = escape(&file_url(path));
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
        Kind::Video => format!(r#"<video id="media" src="{url}" controls preload="metadata"></video>"#),
        Kind::Audio => format!(r#"<audio id="media" src="{url}" controls preload="metadata"></audio>"#),
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
  video {{ max-width: 100%; max-height: 100%; background: #000; }}
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
        let html = page(Path::new("C:/v/clip one.mp4"), Kind::Video);
        assert!(html.contains(r#"<video id="media" src="https://ccres.localhost/file/C%3A/v/clip%20one.mp4" controls"#));
        assert!(page(Path::new("a.png"), Kind::Image).contains("<img"));
    }
}

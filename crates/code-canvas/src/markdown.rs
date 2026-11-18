use std::collections::HashMap;
use std::hash::{BuildHasher, Hasher};
use std::path::Path;

use canvas_render::theme;
use pulldown_cmark::{html, CowStr, Event, Options, Parser, Tag, TagEnd};

pub fn is_markdown(path: &Path) -> bool {
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    matches!(extension.as_str(), "md" | "markdown" | "mdown" | "mkd")
}

fn slug(title: &str) -> String {
    title
        .trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
            _ => None,
        })
        .collect()
}

const ACTS_ON_ITS_OWN: [&str; 9] =
    ["<meta", "<base", "<link", "<iframe", "<object", "<embed", "<form", "<script", "<style"];

fn inert(event: Event) -> Event {
    match event {
        Event::Html(markup) | Event::InlineHtml(markup)
            if ACTS_ON_ITS_OWN.iter().any(|tag| markup.to_ascii_lowercase().contains(tag)) =>
        {
            Event::Text(markup)
        }
        other => other,
    }
}

pub fn body(text: &str) -> String {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_GFM;
    let mut events: Vec<Event> = Parser::new_ext(text, options).map(inert).collect();
    let mut taken: HashMap<String, usize> = HashMap::new();
    let mut at = 0;
    while at < events.len() {
        if matches!(events[at], Event::Start(Tag::Heading { .. })) {
            let mut title = String::new();
            let mut end = at + 1;
            while end < events.len() && !matches!(events[end], Event::End(TagEnd::Heading(_))) {
                if let Event::Text(part) | Event::Code(part) = &events[end] {
                    title.push_str(part);
                }
                end += 1;
            }
            let plain = slug(&title);
            let repeats = taken.entry(plain.clone()).or_insert(0);
            let anchor = if *repeats == 0 { plain } else { format!("{plain}-{repeats}") };
            *repeats += 1;
            if let Event::Start(Tag::Heading { id, .. }) = &mut events[at] {
                if id.is_none() {
                    *id = Some(CowStr::from(anchor));
                }
            }
            at = end;
        }
        at += 1;
    }
    let mut out = String::new();
    html::push_html(&mut out, events.into_iter());
    out
}

fn colour(rgba: u32) -> String {
    let [r, g, b, a] = theme::unpack(rgba);
    format!("rgba({r},{g},{b},{:.3})", a as f32 / 255.0)
}

const STYLE: &str = r#"
  html { background: var(--card); }
  body { margin: 0; color: var(--text); font: 14px/1.6 "Segoe UI", system-ui, sans-serif; overflow-wrap: break-word; }
  article { max-width: 860px; margin: 0 auto; padding: 16px clamp(14px, 5vw, 32px) 48px; }
  article > :first-child { margin-top: 0; }
  h1, h2, h3, h4, h5, h6 { margin: 24px 0 14px; font-weight: 600; line-height: 1.25; }
  h1 { font-size: 2em; padding-bottom: 0.3em; border-bottom: 1px solid var(--border); }
  h2 { font-size: 1.5em; padding-bottom: 0.3em; border-bottom: 1px solid var(--border); }
  h3 { font-size: 1.25em; }
  h4 { font-size: 1em; }
  h5 { font-size: 0.875em; }
  h6 { font-size: 0.85em; color: var(--dim); }
  p, ul, ol, pre, table, blockquote, dl { margin: 0 0 16px; }
  a { color: var(--accent); text-decoration: none; }
  a:hover { text-decoration: underline; }
  ul, ol { padding-left: 2em; }
  li + li { margin-top: 0.25em; }
  li > ul, li > ol { margin: 0.25em 0 0; }
  li:has(> input[type="checkbox"]) { list-style: none; margin-left: -1.4em; }
  input[type="checkbox"] { margin: 0 0.4em 0.2em 0; vertical-align: middle; accent-color: var(--accent); }
  code { font: 85% Consolas, "Cascadia Mono", monospace; padding: 0.2em 0.4em; border-radius: 6px; background: var(--well); }
  pre { padding: 14px 16px; overflow: auto; border-radius: 8px; background: var(--well); line-height: 1.45; }
  pre code { padding: 0; background: none; font-size: 13px; }
  blockquote { padding: 0 1em; color: var(--dim); border-left: 4px solid var(--border); }
  blockquote > :last-child { margin-bottom: 0; }
  blockquote[class^="markdown-alert"] { padding: 8px 1em; color: var(--text); border-left-color: var(--accent); }
  blockquote.markdown-alert-warning, blockquote.markdown-alert-caution { border-left-color: var(--warn); }
  table { display: block; width: max-content; max-width: 100%; overflow: auto; border-collapse: collapse; }
  th, td { padding: 6px 13px; border: 1px solid var(--border); }
  th { font-weight: 600; background: var(--row); }
  tr:nth-child(2n) td { background: var(--row); }
  img, video { max-width: 100%; }
  hr { height: 2px; margin: 24px 0; border: 0; background: var(--border); }
  del { color: var(--dim); }
  kbd { padding: 2px 5px; font: 12px Consolas, monospace; border: 1px solid var(--border); border-radius: 5px; background: var(--well); }
  details { margin-bottom: 16px; }
  summary { cursor: pointer; }
  sup.footnote-reference a { padding: 0 2px; }
  .footnote-definition { display: flex; gap: 8px; color: var(--dim); font-size: 0.9em; }
  .footnote-definition p { margin: 0 0 6px; }
"#;

const KEEP_SCROLL: &str = r#"
  const key = 'scroll ' + FILE;
  const saved = Number(sessionStorage.getItem(key) || 0);
  if (saved > 0 && !location.hash) scrollTo(0, saved);
  addEventListener('scroll', () => sessionStorage.setItem(key, String(scrollY)), { passive: true });
"#;

pub fn page(text: &str, file: &Path) -> String {
    let folder = crate::media::file_url(file.parent().unwrap_or(Path::new("")));
    document(&body(text), &format!("{folder}/"), &file.to_string_lossy(), "")
}

/// A page of the app's look around `article` (HTML already made safe, such as `body`'s),
/// whose relative links start at `base`; `key` names where it was scrolled to, kept while
/// the app runs; `style` is added to the app's.
pub fn document(article: &str, base: &str, key: &str, style: &str) -> String {
    let t = theme::get();
    let nonce = format!("{:016x}", std::collections::hash_map::RandomState::new().build_hasher().finish());
    let base = base.replace(['"', '<', '>'], "");
    let name = serde_json::to_string(key).unwrap_or_default().replace('<', "\\u003c");
    let variables = format!(
        ":root {{ --card: var(--vscode-editor-background, {}); --text: var(--vscode-foreground, {}); \
         --dim: var(--vscode-descriptionForeground, {}); --border: var(--vscode-panel-border, {}); \
         --well: var(--vscode-input-background, {}); --row: var(--vscode-dropdown-background, {}); \
         --accent: var(--vscode-focusBorder, {}); --warn: var(--vscode-charts-yellow, {}); }}",
        colour(t.card),
        colour(t.text),
        colour(t.text_dim),
        colour(t.border),
        colour(t.well),
        colour(t.row),
        colour(t.accent),
        colour(t.problem_warning),
    );
    let policy = format!(
        "default-src 'none'; img-src https: http: data:; media-src https: http:; style-src 'unsafe-inline'; \
         script-src 'nonce-{nonce}'"
    );
    format!(
        "<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\">\n\
         <meta http-equiv=\"Content-Security-Policy\" content=\"{policy}\">\n\
         <base href=\"{base}\">\n\
         <style>{variables}{STYLE}{style}</style></head>\n\
         <body><article>{article}</article>\n\
         <script nonce=\"{nonce}\">const FILE = {name};{KEEP_SCROLL}</script></body></html>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_by_extension() {
        assert!(is_markdown(Path::new("README.md")));
        assert!(is_markdown(Path::new("docs/Guide.MARKDOWN")));
        assert!(!is_markdown(Path::new("main.rs")));
        assert!(!is_markdown(Path::new("md")));
    }

    #[test]
    fn renders_what_github_does() {
        let html = body("# Title\n\n- [x] done\n- [ ] todo\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n~~old~~ `code`\n");
        assert!(html.contains("<h1 id=\"title\">Title</h1>"));
        assert!(html.contains("type=\"checkbox\"") && html.contains("checked"));
        assert!(html.contains("<table>") && html.contains("<del>old</del>") && html.contains("<code>code</code>"));
    }

    #[test]
    fn headings_can_be_linked_to() {
        let html = body("## Qué hace\n\n## Setup & `run`\n\n## Setup & run\n");
        assert!(html.contains("id=\"qué-hace\""));
        assert!(html.contains("id=\"setup--run\""));
        assert!(html.contains("id=\"setup--run-1\""));
    }

    #[test]
    fn a_page_cannot_run_what_the_file_brings() {
        let html = page("<script>alert(1)</script>\n\n![shot](img/a.png)\n", Path::new("C:/proj/docs/READ ME.md"));
        assert!(html.contains("script-src 'nonce-"));
        assert!(html.contains("<base href=\"https://ccres.localhost/file/C%3A/proj/docs/\">"));
        let nonce = html.split("script-src 'nonce-").nth(1).unwrap().split('\'').next().unwrap();
        assert_eq!(html.matches(&format!("nonce=\"{nonce}\"")).count(), 1);
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;") && !html.contains("<script>alert"));
    }

    #[test]
    fn markup_that_acts_on_its_own_is_shown_as_text() {
        let html = body(
            "<META http-equiv=\"refresh\" content=\"0; url=https://example.com\">

A <iframe src=x></iframe> b
",
        );
        assert!(!html.to_lowercase().contains("<meta") && !html.contains("<iframe"));
        assert!(html.contains("&lt;META") && html.contains("&lt;iframe"));
        let kept = body(
            "<details><summary>More</summary>

Hidden

</details>

<img src=\"a.png\" width=\"40\">
",
        );
        assert!(kept.contains("<details><summary>More</summary>") && kept.contains("<img src=\"a.png\" width=\"40\">"));
    }
}

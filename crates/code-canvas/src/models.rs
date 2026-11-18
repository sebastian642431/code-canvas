use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};

use serde_json::{json, Value};

const LOOKS: [u64; 3] = [256 << 10, 2 << 20, 8 << 20];
const LONGEST_NAME: usize = 24;

fn all_digits(word: &str) -> bool {
    !word.is_empty() && word.chars().all(|c| c.is_ascii_digit())
}

fn a_version(word: &str) -> bool {
    !word.is_empty() && word.len() <= 4 && word.chars().all(|c| c.is_ascii_digit() || c == '.')
}

fn capital(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn shortened(text: &str) -> String {
    match text.chars().count() > LONGEST_NAME {
        true => format!("{}…", text.chars().take(LONGEST_NAME - 1).collect::<String>()),
        false => text.to_string(),
    }
}

pub fn name(id: &str) -> String {
    let id = id.trim();
    let id = id.rsplit('/').next().unwrap_or(id);
    let id = id.split('[').next().unwrap_or(id).trim();
    if id.is_empty() {
        return String::new();
    }
    let lower = id.to_ascii_lowercase();
    let mut base = lower.as_str();
    if let Some(at) = base.find("anthropic.") {
        base = &base[at + "anthropic.".len()..];
    }
    base = base.split(['@', ':']).next().unwrap_or(base);
    let mut words: Vec<&str> = base.split(['-', '_', ' ']).filter(|word| !word.is_empty()).collect();
    loop {
        match words.last() {
            Some(word) if word.len() >= 6 && all_digits(word) => words.pop(),
            Some(word) if word.len() >= 2 && word.starts_with('v') && all_digits(&word[1..]) => words.pop(),
            Some(&"latest" | &"preview") if words.len() > 2 => words.pop(),
            _ => break,
        };
    }
    let n = words.len();
    if n >= 4 && words[n - 3..].iter().zip([4, 2, 2]).all(|(word, length)| word.len() == length && all_digits(word)) {
        words.truncate(n - 3);
    }
    match words.as_slice() {
        ["claude", rest @ ..] if !rest.is_empty() => {
            let version: Vec<&str> = rest.iter().copied().filter(|word| a_version(word)).collect();
            let named: Vec<String> = rest.iter().filter(|word| !a_version(word)).map(|word| capital(word)).collect();
            let family = match named.is_empty() {
                true => "Claude".to_string(),
                false => named.join(" "),
            };
            match version.is_empty() {
                true => family,
                false => format!("{family} {}", version.join(".")),
            }
        }
        [family @ ("opus" | "sonnet" | "haiku" | "fable")] => capital(family),
        ["gpt", rest @ ..] => {
            let (head, tail) = match rest.first() {
                Some(first) if first.starts_with(|c: char| c.is_ascii_digit()) => (format!("GPT-{first}"), &rest[1..]),
                _ => ("GPT".to_string(), rest),
            };
            let more: Vec<String> = tail.iter().map(|word| capital(word)).collect();
            match more.is_empty() {
                true => head,
                false => format!("{head} {}", more.join(" ")),
            }
        }
        _ => shortened(id),
    }
}

pub fn in_tail(tail: &[u8], from_the_start: bool) -> Option<String> {
    let lines: Vec<&[u8]> = tail.split(|byte| *byte == b'\n').collect();
    let whole = if from_the_start { &lines[..] } else { &lines[1.min(lines.len())..] };
    let has = |line: &[u8], word: &[u8]| line.windows(word.len()).any(|part| part == word);
    for line in whole.iter().rev() {
        if !has(line, b"\"assistant\"") || !has(line, b"\"model\"") {
            continue;
        }
        let Ok(entry) = serde_json::from_slice::<Value>(line) else { continue };
        if entry.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let model = entry.pointer("/message/model").and_then(Value::as_str).map(str::trim);
        if let Some(model) = model.filter(|model| !model.is_empty() && !model.starts_with('<')) {
            return Some(model.to_string());
        }
    }
    None
}

#[cfg(test)]
pub fn last_in(path: &Path) -> Option<String> {
    read_back(path).0
}

/// The title of a session in a tail of its transcript: the one it was given by hand, else the last one Claude
/// Code made up for it.
pub fn title_in(tail: &[u8], from_the_start: bool) -> Option<String> {
    let lines: Vec<&[u8]> = tail.split(|byte| *byte == b'\n').collect();
    let whole = if from_the_start { &lines[..] } else { &lines[1.min(lines.len())..] };
    let has = |line: &[u8], word: &[u8]| line.windows(word.len()).any(|part| part == word);
    let mut made_up = None;
    for line in whole.iter().rev() {
        let (by_hand, made) = (has(line, b"\"custom-title\""), has(line, b"\"ai-title\""));
        if !by_hand && (!made || made_up.is_some()) {
            continue;
        }
        let Ok(entry) = serde_json::from_slice::<Value>(line) else { continue };
        let key = match entry.get("type").and_then(Value::as_str) {
            Some("custom-title") => "customTitle",
            Some("ai-title") => "aiTitle",
            _ => continue,
        };
        let title = entry.get(key).and_then(Value::as_str).map(str::trim).filter(|title| !title.is_empty());
        match (key, title) {
            ("customTitle", Some(title)) => return Some(title.to_string()),
            (_, Some(title)) => made_up = Some(title.to_string()),
            _ => {}
        }
    }
    made_up
}

/// The model of the last answer in a transcript and the title of its session, read from its end and further
/// back while either is missing.
pub fn read_back(path: &Path) -> (Option<String>, Option<String>) {
    let (mut model, mut title) = (None, None);
    let Ok(mut file) = File::open(path) else { return (model, title) };
    let length = file.metadata().map(|meta| meta.len()).unwrap_or_default();
    for look in LOOKS {
        let from = length.saturating_sub(look);
        let mut tail = Vec::new();
        if file.seek(SeekFrom::Start(from)).is_err() || (&mut file).take(look).read_to_end(&mut tail).is_err() {
            break;
        }
        model = model.or_else(|| in_tail(&tail, from == 0));
        title = title.or_else(|| title_in(&tail, from == 0));
        if from == 0 || (model.is_some() && title.is_some()) {
            break;
        }
    }
    (model, title)
}

pub fn of_helper(main: &Path, helper: &str) -> PathBuf {
    let own = format!("agent-{helper}");
    if main.file_stem().is_some_and(|stem| stem.to_string_lossy() == own) {
        return main.to_path_buf();
    }
    let session = main.file_stem().map(|stem| stem.to_os_string()).unwrap_or_default();
    main.with_file_name(session).join("subagents").join(format!("{own}.jsonl"))
}

pub struct Reader {
    asks: Sender<(String, PathBuf)>,
}

impl Reader {
    pub fn start(tell: Sender<Value>) -> Option<Reader> {
        let (asks, asked) = mpsc::channel::<(String, PathBuf)>();
        let spawned = std::thread::Builder::new().name("assistant models".to_string()).spawn(move || {
            while let Ok(first) = asked.recv() {
                let mut all = vec![first];
                all.extend(asked.try_iter());
                let mut seen = std::collections::HashSet::new();
                let latest: Vec<(String, PathBuf)> =
                    all.into_iter().rev().filter(|(key, _)| seen.insert(key.clone())).collect();
                for (key, path) in latest {
                    let (model, title) = read_back(&path);
                    if model.is_none() && title.is_none() {
                        continue;
                    }
                    if tell.send(json!({ "model_of": key, "model": model, "title": title })).is_err() {
                        return;
                    }
                    crate::ext::wake_ui();
                }
            }
        });
        match spawned {
            Ok(_) => Some(Reader { asks }),
            Err(why) => {
                log::warn!("the models of the assistants will not be shown: {why}");
                None
            }
        }
    }

    pub fn ask(&self, key: &str, transcript: PathBuf) {
        let _ = self.asks.send((key.to_string(), transcript));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_model_is_named_by_its_family_and_version() {
        let named = |id: &str| name(id);
        assert_eq!(named("claude-opus-5-5"), "Opus 5.5");
        assert_eq!(named("claude-sonnet-5-5"), "Sonnet 5.5");
        assert_eq!(named("claude-haiku-5-5"), "Haiku 5.5");
        assert_eq!(named("claude-fable-5-2"), "Fable 5.2");
        assert_eq!(named("claude-opus-7-1"), "Opus 7.1", "a version not known today is read the same way");
        assert_eq!(named("claude-sonnet-4-5-20250929"), "Sonnet 4.5");
        assert_eq!(named("claude-3-5-sonnet-20241022"), "Sonnet 3.5");
        assert_eq!(named("us.anthropic.claude-sonnet-4-5-20250929-v1:0"), "Sonnet 4.5");
        assert_eq!(named("claude-opus-4-1@20250805"), "Opus 4.1");
        assert_eq!(named("claude-opus-5-5[1m]"), "Opus 5.5");
        assert_eq!(named("anthropic/claude-haiku-5-5"), "Haiku 5.5");
        assert_eq!(named("claude-2.1"), "Claude 2.1");
        assert_eq!(named("opus"), "Opus");
        assert_eq!(named("gpt-5"), "GPT-5");
        assert_eq!(named("gpt-5.1-codex-max"), "GPT-5.1 Codex Max");
        assert_eq!(named("gpt-5-codex"), "GPT-5 Codex");
        assert_eq!(named("gpt-4o-2024-08-06"), "GPT-4o");
        assert_eq!(named("o4-mini"), "o4-mini");
        assert_eq!(named("some-very-long-model-name-from-elsewhere"), "some-very-long-model-na…");
        assert_eq!(named("  "), "");
    }

    fn line(kind: &str, model: Option<&str>) -> String {
        let mut entry = json!({ "type": kind, "sessionId": "s1", "message": { "role": kind, "content": [] } });
        if let Some(model) = model {
            entry["message"]["model"] = json!(model);
        }
        format!("{entry}\n")
    }

    #[test]
    fn the_model_is_the_one_of_the_last_answer_in_a_transcript() {
        let mut text = line("user", None) + &line("assistant", Some("claude-sonnet-5-5"));
        text += &line("assistant", Some("claude-opus-5-5"));
        text += &line("assistant", Some("<synthetic>"));
        text += &line("user", None);
        let tool = json!({ "type": "assistant", "message": { "content": [{ "type": "tool_use", "input": { "model": "haiku" } }] } });
        text += &format!("{tool}\n");
        text += "{\"type\":\"assistant\",\"message\":{\"model\":\"claude-cut";
        assert_eq!(in_tail(text.as_bytes(), true).as_deref(), Some("claude-opus-5-5"));
        assert_eq!(in_tail(line("user", None).as_bytes(), true), None);
        let cut = &text.as_bytes()[20..];
        assert_eq!(in_tail(cut, false).as_deref(), Some("claude-opus-5-5"), "the first piece of a tail is not a line");
    }

    #[test]
    fn a_session_is_named_by_its_title_and_a_title_given_by_hand_wins() {
        let made = |title: &str| format!("{}\n", json!({ "type": "ai-title", "aiTitle": title, "sessionId": "s1" }));
        let named = format!("{}\n", json!({ "type": "custom-title", "customTitle": "Mine", "sessionId": "s1" }));
        let text = line("user", None) + &made("Old one") + &line("assistant", Some("x")) + &made("Tab drag");
        assert_eq!(title_in(text.as_bytes(), true).as_deref(), Some("Tab drag"), "the last made-up title");
        let text = named.clone() + &text;
        assert_eq!(title_in(text.as_bytes(), true).as_deref(), Some("Mine"), "a name given by hand comes first");
        assert_eq!(title_in(line("user", None).as_bytes(), true), None);
        let quoted = line("user", None).replace("\"content\":[]", "\"content\":\"ai-title\"");
        assert_eq!(title_in(quoted.as_bytes(), true), None, "the words in a message are not a title");
    }

    #[test]
    fn a_transcript_is_read_from_its_end_and_further_back_when_needed() {
        let dir = std::env::temp_dir().join(format!("code-canvas-test-models-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let main = dir.join("project").join("s1.jsonl");
        let helper = of_helper(&main, "a42");
        assert_eq!(helper, dir.join("project").join("s1").join("subagents").join("agent-a42.jsonl"));
        assert_eq!(of_helper(&helper, "a42"), helper, "a helper's own transcript is kept");
        std::fs::create_dir_all(helper.parent().unwrap()).unwrap();
        let long = json!({ "type": "user", "toolUseResult": "x".repeat(300 << 10) });
        let text = line("assistant", Some("claude-opus-5-5")) + &format!("{long}\n") + &line("user", None);
        std::fs::write(&main, text).unwrap();
        std::fs::write(&helper, line("assistant", Some("claude-haiku-5-5"))).unwrap();
        assert_eq!(last_in(&main).as_deref(), Some("claude-opus-5-5"));
        assert_eq!(last_in(&helper).as_deref(), Some("claude-haiku-5-5"));
        assert_eq!(last_in(&dir.join("none.jsonl")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

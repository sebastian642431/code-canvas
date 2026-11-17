use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

const MAX_DIFF: usize = 30_000;
const LIMIT: Duration = Duration::from_secs(120);

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub provider: String,
    pub claude_model: String,
    pub codex_model: String,
    pub custom: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            provider: "auto".into(),
            claude_model: "haiku".into(),
            codex_model: String::new(),
            custom: String::new(),
        }
    }
}

fn path() -> PathBuf {
    crate::session::data_dir().join("commit.json")
}

pub fn load() -> Settings {
    std::fs::read_to_string(path()).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

pub fn save(settings: &Settings) {
    let result = serde_json::to_string_pretty(settings)
        .map_err(std::io::Error::other)
        .and_then(|json| std::fs::write(path(), json));
    if let Err(e) = result {
        log::warn!("could not save the commit message settings: {e}");
    }
}

pub struct Change {
    pub staged: bool,
    pub diff: String,
    pub recent: Vec<String>,
}

pub fn prompt(repository: &str, change: &Change) -> String {
    let mut diff = change.diff.as_str();
    let mut cut = "";
    if diff.len() > MAX_DIFF {
        let mut end = MAX_DIFF;
        while !diff.is_char_boundary(end) {
            end -= 1;
        }
        diff = &diff[..end];
        cut = "\n[diff truncated]";
    }
    let what = if change.staged { "staged changes" } else { "working tree changes, nothing staged" };
    let recent = if change.recent.is_empty() { "(none yet)".to_string() } else { change.recent.join("\n") };
    format!(
        "You are writing a git commit message for the repository \"{repository}\".\n\
         \n\
         Rules:\n\
         - A single subject line. No body.\n\
         - Imperative, at most 72 characters, in the language and style of the recent commits below \
         (Conventional Commits only if they use it; English when there are none).\n\
         - Describe only what this diff changes: what it achieves, not a restatement of the code.\n\
         - Use the identifiers and terms of the code.\n\
         - No justifications or explanations of why the change is correct.\n\
         - Output ONLY the subject line. No quotes, no markdown fences, no Co-Authored-By lines.\n\
         \n\
         Recent commits:\n\
         {recent}\n\
         \n\
         Diff ({what}):\n\
         {diff}{cut}"
    )
}

pub fn clean(output: &str) -> String {
    let line = output.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with("```")).unwrap_or_default();
    let quoted = line.len() >= 2
        && [('"', '"'), ('\'', '\''), ('`', '`')].iter().any(|(a, b)| line.starts_with(*a) && line.ends_with(*b));
    if quoted {
        line[1..line.len() - 1].trim().to_string()
    } else {
        line.to_string()
    }
}

pub fn words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let (mut quote, mut started) = (None, false);
    for c in line.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => word.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                started = true;
            }
            (None, c) if c.is_whitespace() => {
                if started || !word.is_empty() {
                    out.push(std::mem::take(&mut word));
                }
                started = false;
            }
            (None, c) => word.push(c),
        }
    }
    if started || !word.is_empty() {
        out.push(word);
    }
    out
}

pub fn find_program(name: &str) -> Option<PathBuf> {
    let given = Path::new(name);
    if given.components().count() > 1 {
        return given.is_file().then(|| given.to_path_buf());
    }
    let endings: Vec<String> = if cfg!(windows) {
        let known = std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT".into());
        std::iter::once(String::new()).chain(known.split(';').map(str::to_lowercase)).collect()
    } else {
        vec![String::new()]
    };
    let folders = std::env::var_os("PATH")?;
    std::env::split_paths(&folders).find_map(|folder| {
        endings
            .iter()
            .map(|ending| folder.join(format!("{name}{ending}")))
            .find(|candidate| candidate.is_file() && (!cfg!(windows) || candidate.extension().is_some()))
    })
}

#[derive(Debug, PartialEq)]
pub struct Invocation {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub answer_file: Option<PathBuf>,
}

pub fn invocation(
    settings: &Settings,
    find: &dyn Fn(&str) -> Option<PathBuf>,
    scratch: &Path,
) -> Result<Invocation, String> {
    let text = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<String>>();
    let claude = |program: PathBuf| {
        let mut args = text(&["-p", "--no-session-persistence", "--tools", ""]);
        if !settings.claude_model.is_empty() {
            args.extend(text(&["--model", &settings.claude_model]));
        }
        args.extend(text(&[
            "--system-prompt",
            "You write git commit messages. Follow the rules exactly and output only the commit message.",
            "--strict-mcp-config",
            "--disable-slash-commands",
            "--setting-sources",
            "",
            "--output-format",
            "text",
        ]));
        Invocation { program, args, answer_file: None }
    };
    let codex = |program: PathBuf| {
        let mut args = text(&["exec", "--sandbox", "read-only", "--skip-git-repo-check", "--ephemeral"]);
        if !settings.codex_model.is_empty() {
            args.extend(text(&["--model", &settings.codex_model]));
        }
        args.extend(text(&["--output-last-message", &scratch.to_string_lossy(), "-"]));
        Invocation { program, args, answer_file: Some(scratch.to_path_buf()) }
    };
    let missing = |name: &str| format!("{name} is not installed (it was not found on the PATH)");
    match settings.provider.as_str() {
        "claude" => find("claude").map(claude).ok_or_else(|| missing("Claude Code")),
        "codex" => find("codex").map(codex).ok_or_else(|| missing("Codex")),
        "custom" => {
            let mut parts = words(&settings.custom).into_iter();
            let name = parts.next().ok_or("no command is set for commit messages: write one in the settings")?;
            let program = find(&name).ok_or_else(|| format!("\"{name}\" was not found"))?;
            Ok(Invocation { program, args: parts.collect(), answer_file: None })
        }
        _ => find("claude").map(claude).or_else(|| find("codex").map(codex)).ok_or_else(|| {
            "neither Claude Code nor Codex is installed: install one, or set a command of your own in the settings"
                .to_string()
        }),
    }
}

fn run(invocation: &Invocation, root: &Path, request: &str) -> Result<String, String> {
    let mut command = Command::new(&invocation.program);
    command.args(&invocation.args).current_dir(root);
    command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let name = invocation.program.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut child = command.spawn().map_err(|e| format!("{name} could not be started: {e}"))?;
    let mut stdin = child.stdin.take();
    let request = request.to_string();
    let feed = std::thread::spawn(move || {
        if let Some(stdin) = stdin.as_mut() {
            let _ = stdin.write_all(request.as_bytes());
        }
    });
    let drain = |mut pipe: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = pipe.read_to_end(&mut bytes);
            String::from_utf8_lossy(&bytes).into_owned()
        })
    };
    let out = child.stdout.take().map(|p| drain(Box::new(p)));
    let err = child.stderr.take().map(|p| drain(Box::new(p)));
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() > LIMIT => {
                let _ = child.kill();
                return Err(format!("{name} did not answer in {} seconds", LIMIT.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(60)),
            Err(e) => return Err(e.to_string()),
        }
    };
    let _ = feed.join();
    let joined = |h: Option<std::thread::JoinHandle<String>>| h.and_then(|h| h.join().ok()).unwrap_or_default();
    let (printed, complaints) = (joined(out), joined(err));
    if !status.success() {
        let why = [complaints.trim(), printed.trim()].into_iter().find(|t| !t.is_empty()).unwrap_or("it failed");
        let tail: Vec<&str> = why.lines().rev().take(3).collect();
        return Err(format!("{name}: {}", tail.into_iter().rev().collect::<Vec<_>>().join(" ")));
    }
    let answer = match &invocation.answer_file {
        Some(file) => {
            let text = std::fs::read_to_string(file).unwrap_or_default();
            let _ = std::fs::remove_file(file);
            text
        }
        None => printed,
    };
    let message = clean(&answer);
    if message.is_empty() {
        Err(format!("{name} gave no message"))
    } else {
        Ok(message)
    }
}

pub fn write(root: &Path, settings: &Settings) -> Result<String, String> {
    let change = crate::git::change_for_message(root).ok_or("there is nothing to commit")?;
    let repository = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let scratch = std::env::temp_dir().join(format!("code-canvas-commit-{}.txt", std::process::id()));
    let invocation = invocation(settings, &find_program, &scratch)?;
    run(&invocation, root, &prompt(&repository, &change))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_carries_the_change() {
        let change = Change {
            staged: true,
            diff: "+let x = 1;".into(),
            recent: vec!["Fix the parser".into(), "Add tests".into()],
        };
        let text = prompt("demo", &change);
        assert!(text.contains("repository \"demo\""));
        assert!(text.contains("Recent commits:\nFix the parser\nAdd tests\n"));
        assert!(text.ends_with("Diff (staged changes):\n+let x = 1;"));

        let big = Change { staged: false, diff: "é".repeat(MAX_DIFF), recent: Vec::new() };
        let text = prompt("demo", &big);
        assert!(text.contains("nothing staged"));
        assert!(text.contains("(none yet)"));
        assert!(text.ends_with("[diff truncated]"));
        assert!(text.len() < MAX_DIFF + 2000);
    }

    #[test]
    fn the_message_is_the_first_real_line() {
        assert_eq!(clean("Fix the parser\n"), "Fix the parser");
        assert_eq!(clean("```\nfix: the parser\n```\n"), "fix: the parser");
        assert_eq!(clean("\n\n  \"Add tests\"  \nand more words"), "Add tests");
        assert_eq!(clean("`feat: x`"), "feat: x");
        assert_eq!(clean("Don't quote 'this'"), "Don't quote 'this'");
        assert_eq!(clean("   \n"), "");
    }

    #[test]
    fn command_lines_into_words() {
        assert_eq!(words("ollama run llama3"), vec!["ollama", "run", "llama3"]);
        assert_eq!(
            words(r#"  "C:\Program Files\ai\ai.exe"  --flag "two words" '' "#),
            vec![r"C:\Program Files\ai\ai.exe", "--flag", "two words", ""]
        );
        assert!(words("   ").is_empty());
    }

    #[test]
    fn which_command_is_run() {
        let scratch = Path::new("/tmp/answer.txt");
        let found = |installed: &'static [&'static str]| {
            move |name: &str| installed.contains(&name).then(|| PathBuf::from(format!("/bin/{name}")))
        };
        let settings = |provider: &str| Settings { provider: provider.into(), ..Default::default() };

        let both = invocation(&settings("auto"), &found(&["claude", "codex"]), scratch).unwrap();
        assert_eq!(both.program, PathBuf::from("/bin/claude"));
        assert!(both.args.starts_with(&["-p".to_string()]) && both.args.contains(&"haiku".to_string()));
        assert_eq!(both.answer_file, None);

        let codex = invocation(&settings("auto"), &found(&["codex"]), scratch).unwrap();
        assert_eq!(codex.program, PathBuf::from("/bin/codex"));
        assert_eq!(codex.args[..3], ["exec", "--sandbox", "read-only"]);
        assert_eq!(codex.args.last().map(String::as_str), Some("-"));
        assert_eq!(codex.answer_file.as_deref(), Some(scratch));
        assert!(!codex.args.contains(&"--model".to_string()));

        assert!(invocation(&settings("auto"), &found(&[]), scratch).unwrap_err().contains("neither"));
        assert!(invocation(&settings("codex"), &found(&["claude"]), scratch).unwrap_err().contains("Codex"));

        let mut own = settings("custom");
        assert!(invocation(&own, &found(&["ollama"]), scratch).unwrap_err().contains("no command"));
        own.custom = "ollama run \"my model\"".into();
        let custom = invocation(&own, &found(&["ollama"]), scratch).unwrap();
        assert_eq!(
            (custom.program, custom.args),
            (PathBuf::from("/bin/ollama"), vec!["run".to_string(), "my model".to_string()])
        );
        assert!(invocation(&own, &found(&[]), scratch).unwrap_err().contains("ollama"));
    }

    #[test]
    #[ignore]
    fn a_real_assistant_answers() {
        let provider = std::env::var("COMMIT_AI_TEST").expect("set COMMIT_AI_TEST to auto, claude, codex or custom");
        let settings =
            Settings { provider, custom: std::env::var("COMMIT_AI_COMMAND").unwrap_or_default(), ..Default::default() };
        let change = Change {
            staged: true,
            diff: "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1,3 @@\n pub fn add(a: i32, b: i32) -> i32 { a + b }\n+\n+pub fn sub(a: i32, b: i32) -> i32 { a - b }\n".into(),
            recent: vec!["Add the add function".into(), "Start the library".into()],
        };
        let scratch = std::env::temp_dir().join("code-canvas-commit-test.txt");
        let command = invocation(&settings, &find_program, &scratch).expect("a command");
        println!("running {}", command.program.display());
        let message = run(&command, &std::env::temp_dir(), &prompt("demo", &change)).expect("a message");
        println!("message: {message}");
        assert!(!message.is_empty() && message.lines().count() == 1);
    }
}

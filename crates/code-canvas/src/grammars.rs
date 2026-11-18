use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use canvas_core::{Lang, Painted, Painter, Piece, SourceFile, TokenKind};
use irosashi::{Highlighter, HighlighterBuilder, ScopeListId, Session, StateStack, TokenizeOptions};

use crate::ext::{Extensions, GrammarDecl, LanguageDecl};

pub const MAX_LINES: usize = 200_000;
const MAX_LINE_BYTES: usize = 4_000;
const SLICE: Duration = Duration::from_millis(15);
const SEND_EVERY: Duration = Duration::from_millis(150);
const EDIT_WAIT: Duration = Duration::from_millis(8);
const MEMO_LINES: usize = 400_000;

#[derive(Default)]
pub struct Matcher {
    by_name: HashMap<String, String>,
    by_ext: HashMap<String, String>,
    everywhere: bool,
}

impl Matcher {
    pub fn new<'a>(
        languages: impl Iterator<Item = &'a LanguageDecl>,
        grammars: &[GrammarDecl],
        everywhere: bool,
    ) -> Matcher {
        let with_grammar: HashSet<&str> = grammars.iter().filter_map(|g| g.language.as_deref()).collect();
        let mut out = Matcher { everywhere, ..Default::default() };
        for language in languages.filter(|l| with_grammar.contains(l.id.as_str())) {
            for ext in &language.extensions {
                let ext = if ext.starts_with('.') { ext.clone() } else { format!(".{ext}") };
                out.by_ext.insert(ext, language.id.clone());
            }
            for name in &language.filenames {
                out.by_name.insert(name.clone(), language.id.clone());
            }
        }
        out
    }

    pub fn language(&self, rel_path: &str, own: Lang) -> Option<&str> {
        if !self.everywhere && own != Lang::Plain {
            return None;
        }
        let name = rel_path.rsplit(['/', '\\']).next().unwrap_or(rel_path).to_lowercase();
        if let Some(language) = self.by_name.get(&name) {
            return Some(language);
        }
        name.match_indices('.').find_map(|(at, _)| self.by_ext.get(&name[at..])).map(String::as_str)
    }

    fn paints(&self, file: &SourceFile) -> bool {
        file.note.as_ref().is_none_or(|note| note.starts_with("truncated"))
            && file.line_count() <= MAX_LINES
            && self.language(&file.rel_path, file.lang).is_some()
    }

    fn is_empty(&self) -> bool {
        self.by_ext.is_empty() && self.by_name.is_empty()
    }
}

pub struct Done {
    pub page: u32,
    pub file: u32,
    pub serial: u64,
    pub painted: Arc<Painted>,
    pub complete: bool,
    generation: u64,
}

struct Job {
    page: u32,
    file: u32,
    serial: u64,
    rel_path: String,
    language: String,
    text: String,
    starts: Vec<u32>,
    urgent: bool,
    generation: u64,
}

enum ToWorker {
    Grammars(u64, Vec<GrammarDecl>),
    Paint(Job),
    Forget(u32),
}

pub struct Grammars {
    matcher: Arc<Matcher>,
    applied: Option<(u64, bool)>,
    generation: u64,
    worker: Option<(Sender<ToWorker>, Receiver<Done>)>,
    flying: HashMap<(u32, u32), u64>,
    backlog: Vec<Done>,
    hurry: Arc<AtomicBool>,
}

impl Default for Grammars {
    fn default() -> Self {
        Grammars {
            matcher: Arc::new(Matcher::default()),
            applied: None,
            generation: 0,
            worker: None,
            flying: HashMap::new(),
            backlog: Vec::new(),
            hurry: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Grammars {
    pub fn update(&mut self, ext: &Extensions, everywhere: bool) -> bool {
        let key = (ext.installed_revision, everywhere);
        if self.applied == Some(key) {
            return false;
        }
        self.applied = Some(key);
        let grammars: Vec<GrammarDecl> = ext.grammars().cloned().collect();
        let matcher = Matcher::new(ext.languages(), &grammars, everywhere);
        let had = !self.matcher.is_empty();
        self.matcher = Arc::new(matcher);
        self.generation += 1;
        self.flying.clear();
        self.backlog.clear();
        if !self.matcher.is_empty() || self.worker.is_some() {
            log::info!(
                "grammars of extensions: {} (for {} file names and extensions)",
                grammars.len(),
                self.matcher.by_ext.len() + self.matcher.by_name.len()
            );
            let generation = self.generation;
            let _ = self.sender().send(ToWorker::Grammars(generation, grammars));
        }
        had || !self.matcher.is_empty()
    }

    fn sender(&mut self) -> &Sender<ToWorker> {
        let hurry = self.hurry.clone();
        &self
            .worker
            .get_or_insert_with(|| {
                let (to, jobs) = mpsc::channel();
                let (done, from) = mpsc::channel();
                let spawned = std::thread::Builder::new()
                    .name("grammars".into())
                    .spawn(move || Worker { hurry, ..Worker::new(done) }.run(jobs));
                if let Err(e) = spawned {
                    log::error!("the grammar thread did not start: {e}");
                }
                (to, from)
            })
            .0
    }

    pub fn painter(&self) -> Option<Painter> {
        let matcher = self.matcher.clone();
        (!matcher.is_empty()).then(|| Arc::new(move |file: &SourceFile| matcher.paints(file)) as Painter)
    }

    pub fn language(&self, file: &SourceFile) -> Option<&str> {
        self.matcher.paints(file).then(|| self.matcher.language(&file.rel_path, file.lang)).flatten()
    }

    pub fn ask(&mut self, page: u32, index: usize, file: &SourceFile, urgent: bool) -> Option<u64> {
        let serial = file.serial();
        if self.flying.get(&(page, index as u32)) == Some(&serial) || !self.matcher.paints(file) {
            return None;
        }
        let language = self.matcher.language(&file.rel_path, file.lang)?.to_string();
        let job = Job {
            page,
            file: index as u32,
            serial,
            rel_path: file.rel_path.clone(),
            language,
            text: file.text().to_string(),
            starts: file.line_starts().to_vec(),
            urgent,
            generation: self.generation,
        };
        self.flying.insert((page, index as u32), serial);
        self.sender().send(ToWorker::Paint(job)).ok()?;
        if urgent {
            self.hurry.store(true, Ordering::Relaxed);
        }
        Some(serial)
    }

    pub fn take(&mut self) -> Vec<Done> {
        let mut out = std::mem::take(&mut self.backlog);
        if let Some((_, rx)) = &self.worker {
            out.extend(rx.try_iter());
        }
        out.retain(|done| done.generation == self.generation);
        for done in out.iter().filter(|done| done.complete) {
            if self.flying.get(&(done.page, done.file)) == Some(&done.serial) {
                self.flying.remove(&(done.page, done.file));
            }
        }
        out
    }

    pub fn wait(&mut self, page: u32, index: usize, serial: u64) -> Option<Arc<Painted>> {
        let until = Instant::now() + EDIT_WAIT;
        let rx = &self.worker.as_ref()?.1;
        loop {
            let left = until.checked_duration_since(Instant::now())?;
            let done = rx.recv_timeout(left).ok()?;
            let ours = (done.page, done.file, done.serial, done.complete, done.generation)
                == (page, index as u32, serial, true, self.generation);
            if ours {
                self.flying.remove(&(page, index as u32));
                return Some(done.painted);
            }
            self.backlog.push(done);
        }
    }

    pub fn forget_page(&mut self, page: u32) {
        self.flying.retain(|(p, _), _| *p != page);
        if let Some((tx, _)) = &self.worker {
            let _ = tx.send(ToWorker::Forget(page));
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Rule {
    word: Option<TokenKind>,
    other: Option<TokenKind>,
}

const RULES: &[(&str, TokenKind, TokenKind)] = &[
    ("comment", TokenKind::Comment, TokenKind::Comment),
    ("punctuation.definition.comment", TokenKind::Comment, TokenKind::Comment),
    ("string", TokenKind::String, TokenKind::String),
    ("punctuation.definition.string", TokenKind::String, TokenKind::String),
    ("constant.character.escape", TokenKind::String, TokenKind::String),
    ("markup.heading", TokenKind::Heading, TokenKind::Heading),
    ("entity.name.section", TokenKind::Heading, TokenKind::Heading),
    ("punctuation.definition.heading", TokenKind::Heading, TokenKind::Heading),
    ("markup.inline.raw", TokenKind::String, TokenKind::String),
    ("markup.raw", TokenKind::String, TokenKind::String),
    ("markup.fenced_code", TokenKind::String, TokenKind::String),
    ("markup", TokenKind::Text, TokenKind::Text),
    ("constant.numeric", TokenKind::Number, TokenKind::Number),
    ("constant.language", TokenKind::Keyword, TokenKind::Keyword),
    ("constant", TokenKind::Number, TokenKind::Number),
    ("support.constant", TokenKind::Number, TokenKind::Number),
    ("keyword.operator", TokenKind::Keyword, TokenKind::Punct),
    ("keyword.control.directive", TokenKind::Macro, TokenKind::Macro),
    ("meta.preprocessor", TokenKind::Macro, TokenKind::Macro),
    ("entity.name.function.macro", TokenKind::Macro, TokenKind::Macro),
    ("entity.name.function.preprocessor", TokenKind::Macro, TokenKind::Macro),
    ("support.function.macro", TokenKind::Macro, TokenKind::Macro),
    ("entity.name.macro", TokenKind::Macro, TokenKind::Macro),
    ("keyword", TokenKind::Keyword, TokenKind::Keyword),
    ("storage", TokenKind::Keyword, TokenKind::Keyword),
    ("entity.name.tag", TokenKind::Keyword, TokenKind::Keyword),
    ("variable.language", TokenKind::Keyword, TokenKind::Keyword),
    ("entity.other.attribute-name", TokenKind::Function, TokenKind::Function),
    ("support.type.property-name", TokenKind::Function, TokenKind::Function),
    ("entity.name.function", TokenKind::Function, TokenKind::Function),
    ("support.function", TokenKind::Function, TokenKind::Function),
    ("entity.name.type", TokenKind::Type, TokenKind::Type),
    ("entity.name.class", TokenKind::Type, TokenKind::Type),
    ("entity.name.namespace", TokenKind::Type, TokenKind::Type),
    ("entity.other.inherited-class", TokenKind::Type, TokenKind::Type),
    ("support.type", TokenKind::Type, TokenKind::Type),
    ("support.class", TokenKind::Type, TokenKind::Type),
    ("punctuation", TokenKind::Punct, TokenKind::Punct),
    ("meta.brace", TokenKind::Punct, TokenKind::Punct),
    ("variable", TokenKind::Ident, TokenKind::Punct),
    ("entity.name", TokenKind::Ident, TokenKind::Punct),
    ("support", TokenKind::Ident, TokenKind::Punct),
];

fn rule_of(scopes: &[&str]) -> Rule {
    let fits = |scope: &str, prefix: &str| {
        scope.strip_prefix(prefix).is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
    };
    for scope in scopes.iter().rev() {
        let best = RULES.iter().filter(|(prefix, ..)| fits(scope, prefix)).max_by_key(|(prefix, ..)| prefix.len());
        if let Some(&(_, word, other)) = best {
            return Rule { word: Some(word), other: Some(other) };
        }
    }
    match scopes.first().is_some_and(|root| fits(root, "text")) {
        true => Rule { word: Some(TokenKind::Text), other: Some(TokenKind::Text) },
        false => Rule { word: Some(TokenKind::Ident), other: Some(TokenKind::Punct) },
    }
}

fn push(out: &mut Vec<Piece>, start: usize, end: usize, kind: Option<TokenKind>) {
    let Some(kind) = kind.filter(|_| end > start) else { return };
    if let Some(last) = out.last_mut().filter(|last| last.kind == kind && (last.start + last.len) as usize == start) {
        last.len += (end - start) as u32;
        return;
    }
    out.push(Piece { start: start as u32, len: (end - start) as u32, kind });
}

fn cut(line: &str, start: usize, end: usize, rule: Rule, out: &mut Vec<Piece>) {
    let text = &line[start..end];
    let whole = matches!(rule.word, Some(TokenKind::String | TokenKind::Comment));
    if whole {
        let trimmed = text.trim_start();
        let from = start + text.len() - trimmed.len();
        let to = from + trimmed.trim_end().len();
        let joins = |last: &&mut Piece| {
            let end = (last.start + last.len) as usize;
            Some(last.kind) == rule.word && end <= from && line[end..from].trim().is_empty()
        };
        match out.last_mut().filter(joins) {
            Some(last) if to > from => last.len = (to - last.start as usize) as u32,
            _ => push(out, from, to, rule.word),
        }
        return;
    }
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let mut run: Option<(usize, bool)> = None;
    for (at, c) in text.char_indices().chain(std::iter::once((text.len(), ' '))) {
        let class = (!c.is_whitespace()).then(|| rule.word == rule.other || word(c));
        if run.is_some_and(|(_, was)| Some(was) != class) {
            let (from, was) = run.take().unwrap_or_default();
            push(out, start + from, start + at, if was { rule.word } else { rule.other });
        }
        if let (None, Some(class)) = (run, class) {
            run = Some((at, class));
        }
    }
}

fn hash_line(text: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

struct Line {
    hash: u64,
    state: StateStack,
    pieces: Arc<[Piece]>,
}

struct Memo {
    language: String,
    lines: Vec<Line>,
    complete: bool,
    used: u64,
}

struct Task {
    job: Job,
    hashes: Vec<u64>,
    lines: Vec<Line>,
    old: Vec<Line>,
    old_total: usize,
    prefix: usize,
    suffix: usize,
    state: StateStack,
    sent: Option<Instant>,
    sent_lines: usize,
    began: Instant,
    tokenized: usize,
}

impl Task {
    fn total(&self) -> usize {
        self.hashes.len()
    }

    fn line(&self, i: usize) -> &str {
        let start = self.job.starts[i] as usize;
        let end = self.job.starts.get(i + 1).map_or(self.job.text.len(), |&next| next as usize - 1);
        self.job.text.get(start..end).unwrap_or("")
    }

    fn painted(&self) -> Arc<Painted> {
        Arc::new(Painted {
            serial: self.job.serial,
            total: self.total(),
            lines: self.lines.iter().map(|line| line.pieces.clone()).collect(),
        })
    }
}

struct Grammar {
    session: Session,
    rules: HashMap<ScopeListId, Rule>,
}

struct Worker {
    out: Sender<Done>,
    highlighter: Option<Highlighter>,
    generation: u64,
    grammars: HashMap<String, Option<Grammar>>,
    memos: HashMap<(u32, String), Memo>,
    tasks: Vec<Task>,
    clock: u64,
    empty: Arc<[Piece]>,
    hurry: Arc<AtomicBool>,
}

fn plain_json(bytes: &[u8]) -> Vec<u8> {
    bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes).to_vec()
}

impl Worker {
    fn new(out: Sender<Done>) -> Worker {
        Worker {
            out,
            highlighter: None,
            generation: 0,
            grammars: HashMap::new(),
            memos: HashMap::new(),
            tasks: Vec::new(),
            clock: 0,
            empty: Arc::from(Vec::new()),
            hurry: Arc::new(AtomicBool::new(false)),
        }
    }

    fn run(mut self, jobs: Receiver<ToWorker>) {
        loop {
            if self.tasks.is_empty() {
                match jobs.recv() {
                    Ok(message) => self.take(message),
                    Err(_) => return,
                }
            }
            self.hurry.store(false, Ordering::Relaxed);
            while let Ok(message) = jobs.try_recv() {
                self.take(message);
            }
            self.step();
        }
    }

    fn take(&mut self, message: ToWorker) {
        match message {
            ToWorker::Grammars(generation, decls) => self.load(generation, decls),
            ToWorker::Paint(job) if job.generation == self.generation => self.start(job),
            ToWorker::Paint(_) => {}
            ToWorker::Forget(page) => {
                self.tasks.retain(|task| task.job.page != page);
                self.memos.retain(|(p, _), _| *p != page);
            }
        }
    }

    fn load(&mut self, generation: u64, decls: Vec<GrammarDecl>) {
        (self.generation, self.tasks, self.memos, self.grammars) =
            (generation, Vec::new(), HashMap::new(), HashMap::new());
        let highlighter = match HighlighterBuilder::without_bundled_assets().build() {
            Ok(highlighter) => highlighter,
            Err(e) => {
                log::error!("grammars: {e}");
                self.highlighter = None;
                return;
            }
        };
        for decl in &decls {
            let name = decl.language.clone().unwrap_or_else(|| decl.scope_name.clone());
            let Some(json) = grammar_json(decl) else { continue };
            if let Err(e) = highlighter.load_language(&name, json) {
                log::warn!("grammar {} of {name} is left out: {e}", decl.file.display());
            }
        }
        self.highlighter = Some(highlighter);
    }

    fn grammar(&mut self, language: &str) -> Option<&mut Grammar> {
        if !self.grammars.contains_key(language) {
            let made = self.highlighter.as_ref().map(|h| h.session(language));
            let grammar = match made {
                Some(Ok(session)) => Some(Grammar { session, rules: HashMap::new() }),
                Some(Err(e)) => {
                    log::warn!("the grammar of {language} cannot be used: {e}");
                    None
                }
                None => None,
            };
            self.grammars.insert(language.to_string(), grammar);
        }
        self.grammars.get_mut(language)?.as_mut()
    }

    fn start(&mut self, job: Job) {
        let key = (job.page, job.rel_path.clone());
        if let Some(at) = self.tasks.iter().position(|t| (t.job.page, &t.job.rel_path) == (key.0, &key.1)) {
            let task = self.tasks.remove(at);
            if !task.lines.is_empty() {
                self.clock += 1;
                let memo = Memo { language: task.job.language, lines: task.lines, complete: false, used: self.clock };
                self.memos.insert(key.clone(), memo);
            }
        }
        let Some(initial) = self.grammar(&job.language).map(|g| g.session.initial_state()) else { return };
        let total = job.starts.len();
        let line = |i: usize| {
            let start = job.starts[i] as usize;
            let end = job.starts.get(i + 1).map_or(job.text.len(), |&next| next as usize - 1);
            job.text.get(start..end).unwrap_or("")
        };
        let hashes: Vec<u64> = (0..total).map(|i| hash_line(line(i))).collect();
        let memo = self.memos.remove(&key).filter(|memo| memo.language == job.language);
        let (mut lines, complete) = memo.map_or((Vec::new(), false), |memo| (memo.lines, memo.complete));
        let prefix = lines.iter().zip(&hashes).take_while(|(line, hash)| line.hash == **hash).count();
        let old_total = lines.len();
        let suffix = match complete {
            true => lines
                .iter()
                .rev()
                .zip(hashes.iter().rev())
                .take(old_total.min(total) - prefix)
                .take_while(|(line, hash)| line.hash == **hash)
                .count(),
            false => 0,
        };
        let old = lines.split_off(prefix);
        let state = lines.last().map_or(initial, |line| line.state.clone());
        let task = Task {
            job,
            hashes,
            lines,
            old,
            old_total,
            prefix,
            suffix,
            state,
            sent: None,
            sent_lines: 0,
            began: Instant::now(),
            tokenized: 0,
        };
        self.tasks.push(task);
    }

    fn step(&mut self) {
        let pick = self
            .tasks
            .iter()
            .enumerate()
            .min_by_key(|(_, t)| (!t.job.urgent, t.total() - t.lines.len()))
            .map(|(at, _)| at);
        let Some(at) = pick else { return };
        let mut task = self.tasks.swap_remove(at);
        let deadline = Instant::now() + SLICE;
        let (empty, hurry) = (self.empty.clone(), self.hurry.clone());
        let Some(grammar) = self.grammar(&task.job.language) else { return };
        let initial = grammar.session.initial_state();
        while task.lines.len() < task.total() {
            let i = task.lines.len();
            if task.suffix > 0 && i >= task.total() - task.suffix {
                let j = i + task.old_total - task.total();
                let before = match j {
                    0 => &initial,
                    j if j - 1 < task.prefix => &task.lines[j - 1].state,
                    j => &task.old[j - 1 - task.prefix].state,
                };
                if *before == task.state {
                    let rest = task.old.drain(j - task.prefix..);
                    task.lines.extend(rest);
                    break;
                }
            }
            let line = task.line(i);
            let (pieces, state) = if line.len() > MAX_LINE_BYTES {
                (empty.clone(), task.state.clone())
            } else {
                let options = TokenizeOptions { max_line_length: None };
                let result = grammar.session.tokenize_line(line, &task.state, i == 0, options);
                let mut pieces = Vec::with_capacity(result.tokens.len());
                for token in &result.tokens {
                    let rule = *grammar
                        .rules
                        .entry(token.scopes)
                        .or_insert_with(|| rule_of(&grammar.session.scope_names(token.scopes)));
                    cut(line, token.start, token.end, rule, &mut pieces);
                }
                let pieces = if pieces.is_empty() { empty.clone() } else { Arc::from(pieces) };
                (pieces, result.state)
            };
            task.tokenized += 1;
            let state = if state == task.state { task.state.clone() } else { state };
            task.state = state.clone();
            task.lines.push(Line { hash: task.hashes[i], state, pieces });
            if i % 32 == 31 && (Instant::now() > deadline || hurry.load(Ordering::Relaxed)) {
                break;
            }
        }
        let complete = task.lines.len() == task.total();
        let due = task.sent.is_none_or(|sent| sent.elapsed() > SEND_EVERY) && task.lines.len() > task.sent_lines;
        if complete || due {
            let done = Done {
                page: task.job.page,
                file: task.job.file,
                serial: task.job.serial,
                painted: task.painted(),
                complete,
                generation: task.job.generation,
            };
            (task.sent, task.sent_lines) = (Some(Instant::now()), task.lines.len());
            if self.out.send(done).is_ok() {
                crate::ext::wake_ui();
            }
        }
        if !complete {
            self.tasks.push(task);
            return;
        }
        let (ms, lines) = (task.began.elapsed().as_secs_f32() * 1000.0, task.total());
        let said = format!(
            "the grammar of {} coloured {}: {} of its {lines} lines tokenized in {ms:.0} ms",
            task.job.language, task.job.rel_path, task.tokenized
        );
        match task.tokenized == lines {
            true => log::info!("{said}"),
            false => log::debug!("{said}"),
        }
        self.clock += 1;
        let Task { job, lines, .. } = task;
        let memo = Memo { language: job.language, lines, complete: true, used: self.clock };
        self.memos.insert((job.page, job.rel_path), memo);
        let mut kept: usize = self.memos.values().map(|memo| memo.lines.len()).sum();
        while kept > MEMO_LINES && self.memos.len() > 1 {
            let Some(oldest) = self.memos.iter().min_by_key(|(_, memo)| memo.used).map(|(key, _)| key.clone()) else {
                break;
            };
            kept -= self.memos.remove(&oldest).map_or(0, |memo| memo.lines.len());
        }
    }
}

fn grammar_json(decl: &GrammarDecl) -> Option<Vec<u8>> {
    let bytes = match std::fs::read(&decl.file) {
        Ok(bytes) => bytes,
        Err(e) => {
            log::warn!("grammar {} cannot be read: {e}", decl.file.display());
            return None;
        }
    };
    let name = decl.file.to_string_lossy().to_lowercase();
    let json = if name.ends_with(".json") {
        plain_json(&bytes)
    } else if name.ends_with(".tmlanguage") || name.ends_with(".plist") || name.ends_with(".xml") {
        match crate::plist::to_json(&String::from_utf8_lossy(&bytes)) {
            Some(value) => value.to_string().into_bytes(),
            None => {
                log::warn!("grammar {} is not a property list this app reads", decl.file.display());
                return None;
            }
        }
    } else {
        log::warn!("grammar {} is in a format this app does not read", decl.file.display());
        return None;
    };
    if decl.inject_to.is_empty() {
        return Some(json);
    }
    let mut value: serde_json::Value = serde_json::from_slice(&json).ok()?;
    value.as_object_mut()?.insert("injectTo".into(), decl.inject_to.clone().into());
    Some(value.to_string().into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRAMMAR: &str = r##"{
      "scopeName": "source.zed",
      "patterns": [
        { "include": "#comment" },
        { "match": "\\b(when|loop|give)\\b", "name": "keyword.control.zed" },
        { "match": "\\b[0-9]+\\b", "name": "constant.numeric.zed" },
        { "match": "\\b([a-z]+)\\s*(?=\\()", "captures": { "1": { "name": "entity.name.function.zed" } } },
        { "begin": "\"", "end": "\"", "name": "string.quoted.double.zed" },
        { "match": "[-+*/=<>]+", "name": "keyword.operator.zed" }
      ],
      "repository": {
        "comment": { "begin": "\\(\\*", "end": "\\*\\)", "name": "comment.block.zed" }
      }
    }"##;

    fn highlighter() -> Highlighter {
        let h = HighlighterBuilder::without_bundled_assets().build().unwrap();
        h.load_language("zed", GRAMMAR.as_bytes().to_vec()).unwrap();
        h
    }

    fn kinds(text: &str) -> Vec<Vec<(String, TokenKind)>> {
        let mut session = highlighter().session("zed").unwrap();
        let mut state = session.initial_state();
        let mut out = Vec::new();
        for (i, line) in text.lines().enumerate() {
            let result = session.tokenize_line(line, &state, i == 0, TokenizeOptions::default());
            let mut pieces = Vec::new();
            for token in &result.tokens {
                cut(line, token.start, token.end, rule_of(&session.scope_names(token.scopes)), &mut pieces);
            }
            out.push(
                pieces
                    .iter()
                    .map(|p| (line[p.start as usize..(p.start + p.len) as usize].to_string(), p.kind))
                    .collect(),
            );
            state = result.state;
        }
        out
    }

    #[test]
    fn scopes_become_the_kinds_the_theme_colours() {
        let lines = kinds("when count(3) give \"a b\" (* note\nstill *) x = 2");
        assert_eq!(
            lines[0],
            vec![
                ("when".into(), TokenKind::Keyword),
                ("count".into(), TokenKind::Function),
                ("(".into(), TokenKind::Punct),
                ("3".into(), TokenKind::Number),
                (")".into(), TokenKind::Punct),
                ("give".into(), TokenKind::Keyword),
                ("\"a b\"".into(), TokenKind::String),
                ("(* note".into(), TokenKind::Comment),
            ]
        );
        assert_eq!(
            lines[1],
            vec![
                ("still *)".into(), TokenKind::Comment),
                ("x".into(), TokenKind::Ident),
                ("=".into(), TokenKind::Punct),
                ("2".into(), TokenKind::Number),
            ]
        );
    }

    #[test]
    fn rules_pick_the_innermost_scope_that_says_something() {
        let rule = |scopes: &[&str]| rule_of(scopes).word;
        assert_eq!(
            rule(&["source.x", "string.quoted.x", "punctuation.definition.string.begin.x"]),
            Some(TokenKind::String)
        );
        assert_eq!(rule(&["source.x", "meta.block.x", "keyword.operator.new.x"]), Some(TokenKind::Keyword));
        assert_eq!(rule_of(&["source.x", "keyword.operator.x"]).other, Some(TokenKind::Punct));
        assert_eq!(
            rule(&["source.c", "meta.preprocessor.include.c", "keyword.control.directive.include.c"]),
            Some(TokenKind::Macro)
        );
        assert_eq!(
            rule(&["text.html.markdown", "markup.heading.1.md", "entity.name.section.md"]),
            Some(TokenKind::Heading)
        );
        assert_eq!(rule(&["text.plain"]), Some(TokenKind::Text));
        assert_eq!(rule(&["source.x", "meta.thing.x"]), Some(TokenKind::Ident));
        assert_eq!(rule(&["source.x", "keywordish.x"]), Some(TokenKind::Ident));
    }

    #[test]
    fn languages_are_found_by_extension_and_name_only_where_the_app_has_none() {
        let languages = [LanguageDecl {
            id: "zed".into(),
            extensions: vec![".zed".into(), ".zed.txt".into()],
            filenames: vec!["zedfile".into()],
        }];
        let grammars = [GrammarDecl {
            language: Some("zed".into()),
            scope_name: "source.zed".into(),
            file: "zed.json".into(),
            inject_to: Vec::new(),
        }];
        let m = Matcher::new(languages.iter(), &grammars, false);
        assert_eq!(m.language("a/b/Main.ZED", Lang::Plain), Some("zed"));
        assert_eq!(m.language("notes.zed.txt", Lang::Plain), Some("zed"));
        assert_eq!(m.language("x/Zedfile", Lang::Plain), Some("zed"));
        assert_eq!(m.language("main.rs", Lang::Plain), None);
        assert_eq!(m.language("main.zed", Lang::Rust), None);
        assert_eq!(Matcher::new(languages.iter(), &grammars, true).language("main.zed", Lang::Rust), Some("zed"));
        assert!(Matcher::new(languages.iter(), &[], false).is_empty());
    }

    fn worker_paints(worker: &mut Worker, rx: &Receiver<Done>, text: &str, serial: u64) -> Arc<Painted> {
        let (text, starts, _) = canvas_core::scan::normalise(text);
        let job = Job {
            page: 1,
            file: 0,
            serial,
            rel_path: "a.zed".into(),
            language: "zed".into(),
            text,
            starts,
            urgent: true,
            generation: worker.generation,
        };
        worker.start(job);
        while !worker.tasks.is_empty() {
            worker.step();
        }
        rx.try_iter().filter(|d| d.complete && d.serial == serial).last().unwrap().painted
    }

    #[test]
    fn an_edit_tokenizes_again_only_until_the_state_is_the_same() {
        let dir = std::env::temp_dir().join(format!("code-canvas-grammar-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("zed.json"), GRAMMAR).unwrap();
        let decl = GrammarDecl {
            language: Some("zed".into()),
            scope_name: "source.zed".into(),
            file: dir.join("zed.json"),
            inject_to: Vec::new(),
        };
        let (tx, rx) = mpsc::channel();
        let mut worker = Worker::new(tx);
        worker.load(1, vec![decl]);
        let _ = std::fs::remove_dir_all(&dir);
        let mut text: String = (0..300).map(|i| format!("give f({i})\n")).collect();
        let first = worker_paints(&mut worker, &rx, &text, 1);
        assert_eq!(first.lines.len(), 300);
        let before = worker.grammars["zed"].as_ref().unwrap().session.stats().lines;
        text.replace_range(0..0, "when\n");
        let second = worker_paints(&mut worker, &rx, &text, 2);
        let after = worker.grammars["zed"].as_ref().unwrap().session.stats().lines;
        assert_eq!(second.lines.len(), 301);
        assert!(after - before <= 2, "{} lines tokenized again", after - before);
        assert_eq!(second.lines[1], first.lines[0]);
        text.replace_range(0..0, "(* ");
        let third = worker_paints(&mut worker, &rx, &text, 3);
        assert!(!third.lines[300].is_empty() && third.lines[300].iter().all(|p| p.kind == TokenKind::Comment));
    }
}

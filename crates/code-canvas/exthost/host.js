'use strict';

const fs = require('fs');
const path = require('path');
const os = require('os');
const Module = require('module');
const { pathToFileURL } = require('url');
const { AsyncLocalStorage } = require('async_hooks');

const WIN = process.platform === 'win32';
const VSCODE_VERSION = '1.105.0';

const rawWrite = process.stdout.write.bind(process.stdout);
function send(msg) {
  rawWrite(JSON.stringify(msg) + '\n');
}
function log(level, source, text) {
  send({ type: 'log', level, source, text: String(text).replace(/\s+$/, '') });
}
process.stdout.write = (chunk, _enc, cb) => {
  log('info', currentId() || 'host', chunk);
  if (typeof cb === 'function') cb();
  return true;
};
const fmt = (args) => args.map((a) => (typeof a === 'string' ? a : inspect(a))).join(' ');
function inspect(v) {
  if (v instanceof Error) return v.stack || v.message;
  try {
    return JSON.stringify(v);
  } catch {
    return String(v);
  }
}
for (const [name, level] of [['log', 'info'], ['info', 'info'], ['debug', 'debug'], ['warn', 'warn'], ['error', 'error'], ['trace', 'debug']]) {
  console[name] = (...args) => log(level, currentId() || 'host', fmt(args));
}
process.on('uncaughtException', (e) => log('error', 'host', 'uncaught: ' + inspect(e)));
process.on('unhandledRejection', (e) => log('error', 'host', 'unhandled rejection: ' + inspect(e)));

const scope = new AsyncLocalStorage();
const currentId = () => scope.getStore();

const reported = new Set();
function unsupported(api) {
  const key = (currentId() || '?') + ' ' + api;
  if (reported.has(key)) return;
  reported.add(key);
  send({ type: 'unsupported', extension: currentId() || '', api });
}

class Disposable {
  constructor(fn) {
    this._fn = fn;
  }
  static from(...items) {
    return new Disposable(() => {
      for (const d of items) if (d && typeof d.dispose === 'function') d.dispose();
    });
  }
  dispose() {
    const fn = this._fn;
    this._fn = undefined;
    if (typeof fn === 'function') fn();
  }
}

class EventEmitter {
  constructor() {
    this._listeners = new Set();
    this.event = (listener, thisArg, disposables) => {
      const entry = { listener, thisArg, owner: currentId() };
      this._listeners.add(entry);
      const d = new Disposable(() => this._listeners.delete(entry));
      if (Array.isArray(disposables)) disposables.push(d);
      return d;
    };
  }
  fire(value) {
    for (const { listener, thisArg, owner } of [...this._listeners]) {
      try {
        if (owner && owner !== currentId()) scope.run(owner, () => listener.call(thisArg, value));
        else listener.call(thisArg, value);
      } catch (e) {
        log('error', 'host', 'listener failed: ' + inspect(e));
      }
    }
  }
  dispose() {
    this._listeners.clear();
  }
}
const neverEvent = () => new Disposable(() => {});

class CancellationTokenSource {
  constructor() {
    const emitter = new EventEmitter();
    this._emitter = emitter;
    this.token = { isCancellationRequested: false, onCancellationRequested: emitter.event };
  }
  cancel() {
    if (this.token.isCancellationRequested) return;
    this.token.isCancellationRequested = true;
    this._emitter.fire();
  }
  dispose() {
    this._emitter.dispose();
  }
}
const noToken = Object.freeze({ isCancellationRequested: false, onCancellationRequested: neverEvent });

class CancellationError extends Error {
  constructor() {
    super('Canceled');
    this.name = 'Canceled';
  }
}

const encodePath = (p) =>
  p.replace(/[^A-Za-z0-9\-._~/]/g, (c) => (c === ':' ? '%3A' : encodeURIComponent(c).replace(/[!'()*]/g, (x) => '%' + x.charCodeAt(0).toString(16).toUpperCase())));

class Uri {
  constructor(scheme, authority, p, query, fragment) {
    this.scheme = scheme || '';
    this.authority = authority || '';
    this.path = p || '';
    this.query = query || '';
    this.fragment = fragment || '';
  }
  static isUri(x) {
    return x instanceof Uri;
  }
  static file(p) {
    let authority = '';
    p = String(p).replace(/\\/g, '/');
    if (p.startsWith('//')) {
      const i = p.indexOf('/', 2);
      authority = i < 0 ? p.slice(2) : p.slice(2, i);
      p = i < 0 ? '/' : p.slice(i);
    }
    if (!p.startsWith('/')) p = '/' + p;
    return new Uri('file', authority, p, '', '');
  }
  static parse(value) {
    const m = /^(([^:/?#]+?):)?(\/\/([^/?#]*))?([^?#]*)(\?([^#]*))?(#(.*))?/.exec(String(value));
    if (!m) return new Uri('', '', '', '', '');
    const dec = (s) => {
      try {
        return decodeURIComponent(s || '');
      } catch {
        return s || '';
      }
    };
    return new Uri(m[2] || '', dec(m[4]), dec(m[5]), dec(m[7]), dec(m[9]));
  }
  static from(c) {
    return new Uri(c.scheme, c.authority, c.path, c.query, c.fragment);
  }
  static joinPath(base, ...segments) {
    return base.with({ path: path.posix.join(base.path || '/', ...segments) });
  }
  with(change) {
    const pick = (k) => (change[k] === undefined ? this[k] : change[k] === null ? '' : change[k]);
    return new Uri(pick('scheme'), pick('authority'), pick('path'), pick('query'), pick('fragment'));
  }
  get fsPath() {
    let p = this.path;
    if (this.authority && p.length > 1 && this.scheme === 'file') p = `//${this.authority}${p}`;
    else if (/^\/[A-Za-z]:/.test(p)) p = p[1].toLowerCase() + p.slice(2);
    return WIN ? p.replace(/\//g, '\\') : p;
  }
  toString(skipEncoding) {
    let out = this.scheme ? this.scheme + ':' : '';
    if (this.authority || this.scheme === 'file') out += '//';
    out += skipEncoding ? this.authority : encodeURIComponent(this.authority).replace(/%3A/gi, ':');
    let p = this.path;
    if (/^\/[A-Za-z]:/.test(p)) p = '/' + p[1].toLowerCase() + p.slice(2);
    out += skipEncoding ? p : encodePath(p);
    if (this.query) out += '?' + (skipEncoding ? this.query : encodeURIComponent(this.query).replace(/%3D/g, '=').replace(/%26/g, '&'));
    if (this.fragment) out += '#' + (skipEncoding ? this.fragment : encodeURIComponent(this.fragment));
    return out;
  }
  toJSON() {
    return { $mid: 1, scheme: this.scheme, authority: this.authority, path: this.path, query: this.query, fragment: this.fragment, fsPath: this.fsPath, external: this.toString() };
  }
}

function fileKey(p) {
  p = path.normalize(String(p));
  return WIN ? p.toLowerCase() : p;
}
const uriKey = (uri) => (uri.scheme === 'file' ? fileKey(uri.fsPath) : uri.toString());

class Position {
  constructor(line, character) {
    this.line = Math.max(0, line | 0);
    this.character = Math.max(0, character | 0);
  }
  static isPosition(x) {
    return x instanceof Position || (x && typeof x.line === 'number' && typeof x.character === 'number');
  }
  compareTo(o) {
    return this.line - o.line || this.character - o.character;
  }
  isBefore(o) {
    return this.compareTo(o) < 0;
  }
  isBeforeOrEqual(o) {
    return this.compareTo(o) <= 0;
  }
  isAfter(o) {
    return this.compareTo(o) > 0;
  }
  isAfterOrEqual(o) {
    return this.compareTo(o) >= 0;
  }
  isEqual(o) {
    return this.compareTo(o) === 0;
  }
  translate(a, b) {
    const d = a && typeof a === 'object' ? a : { lineDelta: a, characterDelta: b };
    return new Position(this.line + (d.lineDelta || 0), this.character + (d.characterDelta || 0));
  }
  with(a, b) {
    const d = a && typeof a === 'object' ? a : { line: a, character: b };
    return new Position(d.line === undefined ? this.line : d.line, d.character === undefined ? this.character : d.character);
  }
}

class Range {
  constructor(a, b, c, d) {
    let start = typeof a === 'number' ? new Position(a, b) : new Position(a.line, a.character);
    let end = typeof a === 'number' ? new Position(c, d) : new Position(b.line, b.character);
    if (end.isBefore(start)) [start, end] = [end, start];
    this.start = start;
    this.end = end;
  }
  static isRange(x) {
    return x instanceof Range || (x && Position.isPosition(x.start) && Position.isPosition(x.end));
  }
  get isEmpty() {
    return this.start.isEqual(this.end);
  }
  get isSingleLine() {
    return this.start.line === this.end.line;
  }
  contains(x) {
    if (x instanceof Range || (x && x.start)) return this.contains(x.start) && this.contains(x.end);
    const p = new Position(x.line, x.character);
    return p.isAfterOrEqual(this.start) && p.isBeforeOrEqual(this.end);
  }
  isEqual(o) {
    return this.start.isEqual(o.start) && this.end.isEqual(o.end);
  }
  intersection(o) {
    const start = this.start.isAfter(o.start) ? this.start : o.start;
    const end = this.end.isBefore(o.end) ? this.end : o.end;
    return start.isAfter(end) ? undefined : new Range(start, end);
  }
  union(o) {
    return new Range(this.start.isBefore(o.start) ? this.start : o.start, this.end.isAfter(o.end) ? this.end : o.end);
  }
  with(a, b) {
    const d = a && !Position.isPosition(a) ? a : { start: a, end: b };
    return new Range(d.start || this.start, d.end || this.end);
  }
}

class Selection extends Range {
  constructor(a, b, c, d) {
    super(a, b, c, d);
    this.anchor = typeof a === 'number' ? new Position(a, b) : new Position(a.line, a.character);
    this.active = typeof a === 'number' ? new Position(c, d) : new Position(b.line, b.character);
  }
  get isReversed() {
    return this.anchor.isAfter(this.active);
  }
}

class Location {
  constructor(uri, rangeOrPosition) {
    this.uri = uri;
    this.range = Position.isPosition(rangeOrPosition) && !rangeOrPosition.start ? new Range(rangeOrPosition, rangeOrPosition) : rangeOrPosition;
  }
}

class TextEdit {
  constructor(range, newText) {
    this.range = range;
    this.newText = newText;
  }
  static replace(range, newText) {
    return new TextEdit(range, newText);
  }
  static insert(position, newText) {
    return new TextEdit(new Range(position, position), newText);
  }
  static delete(range) {
    return new TextEdit(range, '');
  }
  static setEndOfLine(eol) {
    const e = new TextEdit(new Range(0, 0, 0, 0), '');
    e.newEol = eol;
    return e;
  }
}

class WorkspaceEdit {
  constructor() {
    this._edits = new Map();
    this._files = [];
  }
  _slot(uri) {
    const key = uriKey(uri);
    if (!this._edits.has(key)) this._edits.set(key, { uri, edits: [] });
    return this._edits.get(key);
  }
  get size() {
    return this._edits.size + this._files.length;
  }
  replace(uri, range, newText) {
    this._slot(uri).edits.push(new TextEdit(range, newText));
  }
  insert(uri, position, newText) {
    this.replace(uri, new Range(position, position), newText);
  }
  delete(uri, range) {
    this.replace(uri, range, '');
  }
  has(uri) {
    return this._edits.has(uriKey(uri));
  }
  set(uri, edits) {
    if (!edits) this._edits.delete(uriKey(uri));
    else {
      this._slot(uri).edits = edits
        .map((e) => (Array.isArray(e) ? e[0] : e))
        .filter((e) => e && e.range)
        .map((e) => (e.snippet ? new TextEdit(e.range, e.snippet.value || '') : e));
    }
  }
  get(uri) {
    const slot = this._edits.get(uriKey(uri));
    return slot ? slot.edits : [];
  }
  entries() {
    return [...this._edits.values()].map((s) => [s.uri, s.edits]);
  }
  createFile(uri, options) {
    this._files.push({ kind: 'create', uri, options });
  }
  deleteFile(uri, options) {
    this._files.push({ kind: 'delete', uri, options });
  }
  renameFile(from, to, options) {
    this._files.push({ kind: 'rename', from, to, options });
  }
}

function applyEdits(doc, edits) {
  const text = doc.getText();
  const spans = edits
    .filter((e) => e && e.range)
    .map((e, i) => ({ from: doc.offsetAt(e.range.start), to: doc.offsetAt(e.range.end), text: e.newText || '', i }))
    .sort((a, b) => a.from - b.from || a.i - b.i);
  let out = '';
  let at = 0;
  for (const s of spans) {
    if (s.from < at) continue;
    out += text.slice(at, s.from) + s.text;
    at = s.to;
  }
  return out + text.slice(at);
}

const EndOfLine = { LF: 1, CRLF: 2 };

class TextDocument {
  constructor(uri, languageId, text) {
    this.uri = uri;
    this.fileName = uri.fsPath;
    this.isUntitled = false;
    this.languageId = languageId;
    this.version = 1;
    this.isDirty = false;
    this.isClosed = false;
    this.encoding = 'utf8';
    this._setText(text);
  }
  _setText(text) {
    this._text = text;
    this._starts = null;
    this.eol = text.includes('\r\n') ? EndOfLine.CRLF : EndOfLine.LF;
  }
  get _lineStarts() {
    if (!this._starts) {
      const starts = [0];
      const t = this._text;
      for (let i = 0; i < t.length; i++) if (t.charCodeAt(i) === 10) starts.push(i + 1);
      this._starts = starts;
    }
    return this._starts;
  }
  get lineCount() {
    return this._lineStarts.length;
  }
  _lineEnd(line) {
    const starts = this._lineStarts;
    let end = line + 1 < starts.length ? starts[line + 1] - 1 : this._text.length;
    if (end > starts[line] && this._text.charCodeAt(end - 1) === 13) end--;
    return end;
  }
  getText(range) {
    if (!range) return this._text;
    return this._text.slice(this.offsetAt(range.start), this.offsetAt(range.end));
  }
  lineAt(lineOrPosition) {
    let line = typeof lineOrPosition === 'number' ? lineOrPosition : lineOrPosition.line;
    line = Math.min(Math.max(0, line), this.lineCount - 1);
    const start = this._lineStarts[line];
    const text = this._text.slice(start, this._lineEnd(line));
    const first = text.search(/\S/);
    return {
      lineNumber: line,
      text,
      range: new Range(line, 0, line, text.length),
      rangeIncludingLineBreak: line + 1 < this.lineCount ? new Range(line, 0, line + 1, 0) : new Range(line, 0, line, text.length),
      firstNonWhitespaceCharacterIndex: first < 0 ? text.length : first,
      isEmptyOrWhitespace: first < 0,
    };
  }
  offsetAt(position) {
    const p = this.validatePosition(position);
    return this._lineStarts[p.line] + p.character;
  }
  positionAt(offset) {
    offset = Math.min(Math.max(0, offset), this._text.length);
    const starts = this._lineStarts;
    let lo = 0;
    let hi = starts.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (starts[mid] <= offset) lo = mid;
      else hi = mid - 1;
    }
    return new Position(lo, Math.min(offset - starts[lo], this._lineEnd(lo) - starts[lo]));
  }
  validatePosition(position) {
    const line = Math.min(Math.max(0, position.line), this.lineCount - 1);
    if (position.line > line) return new Position(line, this._lineEnd(line) - this._lineStarts[line]);
    const max = this._lineEnd(line) - this._lineStarts[line];
    return new Position(line, Math.min(Math.max(0, position.character), max));
  }
  validateRange(range) {
    return new Range(this.validatePosition(range.start), this.validatePosition(range.end));
  }
  getWordRangeAtPosition(position, regex) {
    const p = this.validatePosition(position);
    const text = this.lineAt(p.line).text;
    const re = new RegExp((regex || /[A-Za-z0-9_$]+/).source, 'g');
    for (let m; (m = re.exec(text)); ) {
      if (m[0].length === 0) re.lastIndex++;
      else if (m.index <= p.character && p.character <= m.index + m[0].length) return new Range(p.line, m.index, p.line, m.index + m[0].length);
    }
    return undefined;
  }
  save() {
    return Promise.resolve(true);
  }
}

class TextEditor {
  constructor(document) {
    this.document = document;
    this.selection = new Selection(0, 0, 0, 0);
    this.selections = [this.selection];
    this.visibleRanges = [new Range(0, 0, Math.max(0, document.lineCount - 1), 0)];
    this.options = { tabSize: 4, indentSize: 4, insertSpaces: true, cursorStyle: 1, lineNumbers: 1 };
    this.viewColumn = 1;
    this._decorations = new Map();
  }
  setDecorations(type, list) {
    if (!type || !type.key) return;
    const base = type._options || {};
    const doc = this.document;
    const col = (p) => displayColumn(doc.lineAt(p.line).text, p.character, state.tabWidth);
    const items = (Array.isArray(list) ? list : []).slice(0, 2000).map((entry) => {
      const range = entry.range || entry;
      const own = entry.renderOptions || {};
      const after = { ...(base.after || {}), ...(own.after || {}) };
      return {
        line: range.start.line,
        col: col(range.start),
        endLine: range.end.line,
        endCol: col(range.end),
        wholeLine: !!base.isWholeLine,
        text: String(after.contentText || '').slice(0, 300),
        color: colorName(after.color || own.color || base.color),
        background: colorName(own.backgroundColor || base.backgroundColor || after.backgroundColor),
      };
    });
    if (items.length === 0 && !this._decorations.has(type.key)) return;
    if (items.length) this._decorations.set(type.key, items);
    else this._decorations.delete(type.key);
    send({ type: 'decorations', path: appPath(doc.uri), key: type.key, items });
  }
  edit() {
    unsupported('TextEditor.edit');
    return Promise.resolve(false);
  }
  insertSnippet() {
    return Promise.resolve(false);
  }
  revealRange() {}
  show() {}
  hide() {}
}

function colorName(c) {
  if (!c) return '';
  if (typeof c === 'string') return c;
  return c.id ? 'theme:' + c.id : '';
}

let decorationSeq = 0;
function createTextEditorDecorationType(options) {
  const type = {
    key: 'decoration-' + ++decorationSeq,
    _options: options || {},
    dispose() {
      for (const doc of documents.values()) doc._editor.setDecorations(type, []);
    },
  };
  return type;
}

function record(name, fields, extra) {
  const C = {
    [name]: class {
      constructor(...args) {
        fields.forEach((f, i) => {
          this[f] = args[i];
        });
      }
    },
  }[name];
  if (extra) Object.assign(C, extra);
  return C;
}
const enumOf = (...names) => Object.fromEntries(names.map((n, i) => [n, i]));
const enumFrom1 = (...names) => Object.fromEntries(names.map((n, i) => [n, i + 1]));

const DiagnosticSeverity = { Error: 0, Warning: 1, Information: 2, Hint: 3 };

class Diagnostic {
  constructor(range, message, severity) {
    this.range = range;
    this.message = message;
    this.severity = severity === undefined ? DiagnosticSeverity.Error : severity;
  }
}

class CodeActionKind {
  constructor(value) {
    this.value = value;
  }
  append(part) {
    return new CodeActionKind(this.value ? this.value + '.' + part : part);
  }
  contains(other) {
    return this.value === other.value || other.value.startsWith(this.value + '.') || this.value === '';
  }
  intersects(other) {
    return this.contains(other) || other.contains(this);
  }
}
CodeActionKind.Empty = new CodeActionKind('');
CodeActionKind.QuickFix = new CodeActionKind('quickfix');
CodeActionKind.Refactor = new CodeActionKind('refactor');
CodeActionKind.RefactorExtract = new CodeActionKind('refactor.extract');
CodeActionKind.RefactorInline = new CodeActionKind('refactor.inline');
CodeActionKind.RefactorMove = new CodeActionKind('refactor.move');
CodeActionKind.RefactorRewrite = new CodeActionKind('refactor.rewrite');
CodeActionKind.Source = new CodeActionKind('source');
CodeActionKind.SourceOrganizeImports = new CodeActionKind('source.organizeImports');
CodeActionKind.SourceFixAll = new CodeActionKind('source.fixAll');
CodeActionKind.Notebook = new CodeActionKind('notebook');

class DocumentDropOrPasteEditKind extends CodeActionKind {
  append(part) {
    return new DocumentDropOrPasteEditKind(this.value ? this.value + '.' + part : part);
  }
}
DocumentDropOrPasteEditKind.Empty = new DocumentDropOrPasteEditKind('');
DocumentDropOrPasteEditKind.Text = new DocumentDropOrPasteEditKind('text');
DocumentDropOrPasteEditKind.TextUpdateImports = new DocumentDropOrPasteEditKind('text.updateImports');

class MarkdownString {
  constructor(value, supportThemeIcons) {
    this.value = value || '';
    this.isTrusted = false;
    this.supportThemeIcons = !!supportThemeIcons;
    this.supportHtml = false;
  }
  appendText(v) {
    this.value += String(v).replace(/[\\`*_{}[\]()#+\-.!]/g, '\\$&');
    return this;
  }
  appendMarkdown(v) {
    this.value += v;
    return this;
  }
  appendCodeblock(code, language) {
    this.value += '\n```' + (language || '') + '\n' + code + '\n```\n';
    return this;
  }
}

class SnippetString {
  constructor(value) {
    this.value = value || '';
  }
  appendText(v) {
    this.value += String(v).replace(/[$}\\]/g, '\\$&');
    return this;
  }
  appendTabstop(n) {
    this.value += '$' + (n === undefined ? 0 : n);
    return this;
  }
  appendPlaceholder(v, n) {
    this.value += '${' + (n === undefined ? 1 : n) + ':' + (typeof v === 'function' ? '' : v) + '}';
    return this;
  }
  appendChoice(values, n) {
    this.value += '${' + (n === undefined ? 1 : n) + '|' + values.join(',') + '|}';
    return this;
  }
  appendVariable(name, def) {
    this.value += '${' + name + (typeof def === 'string' ? ':' + def : '') + '}';
    return this;
  }
}

class RelativePattern {
  constructor(base, pattern) {
    this.baseUri = typeof base === 'string' ? Uri.file(base) : base.uri || base;
    this.base = this.baseUri.fsPath;
    this.pattern = pattern;
  }
}

class SemanticTokensLegend {
  constructor(tokenTypes, tokenModifiers) {
    this.tokenTypes = tokenTypes;
    this.tokenModifiers = tokenModifiers || [];
  }
}
class SemanticTokensBuilder {
  constructor() {
    this._data = [];
  }
  push() {}
  build(resultId) {
    return { resultId, data: new Uint32Array(this._data) };
  }
}

class FileSystemError extends Error {
  constructor(messageOrUri, code) {
    super(typeof messageOrUri === 'string' ? messageOrUri : String(messageOrUri));
    this.code = code || 'Unknown';
    this.name = this.code;
  }
}
for (const code of ['FileNotFound', 'FileExists', 'FileNotADirectory', 'FileIsADirectory', 'NoPermissions', 'Unavailable']) {
  FileSystemError[code] = (m) => new FileSystemError(m || code, code);
}

class ThemeColor {
  constructor(id) {
    this.id = id;
  }
}
class ThemeIcon {
  constructor(id, color) {
    this.id = id;
    this.color = color;
  }
}
ThemeIcon.File = new ThemeIcon('file');
ThemeIcon.Folder = new ThemeIcon('folder');

class NotebookCellOutputItem {
  constructor(data, mime) {
    this.data = data;
    this.mime = mime;
  }
  static text(value, mime) {
    return new NotebookCellOutputItem(Buffer.from(String(value)), mime || 'text/plain');
  }
  static json(value, mime) {
    return new NotebookCellOutputItem(Buffer.from(JSON.stringify(value)), mime || 'text/x-json');
  }
  static stdout(value) {
    return NotebookCellOutputItem.text(value, 'application/vnd.code.notebook.stdout');
  }
  static stderr(value) {
    return NotebookCellOutputItem.text(value, 'application/vnd.code.notebook.stderr');
  }
  static error(err) {
    const plain = { name: err && err.name, message: err && err.message, stack: err && err.stack };
    return NotebookCellOutputItem.json(plain, 'application/vnd.code.notebook.error');
  }
}

class TabInputText {
  constructor(uri) {
    this.uri = uri;
  }
}

const classes = {
  Disposable,
  EventEmitter,
  CancellationTokenSource,
  CancellationError,
  Uri,
  Position,
  Range,
  Selection,
  Location,
  TextEdit,
  WorkspaceEdit,
  Diagnostic,
  CodeActionKind,
  DocumentDropOrPasteEditKind,
  MarkdownString,
  SnippetString,
  RelativePattern,
  SemanticTokensLegend,
  SemanticTokensBuilder,
  FileSystemError,
  ThemeColor,
  ThemeIcon,
  TabInputText,
  NotebookCellOutputItem,
  NotebookCellOutput: record('NotebookCellOutput', ['items', 'metadata']),
  SnippetTextEdit: record('SnippetTextEdit', ['range', 'snippet']),
  DiagnosticRelatedInformation: record('DiagnosticRelatedInformation', ['location', 'message']),
  CodeAction: record('CodeAction', ['title', 'kind']),
  CodeLens: record('CodeLens', ['range', 'command']),
  Hover: record('Hover', ['contents', 'range']),
  CompletionItem: record('CompletionItem', ['label', 'kind']),
  CompletionList: record('CompletionList', ['items', 'isIncomplete']),
  InlineCompletionItem: record('InlineCompletionItem', ['insertText', 'range', 'command']),
  InlineCompletionList: record('InlineCompletionList', ['items']),
  SignatureHelp: record('SignatureHelp', []),
  SignatureInformation: record('SignatureInformation', ['label', 'documentation']),
  ParameterInformation: record('ParameterInformation', ['label', 'documentation']),
  DocumentHighlight: record('DocumentHighlight', ['range', 'kind']),
  DocumentLink: record('DocumentLink', ['range', 'target']),
  DocumentSymbol: record('DocumentSymbol', ['name', 'detail', 'kind', 'range', 'selectionRange']),
  SymbolInformation: record('SymbolInformation', ['name', 'kind', 'containerName', 'location']),
  FoldingRange: record('FoldingRange', ['start', 'end', 'kind']),
  SelectionRange: record('SelectionRange', ['range', 'parent']),
  Color: record('Color', ['red', 'green', 'blue', 'alpha']),
  ColorInformation: record('ColorInformation', ['range', 'color']),
  ColorPresentation: record('ColorPresentation', ['label']),
  InlayHint: record('InlayHint', ['position', 'label', 'kind']),
  InlayHintLabelPart: record('InlayHintLabelPart', ['value']),
  CallHierarchyItem: record('CallHierarchyItem', ['kind', 'name', 'detail', 'uri', 'range', 'selectionRange']),
  CallHierarchyIncomingCall: record('CallHierarchyIncomingCall', ['from', 'fromRanges']),
  CallHierarchyOutgoingCall: record('CallHierarchyOutgoingCall', ['to', 'fromRanges']),
  TypeHierarchyItem: record('TypeHierarchyItem', ['kind', 'name', 'detail', 'uri', 'range', 'selectionRange']),
  LinkedEditingRanges: record('LinkedEditingRanges', ['ranges', 'wordPattern']),
  InlineValueText: record('InlineValueText', ['range', 'text']),
  InlineValueVariableLookup: record('InlineValueVariableLookup', ['range', 'variableName', 'caseSensitiveLookup']),
  InlineValueEvaluatableExpression: record('InlineValueEvaluatableExpression', ['range', 'expression']),
  SemanticTokens: record('SemanticTokens', ['data', 'resultId']),
  SemanticTokensEdits: record('SemanticTokensEdits', ['edits', 'resultId']),
  SemanticTokensEdit: record('SemanticTokensEdit', ['start', 'deleteCount', 'data']),
  EvaluatableExpression: record('EvaluatableExpression', ['range', 'expression']),
  TabInputTextDiff: record('TabInputTextDiff', ['original', 'modified']),
  TabInputCustom: record('TabInputCustom', ['uri', 'viewType']),
  TabInputNotebook: record('TabInputNotebook', ['uri', 'notebookType']),
  TabInputNotebookDiff: record('TabInputNotebookDiff', ['original', 'modified', 'notebookType']),
  TabInputWebview: record('TabInputWebview', ['viewType']),
  TabInputTerminal: record('TabInputTerminal', []),
  NotebookRange: record('NotebookRange', ['start', 'end']),
  NotebookCellData: record('NotebookCellData', ['kind', 'value', 'languageId']),
  NotebookData: record('NotebookData', ['cells']),
  TreeItem: record('TreeItem', ['label', 'collapsibleState']),
  QuickInputButtons: { Back: {} },
  ShellExecution: record('ShellExecution', ['commandLine', 'args', 'options']),
  ProcessExecution: record('ProcessExecution', ['process', 'args', 'options']),
  CustomExecution: record('CustomExecution', ['callback']),
  Task: record('Task', ['definition', 'scope', 'name', 'source', 'execution', 'problemMatchers']),
  TaskGroup: record('TaskGroup', ['id', 'label'], { Build: { id: 'build' }, Test: { id: 'test' }, Clean: { id: 'clean' }, Rebuild: { id: 'rebuild' } }),
  DocumentDropEdit: record('DocumentDropEdit', ['insertText']),
  DataTransferItem: record('DataTransferItem', ['value']),
  LanguageModelTextPart: record('LanguageModelTextPart', ['value']),
  TextDocumentContentChangeEvent: record('TextDocumentContentChangeEvent', ['range', 'rangeOffset', 'rangeLength', 'text']),
};

const enums = {
  EndOfLine,
  DiagnosticSeverity,
  DiagnosticTag: { Unnecessary: 1, Deprecated: 2 },
  CompletionItemKind: enumOf('Text', 'Method', 'Function', 'Constructor', 'Field', 'Variable', 'Class', 'Interface', 'Module', 'Property', 'Unit', 'Value', 'Enum', 'Keyword', 'Snippet', 'Color', 'File', 'Reference', 'Folder', 'EnumMember', 'Constant', 'Struct', 'Event', 'Operator', 'TypeParameter', 'User', 'Issue'),
  CompletionItemTag: { Deprecated: 1 },
  CompletionTriggerKind: enumOf('Invoke', 'TriggerCharacter', 'TriggerForIncompleteCompletions'),
  SymbolKind: enumOf('File', 'Module', 'Namespace', 'Package', 'Class', 'Method', 'Property', 'Field', 'Constructor', 'Enum', 'Interface', 'Function', 'Variable', 'Constant', 'String', 'Number', 'Boolean', 'Array', 'Object', 'Key', 'Null', 'EnumMember', 'Struct', 'Event', 'Operator', 'TypeParameter'),
  SymbolTag: { Deprecated: 1 },
  DocumentHighlightKind: enumOf('Text', 'Read', 'Write'),
  CodeActionTriggerKind: { Invoke: 1, Automatic: 2 },
  SignatureHelpTriggerKind: enumFrom1('Invoke', 'TriggerCharacter', 'ContentChange'),
  InlineCompletionTriggerKind: enumOf('Invoke', 'Automatic'),
  FoldingRangeKind: enumFrom1('Comment', 'Imports', 'Region'),
  InlayHintKind: enumFrom1('Type', 'Parameter'),
  TextDocumentSaveReason: enumFrom1('Manual', 'AfterDelay', 'FocusOut'),
  TextDocumentChangeReason: enumFrom1('Undo', 'Redo'),
  TextEditorRevealType: enumOf('Default', 'InCenter', 'InCenterIfOutsideViewport', 'AtTop'),
  TextEditorCursorStyle: enumFrom1('Line', 'Block', 'Underline', 'LineThin', 'BlockOutline', 'UnderlineThin'),
  TextEditorLineNumbersStyle: enumOf('Off', 'On', 'Relative', 'Interval'),
  TextEditorSelectionChangeKind: enumFrom1('Keyboard', 'Mouse', 'Command'),
  OverviewRulerLane: { Left: 1, Center: 2, Right: 4, Full: 7 },
  DecorationRangeBehavior: enumOf('OpenOpen', 'ClosedClosed', 'OpenClosed', 'ClosedOpen'),
  ViewColumn: { Active: -1, Beside: -2, One: 1, Two: 2, Three: 3, Four: 4, Five: 5, Six: 6, Seven: 7, Eight: 8, Nine: 9 },
  StatusBarAlignment: { Left: 1, Right: 2 },
  ProgressLocation: { SourceControl: 1, Window: 10, Notification: 15 },
  ConfigurationTarget: { Global: 1, Workspace: 2, WorkspaceFolder: 3 },
  FileType: { Unknown: 0, File: 1, Directory: 2, SymbolicLink: 64 },
  FileChangeType: enumFrom1('Changed', 'Created', 'Deleted'),
  ExtensionMode: enumFrom1('Production', 'Development', 'Test'),
  ExtensionKind: { UI: 1, Workspace: 2 },
  UIKind: { Desktop: 1, Web: 2 },
  ColorThemeKind: enumFrom1('Light', 'Dark', 'HighContrast', 'HighContrastLight'),
  LogLevel: enumOf('Off', 'Trace', 'Debug', 'Info', 'Warning', 'Error'),
  LanguageStatusSeverity: enumOf('Information', 'Warning', 'Error'),
  NotebookCellKind: { Markup: 1, Code: 2 },
  NotebookEditorRevealType: enumOf('Default', 'InCenter', 'InCenterIfOutsideViewport', 'AtTop'),
  TreeItemCollapsibleState: enumOf('None', 'Collapsed', 'Expanded'),
  TreeItemCheckboxState: enumOf('Unchecked', 'Checked'),
  QuickPickItemKind: { Separator: -1, Default: 0 },
  InputBoxValidationSeverity: enumFrom1('Info', 'Warning', 'Error'),
  TaskRevealKind: enumFrom1('Always', 'Silent', 'Never'),
  TaskPanelKind: enumFrom1('Shared', 'Dedicated', 'New'),
  TaskScope: { Global: 1, Workspace: 2 },
  ShellQuoting: enumFrom1('Escape', 'Strong', 'Weak'),
  IndentAction: enumOf('None', 'Indent', 'IndentOutdent', 'Outdent'),
  CommentThreadCollapsibleState: enumOf('Collapsed', 'Expanded'),
  DebugConsoleMode: enumOf('Separate', 'MergeWithParent'),
  EnvironmentVariableMutatorType: enumFrom1('Replace', 'Append', 'Prepend'),
  TerminalLocation: enumFrom1('Panel', 'Editor'),
  SyntaxTokenType: enumOf('Other', 'Comment', 'String', 'RegEx'),
  InlineCompletionEndOfLifeReasonKind: enumOf('Accepted', 'Rejected', 'Ignored'),
};

function parseJsonc(text) {
  let out = '';
  let i = 0;
  while (i < text.length) {
    const c = text[i];
    if (c === '"') {
      let j = i + 1;
      while (j < text.length && text[j] !== '"') j += text[j] === '\\' ? 2 : 1;
      out += text.slice(i, j + 1);
      i = j + 1;
    } else if (c === '/' && text[i + 1] === '/') {
      while (i < text.length && text[i] !== '\n') i++;
    } else if (c === '/' && text[i + 1] === '*') {
      const end = text.indexOf('*/', i + 2);
      i = end < 0 ? text.length : end + 2;
    } else {
      out += c;
      i++;
    }
  }
  return JSON.parse(out.replace(/,(\s*[}\]])/g, '$1'));
}
function readSettings(file) {
  try {
    const v = parseJsonc(fs.readFileSync(file, 'utf8').replace(/^﻿/, ''));
    return v && typeof v === 'object' && !Array.isArray(v) ? v : {};
  } catch (e) {
    if (e.code !== 'ENOENT') log('warn', 'host', `settings not read: ${file}: ${e.message}`);
    return {};
  }
}
const isPlain = (v) => v && typeof v === 'object' && !Array.isArray(v);
function setDotted(tree, key, value) {
  const parts = key.split('.');
  let node = tree;
  for (let i = 0; i < parts.length - 1; i++) {
    if (!isPlain(node[parts[i]])) node[parts[i]] = {};
    node = node[parts[i]];
  }
  const last = parts[parts.length - 1];
  node[last] = isPlain(value) && isPlain(node[last]) ? { ...node[last], ...value } : value;
}
function getDotted(tree, key) {
  let node = tree;
  for (const part of key ? key.split('.') : []) {
    if (!isPlain(node)) return undefined;
    node = node[part];
  }
  return node;
}
function layerTree(flat, languageId) {
  const tree = {};
  for (const [k, v] of Object.entries(flat)) if (!k.startsWith('[')) setDotted(tree, k, v);
  if (languageId) {
    for (const [k, v] of Object.entries(flat)) {
      if (k.startsWith('[') && isPlain(v) && k.slice(1, -1).split('][').includes(languageId)) {
        for (const [kk, vv] of Object.entries(v)) setDotted(tree, kk, vv);
      }
    }
  }
  return tree;
}

const config = {
  defaults: {
    'editor.tabSize': 4,
    'editor.insertSpaces': true,
    'editor.detectIndentation': true,
    'editor.formatOnSave': true,
    'editor.defaultFormatter': 'esbenp.prettier-vscode',
    '[markdown]': { 'editor.formatOnSave': false },
    '[plaintext]': { 'editor.formatOnSave': false },
    'editor.codeActionsOnSave': {},
    'files.eol': 'auto',
    'files.encoding': 'utf8',
    'files.trimTrailingWhitespace': false,
    'files.insertFinalNewline': false,
    'files.autoSave': 'off',
    'files.exclude': { '**/.git': true, '**/node_modules': true },
    'search.exclude': { '**/node_modules': true },
    'http.proxy': '',
    'http.proxyStrictSSL': true,
    'telemetry.telemetryLevel': 'off',
  },
  userFile: '',
  user: {},
  imported: {},
  folders: new Map(),
  changed: new EventEmitter(),
};

function folderSettingsFor(uri) {
  const folder = uri ? workspace.getWorkspaceFolder(uri) : workspace.workspaceFolders[0];
  return (folder && config.folders.get(folder.uri.fsPath)) || {};
}

function getConfiguration(section, scopeArg) {
  let uri;
  let languageId;
  if (scopeArg instanceof Uri) uri = scopeArg;
  else if (scopeArg && scopeArg.uri instanceof Uri) {
    uri = scopeArg.uri;
    languageId = scopeArg.languageId;
  }
  if (!languageId && uri) {
    const doc = documents.get(uriKey(uri));
    languageId = doc ? doc.languageId : undefined;
  }
  const layers = { defaultValue: config.defaults, globalValue: { ...config.imported, ...config.user }, workspaceValue: folderSettingsFor(uri) };
  const trees = Object.fromEntries(Object.entries(layers).map(([k, flat]) => [k, layerTree(flat, languageId)]));
  const lookup = (tree, key) => getDotted(tree, [section, key].filter(Boolean).join('.'));
  const merged = (key) => {
    let value;
    for (const tree of [trees.defaultValue, trees.globalValue, trees.workspaceValue]) {
      const v = lookup(tree, key);
      if (v === undefined) continue;
      value = isPlain(v) && isPlain(value) ? deepMerge(value, v) : v;
    }
    return value;
  };
  const api = {
    get(key, fallback) {
      const v = merged(key);
      return v === undefined ? fallback : v;
    },
    has(key) {
      return merged(key) !== undefined;
    },
    inspect(key) {
      const out = { key: [section, key].filter(Boolean).join('.') };
      for (const [name, tree] of Object.entries(trees)) out[name] = lookup(tree, key);
      return out;
    },
    update(key, value, target) {
      const full = [section, key].filter(Boolean).join('.');
      const toFolder = target === 2 || target === 3 || target === false;
      const folder = uri ? workspace.getWorkspaceFolder(uri) : workspace.workspaceFolders[0];
      const flat = toFolder && folder ? config.folders.get(folder.uri.fsPath) || {} : config.user;
      if (value === undefined) delete flat[full];
      else flat[full] = value;
      const file = toFolder && folder ? path.join(folder.uri.fsPath, '.vscode', 'settings.json') : config.userFile;
      if (toFolder && folder) config.folders.set(folder.uri.fsPath, flat);
      try {
        fs.mkdirSync(path.dirname(file), { recursive: true });
        fs.writeFileSync(file, JSON.stringify(flat, null, 2) + '\n');
      } catch (e) {
        return Promise.reject(e);
      }
      config.changed.fire({ affectsConfiguration: (s) => full === s || full.startsWith(s + '.') || s.startsWith(full + '.') });
      return Promise.resolve();
    },
  };
  const whole = merged('');
  return isPlain(whole) ? Object.assign(Object.create(api), whole, api) : api;
}
function deepMerge(a, b) {
  const out = { ...a };
  for (const [k, v] of Object.entries(b)) out[k] = isPlain(v) && isPlain(out[k]) ? deepMerge(out[k], v) : v;
  return out;
}

const BUILTIN_LANGUAGES = {
  javascript: ['.js', '.mjs', '.cjs', '.es6'],
  javascriptreact: ['.jsx'],
  typescript: ['.ts', '.mts', '.cts'],
  typescriptreact: ['.tsx'],
  json: ['.json', '.webmanifest', '.map', '.har'],
  jsonc: ['.jsonc', '.eslintrc', '.babelrc', '.swcrc', '.hintrc'],
  html: ['.html', '.htm', '.xhtml'],
  css: ['.css'],
  scss: ['.scss'],
  less: ['.less'],
  markdown: ['.md', '.markdown', '.mdown'],
  yaml: ['.yml', '.yaml'],
  xml: ['.xml', '.xsd', '.svg', '.xaml', '.csproj'],
  rust: ['.rs'],
  python: ['.py', '.pyi', '.pyw'],
  go: ['.go'],
  c: ['.c', '.i'],
  cpp: ['.cpp', '.cc', '.cxx', '.hpp', '.hh', '.hxx', '.h', '.ino', '.inl'],
  csharp: ['.cs', '.csx'],
  java: ['.java'],
  php: ['.php', '.phtml'],
  ruby: ['.rb', '.gemspec'],
  swift: ['.swift'],
  lua: ['.lua'],
  shellscript: ['.sh', '.bash', '.zsh'],
  powershell: ['.ps1', '.psm1', '.psd1'],
  bat: ['.bat', '.cmd'],
  sql: ['.sql'],
  dockerfile: ['.dockerfile'],
  toml: ['.toml'],
  ini: ['.ini'],
  graphql: ['.graphql', '.gql'],
  vue: ['.vue'],
  svelte: ['.svelte'],
  handlebars: ['.handlebars', '.hbs'],
  plaintext: ['.txt'],
};
const BUILTIN_FILENAMES = { Dockerfile: 'dockerfile', Makefile: 'makefile', 'tsconfig.json': 'jsonc', 'jsconfig.json': 'jsonc', '.eslintrc.json': 'jsonc' };
const byExtension = new Map();
const byFilename = new Map(Object.entries(BUILTIN_FILENAMES));
const knownLanguages = new Set(Object.keys(BUILTIN_LANGUAGES));
for (const [id, exts] of Object.entries(BUILTIN_LANGUAGES)) for (const e of exts) byExtension.set(e, id);

function languageOf(file) {
  const base = path.basename(file);
  if (byFilename.has(base)) return byFilename.get(base);
  const lower = base.toLowerCase();
  for (let i = lower.indexOf('.'); i >= 0; i = lower.indexOf('.', i + 1)) {
    const id = byExtension.get(lower.slice(i));
    if (id) return id;
  }
  return 'plaintext';
}

function globToRegExp(glob) {
  let re = '';
  for (let i = 0; i < glob.length; i++) {
    const c = glob[i];
    if (c === '*') {
      if (glob[i + 1] === '*') {
        re += glob[i + 2] === '/' ? '(?:.*/)?' : '.*';
        i += glob[i + 2] === '/' ? 2 : 1;
      } else re += '[^/]*';
    } else if (c === '?') re += '[^/]';
    else if (c === '{') {
      const end = glob.indexOf('}', i);
      if (end < 0) re += '\\{';
      else {
        re += '(?:' + glob.slice(i + 1, end).split(',').map((p) => globToRegExp(p).source.slice(1, -1)).join('|') + ')';
        i = end;
      }
    } else re += /[.+^$()|[\]\\]/.test(c) ? '\\' + c : c;
  }
  return new RegExp('^' + re + '$', WIN ? 'i' : '');
}
function matchesPattern(pattern, uri) {
  const file = uri.fsPath.replace(/\\/g, '/');
  if (typeof pattern === 'string') {
    const re = globToRegExp(pattern);
    return re.test(file) || re.test(path.posix.basename(file)) || workspace.workspaceFolders.some((f) => re.test(path.posix.relative(f.uri.fsPath.replace(/\\/g, '/'), file)));
  }
  const base = String(pattern.base || (pattern.baseUri && pattern.baseUri.fsPath) || '').replace(/\\/g, '/');
  const rel = path.posix.relative(WIN ? base.toLowerCase() : base, WIN ? file.toLowerCase() : file);
  return !rel.startsWith('..') && globToRegExp(pattern.pattern).test(rel);
}
function scoreSelector(selector, doc) {
  if (Array.isArray(selector)) return Math.max(0, ...selector.map((s) => scoreSelector(s, doc)));
  if (typeof selector === 'string') return selector === '*' ? 5 : selector === doc.languageId ? 10 : 0;
  if (!selector) return 0;
  let score = 0;
  if (selector.notebookType) return 0;
  if (selector.scheme) {
    if (selector.scheme === doc.uri.scheme) score = 10;
    else if (selector.scheme === '*') score = 5;
    else return 0;
  }
  if (selector.language) {
    if (selector.language === doc.languageId) score = 10;
    else if (selector.language === '*') score = Math.max(score, 5);
    else return 0;
  }
  if (selector.pattern) {
    if (matchesPattern(selector.pattern, doc.uri)) score = 10;
    else return 0;
  }
  return score;
}

const collections = new Set();
const diagnosticsChanged = new EventEmitter();
const dirtyDiagnostics = new Map();
let diagnosticsTimer = null;

function touchDiagnostics(uri) {
  dirtyDiagnostics.set(uriKey(uri), uri);
  if (!diagnosticsTimer) diagnosticsTimer = setTimeout(flushDiagnostics, 30);
}
function allDiagnostics(uri) {
  const key = uriKey(uri);
  const out = [];
  for (const c of collections) for (const d of c._map.get(key)?.items || []) out.push(d);
  return out;
}
function displayColumn(text, character, tab) {
  let col = 0;
  for (let i = 0; i < Math.min(character, text.length); i++) {
    const code = text.charCodeAt(i);
    if (code === 9) col += tab - (col % tab);
    else if (code >= 0xdc00 && code <= 0xdfff) continue;
    else col++;
  }
  return col;
}
function characterAt(text, col, tab) {
  let at = 0;
  for (let i = 0; i < text.length; i++) {
    const code = text.charCodeAt(i);
    if (code >= 0xdc00 && code <= 0xdfff) continue;
    const w = code === 9 ? tab - (at % tab) : 1;
    if (at + w > col) return i;
    at += w;
  }
  return text.length;
}
function positionAt(doc, line, col) {
  return new Position(line, characterAt(doc.lineAt(line).text, col, state.tabWidth));
}

function flushDiagnostics() {
  diagnosticsTimer = null;
  const uris = [...dirtyDiagnostics.values()];
  dirtyDiagnostics.clear();
  for (const uri of uris) {
    if (uri.scheme !== 'file') continue;
    const doc = documents.get(uriKey(uri)) || loadDocument(uri, true);
    const items = allDiagnostics(uri).map((d) => {
      const r = d.range || new Range(0, 0, 0, 0);
      const col = (p) => (doc ? displayColumn(doc.lineAt(p.line).text, p.character, state.tabWidth) : p.character);
      const code = d.code && typeof d.code === 'object' ? d.code.value : d.code;
      return {
        line: r.start.line,
        col: col(r.start),
        endLine: r.end.line,
        endCol: col(r.end),
        severity: d.severity === undefined ? 0 : d.severity,
        message: String(d.message || ''),
        source: d.source || '',
        code: code === undefined || code === null ? '' : String(code),
      };
    });
    send({ type: 'diagnostics', path: appPath(uri), items });
  }
  diagnosticsChanged.fire({ uris });
}

function createDiagnosticCollection(name) {
  const c = {
    name: name || '',
    _map: new Map(),
    set(target, items) {
      if (Array.isArray(target)) {
        const grouped = new Map();
        for (const [uri, list] of target) {
          const key = uriKey(uri);
          if (!grouped.has(key)) grouped.set(key, { uri, items: [] });
          if (list) grouped.get(key).items.push(...list);
        }
        for (const g of grouped.values()) c.set(g.uri, g.items);
        return;
      }
      if (!items || items.length === 0) c._map.delete(uriKey(target));
      else c._map.set(uriKey(target), { uri: target, items: items.slice() });
      touchDiagnostics(target);
    },
    delete(uri) {
      c.set(uri, undefined);
    },
    clear() {
      const old = [...c._map.values()];
      c._map.clear();
      for (const e of old) touchDiagnostics(e.uri);
    },
    forEach(cb, thisArg) {
      for (const e of c._map.values()) cb.call(thisArg, e.uri, e.items, c);
    },
    get(uri) {
      return c._map.get(uriKey(uri))?.items;
    },
    has(uri) {
      return c._map.has(uriKey(uri));
    },
    dispose() {
      c.clear();
      collections.delete(c);
    },
    [Symbol.iterator]() {
      return [...c._map.values()].map((e) => [e.uri, e.items])[Symbol.iterator]();
    },
  };
  collections.add(c);
  return c;
}

const state = {
  tabWidth: 4,
  folders: [],
  extensions: new Map(),
  commands: new Map(),
  providers: [],
  storageDir: '',
  focused: null,
};
const documents = new Map();
const appPaths = new Map();
const tabs = [];

const events = {
  openDoc: new EventEmitter(),
  closeDoc: new EventEmitter(),
  changeDoc: new EventEmitter(),
  saveDoc: new EventEmitter(),
  willSaveDoc: new EventEmitter(),
  activeEditor: new EventEmitter(),
  visibleEditors: new EventEmitter(),
  tabs: new EventEmitter(),
  folders: new EventEmitter(),
};

const appPath = (uri) => appPaths.get(uriKey(uri)) || uri.fsPath;

function loadDocument(uri, quiet) {
  try {
    return new TextDocument(uri, languageOf(uri.fsPath), fs.readFileSync(uri.fsPath, 'utf8'));
  } catch (e) {
    if (!quiet) log('warn', 'host', `cannot read ${uri.fsPath}: ${e.message}`);
    return null;
  }
}

function activeEditor() {
  return state.focused ? state.focused._editor : undefined;
}
function tabGroup() {
  return { isActive: true, viewColumn: 1, activeTab: tabs.find((t) => t.isActive), tabs };
}

function openDocument(file, focus) {
  const uri = Uri.file(file);
  const key = uriKey(uri);
  let doc = documents.get(key);
  if (!doc) {
    doc = loadDocument(uri);
    if (!doc) return null;
    doc._editor = new TextEditor(doc);
    documents.set(key, doc);
    appPaths.set(key, file);
    const tab = { label: path.basename(file), input: new TabInputText(uri), isActive: false, isDirty: false, isPinned: false, isPreview: false, get group() { return tabGroup(); } };
    doc._tab = tab;
    tabs.push(tab);
    activateByEvent('onLanguage:' + doc.languageId);
    events.openDoc.fire(doc);
    events.tabs.fire({ opened: [tab], closed: [], changed: [] });
    events.visibleEditors.fire([...documents.values()].map((d) => d._editor));
  }
  if ((focus || !state.focused) && state.focused !== doc) {
    for (const t of tabs) t.isActive = t === doc._tab;
    state.focused = doc;
    events.activeEditor.fire(doc._editor);
    events.tabs.fire({ opened: [], closed: [], changed: [doc._tab] });
  }
  return doc;
}

function closeDocument(file) {
  const key = fileKey(file);
  const doc = documents.get(key);
  if (!doc) return;
  documents.delete(key);
  doc.isClosed = true;
  tabs.splice(tabs.indexOf(doc._tab), 1);
  if (state.focused === doc) {
    state.focused = null;
    events.activeEditor.fire(undefined);
  }
  events.closeDoc.fire(doc);
  events.tabs.fire({ opened: [], closed: [doc._tab], changed: [] });
  events.visibleEditors.fire([...documents.values()].map((d) => d._editor));
}

function reloadDocument(file) {
  const doc = documents.get(fileKey(file));
  if (!doc) return;
  let text;
  try {
    text = fs.readFileSync(doc.uri.fsPath, 'utf8');
  } catch {
    return closeDocument(file);
  }
  if (text === doc.getText()) {
    if (doc.isDirty) {
      doc.isDirty = false;
      events.saveDoc.fire(doc);
    }
    return;
  }
  doc.isDirty = false;
  const whole = new Range(0, 0, doc.lineCount, 0);
  const rangeLength = doc.getText().length;
  doc._setText(text);
  doc.version++;
  events.changeDoc.fire({ document: doc, reason: undefined, contentChanges: [{ range: whole, rangeOffset: 0, rangeLength, text }] });
  events.saveDoc.fire(doc);
}

function editDocument(file, text) {
  const doc = documents.get(fileKey(file)) || openDocument(file, true);
  if (!doc || text === doc.getText()) return;
  const whole = new Range(0, 0, doc.lineCount, 0);
  const rangeLength = doc.getText().length;
  doc._setText(text);
  doc.version++;
  doc.isDirty = true;
  events.changeDoc.fire({ document: doc, reason: undefined, contentChanges: [{ range: whole, rangeOffset: 0, rangeLength, text }] });
}

const disposableStub = () => new Disposable(() => {});

function permissive(name, target) {
  return new Proxy(target, {
    get(obj, prop) {
      if (prop in obj || typeof prop === 'symbol' || prop === 'then') return obj[prop];
      unsupported(`${name}.${prop}`);
      if (/^on(Did|Will)[A-Z]/.test(prop)) return neverEvent;
      if (/^(register|create|set|add)[A-Z]/.test(prop)) return () => permissive(`${name}.${prop}()`, { dispose() {} });
      return () => undefined;
    },
  });
}

const RESOURCE_ORIGIN = 'https://ccres.localhost';
const webviews = new Map();
const webviewProviders = new Map();
let webviewSeq = 0;

function createWebview(extension, options) {
  const handle = 'w' + ++webviewSeq;
  const received = new EventEmitter();
  let html = '';
  let opts = options || {};
  const webview = {
    get options() {
      return opts;
    },
    set options(v) {
      opts = v || {};
    },
    get html() {
      return html;
    },
    set html(v) {
      html = String(v);
      send({ type: 'webviewHtml', handle, html });
    },
    cspSource: RESOURCE_ORIGIN,
    asWebviewUri(uri) {
      const file = uri.fsPath.replace(/\\/g, '/');
      return Uri.parse(RESOURCE_ORIGIN + '/file/' + file.split('/').map(encodeURIComponent).join('/'));
    },
    postMessage(message) {
      send({ type: 'webviewPost', handle, message });
      return Promise.resolve(true);
    },
    onDidReceiveMessage: received.event,
  };
  const entry = { webview, received, extension, closed: new EventEmitter() };
  webviews.set(handle, entry);
  return { handle, ...entry };
}

function viewTitle(extension, view) {
  const ext = state.extensions.get(extension);
  const views = (ext && ext.manifest.contributes && ext.manifest.contributes.views) || {};
  for (const list of Object.values(views)) for (const v of list || []) if (v.id === view) return v.name || ext.manifest.displayName || view;
  return (ext && ext.manifest.displayName) || view;
}

function registerWebviewViewProvider(view, provider) {
  const extension = currentId() || '';
  webviewProviders.set(view, { provider, extension });
  send({ type: 'webviewView', view, extension, title: viewTitle(extension, view) });
  return new Disposable(() => webviewProviders.delete(view));
}

function createWebviewPanel(viewType, title, _show, options) {
  const extension = currentId() || '';
  const w = createWebview(extension, options);
  const viewState = new EventEmitter();
  let name = String(title || viewType);
  const panel = {
    viewType,
    webview: w.webview,
    options: options || {},
    viewColumn: 1,
    active: true,
    visible: true,
    iconPath: undefined,
    get title() {
      return name;
    },
    set title(v) {
      name = String(v);
      send({ type: 'webviewPanel', handle: w.handle, extension, title: name });
    },
    onDidDispose: w.closed.event,
    onDidChangeViewState: viewState.event,
    reveal() {
      send({ type: 'webviewPanel', handle: w.handle, extension, title: name, reveal: true });
    },
    dispose() {
      if (!webviews.delete(w.handle)) return;
      send({ type: 'webviewClosed', handle: w.handle });
      w.closed.fire();
    },
  };
  send({ type: 'webviewPanel', handle: w.handle, extension, title: name, reveal: true });
  return panel;
}

const pendingMessages = new Map();
let messageSeq = 0;
const itemTitle = (i) => (typeof i === 'string' ? i : i.title);

function showMessage(level) {
  return (message, ...rest) => {
    const items = rest.filter((r) => typeof r === 'string' || (r && typeof r.title === 'string'));
    const detail = rest.find((r) => r && typeof r === 'object' && typeof r.detail === 'string');
    const text = String(message) + (detail ? ' ' + detail.detail : '');
    const base = { type: 'message', level, extension: currentId() || '', text };
    if (items.length === 0) {
      send({ ...base, actions: [] });
      return Promise.resolve(undefined);
    }
    const id = ++messageSeq;
    send({ ...base, id, actions: items.map(itemTitle) });
    return new Promise((resolve) => pendingMessages.set(id, { resolve, items }));
  };
}

function createOutputChannel(name) {
  const source = String(name);
  let pending = '';
  const line = (level, text) => log(level, source, text);
  const channel = {
    name: source,
    logLevel: 3,
    onDidChangeLogLevel: neverEvent,
    append(v) {
      pending += v;
      const at = pending.lastIndexOf('\n');
      if (at >= 0) {
        line('info', pending.slice(0, at));
        pending = pending.slice(at + 1);
      }
    },
    appendLine(v) {
      line('info', pending + v);
      pending = '';
    },
    replace(v) {
      line('info', v);
    },
    clear() {},
    show() {},
    hide() {},
    dispose() {},
    trace: (...a) => line('debug', fmt(a)),
    debug: (...a) => line('debug', fmt(a)),
    info: (...a) => line('info', fmt(a)),
    warn: (...a) => line('warn', fmt(a)),
    error: (...a) => line('error', fmt(a)),
  };
  return channel;
}

let statusSeq = 0;
function createStatusBarItem(a, b, c) {
  const item = { id: typeof a === 'string' ? a : `status.${++statusSeq}`, alignment: typeof a === 'number' ? a : b, priority: typeof a === 'number' ? b : c, text: '', tooltip: undefined, command: undefined, name: undefined, color: undefined, backgroundColor: undefined, accessibilityInformation: undefined };
  const extension = currentId() || '';
  const plain = (v) => (v && typeof v === 'object' ? v.value || '' : v || '');
  item.show = () => send({ type: 'status', extension, id: item.id, visible: true, text: String(item.text || ''), tooltip: String(plain(item.tooltip)) });
  item.hide = () => send({ type: 'status', extension, id: item.id, visible: false, text: '', tooltip: '' });
  item.dispose = item.hide;
  return item;
}

function fileStat(s) {
  return { type: s.isDirectory() ? 2 : s.isSymbolicLink() ? 64 : 1, ctime: s.ctimeMs, mtime: s.mtimeMs, size: s.size };
}
const fsError = (e, uri) => (e && e.code === 'ENOENT' ? FileSystemError.FileNotFound(uri.fsPath) : e);
const workspaceFs = {
  stat: (uri) => fs.promises.stat(uri.fsPath).then(fileStat, (e) => Promise.reject(fsError(e, uri))),
  readFile: (uri) => fs.promises.readFile(uri.fsPath).then((b) => new Uint8Array(b), (e) => Promise.reject(fsError(e, uri))),
  writeFile: (uri, content) => fs.promises.writeFile(uri.fsPath, content),
  readDirectory: (uri) => fs.promises.readdir(uri.fsPath, { withFileTypes: true }).then((list) => list.map((d) => [d.name, d.isDirectory() ? 2 : d.isSymbolicLink() ? 64 : 1])),
  createDirectory: (uri) => fs.promises.mkdir(uri.fsPath, { recursive: true }).then(() => {}),
  delete: (uri, o) => fs.promises.rm(uri.fsPath, { recursive: !!(o && o.recursive), force: true }),
  rename: (from, to) => fs.promises.rename(from.fsPath, to.fsPath),
  copy: (from, to) => fs.promises.cp(from.fsPath, to.fsPath, { recursive: true }),
  isWritableFileSystem: (scheme) => (scheme === 'file' ? true : undefined),
};

function createFileSystemWatcher() {
  return { ignoreCreateEvents: false, ignoreChangeEvents: false, ignoreDeleteEvents: false, onDidCreate: neverEvent, onDidChange: neverEvent, onDidDelete: neverEvent, dispose() {} };
}

function applyWorkspaceEdit(edit) {
  if (!(edit instanceof WorkspaceEdit)) return Promise.resolve(false);
  for (const [uri, edits] of edit.entries()) {
    const doc = documents.get(uriKey(uri)) || loadDocument(uri);
    if (!doc) return Promise.resolve(false);
    const text = applyEdits(doc, edits);
    if (text !== doc.getText()) send({ type: 'write', path: appPath(uri), text, extension: currentId() || '' });
  }
  return Promise.resolve(true);
}

const workspace = permissive('workspace', {
  get workspaceFolders() {
    return state.folders;
  },
  get name() {
    return state.folders[0] ? state.folders[0].name : undefined;
  },
  get rootPath() {
    return state.folders[0] ? state.folders[0].uri.fsPath : undefined;
  },
  workspaceFile: undefined,
  isTrusted: true,
  get textDocuments() {
    return [...documents.values()];
  },
  notebookDocuments: [],
  fs: workspaceFs,
  getWorkspaceFolder(uri) {
    const file = fileKey(uri.fsPath);
    let best;
    for (const f of state.folders) {
      const root = fileKey(f.uri.fsPath);
      if ((file === root || file.startsWith(root + path.sep)) && (!best || root.length > fileKey(best.uri.fsPath).length)) best = f;
    }
    return best;
  },
  asRelativePath(pathOrUri, includeFolder) {
    const file = typeof pathOrUri === 'string' ? pathOrUri : pathOrUri.fsPath;
    const folder = workspace.getWorkspaceFolder(Uri.file(file));
    if (!folder) return file;
    const rel = path.relative(folder.uri.fsPath, file).replace(/\\/g, '/');
    return includeFolder && state.folders.length > 1 ? `${folder.name}/${rel}` : rel;
  },
  getConfiguration,
  onDidChangeConfiguration: config.changed.event,
  onDidOpenTextDocument: events.openDoc.event,
  onDidCloseTextDocument: events.closeDoc.event,
  onDidChangeTextDocument: events.changeDoc.event,
  onDidSaveTextDocument: events.saveDoc.event,
  onWillSaveTextDocument: events.willSaveDoc.event,
  onDidChangeWorkspaceFolders: events.folders.event,
  onDidGrantWorkspaceTrust: neverEvent,
  onDidOpenNotebookDocument: neverEvent,
  onDidCloseNotebookDocument: neverEvent,
  onDidChangeNotebookDocument: neverEvent,
  onDidSaveNotebookDocument: neverEvent,
  onWillCreateFiles: neverEvent,
  onWillDeleteFiles: neverEvent,
  onWillRenameFiles: neverEvent,
  onDidCreateFiles: neverEvent,
  onDidDeleteFiles: neverEvent,
  onDidRenameFiles: neverEvent,
  createFileSystemWatcher,
  openTextDocument(target) {
    const uri = target instanceof Uri ? target : typeof target === 'string' ? Uri.file(target) : null;
    if (!uri) return Promise.reject(new Error('untitled documents are not supported yet'));
    const doc = documents.get(uriKey(uri)) || loadDocument(uri);
    return doc ? Promise.resolve(doc) : Promise.reject(FileSystemError.FileNotFound(uri.fsPath));
  },
  applyEdit: applyWorkspaceEdit,
  saveAll: () => Promise.resolve(true),
  save: (uri) => Promise.resolve(uri),
  findFiles: () => Promise.resolve([]),
  registerTextDocumentContentProvider: disposableStub,
  registerFileSystemProvider: disposableStub,
  registerTaskProvider: disposableStub,
  registerNotebookSerializer: disposableStub,
  updateWorkspaceFolders: () => false,
});

const window = permissive('window', {
  get activeTextEditor() {
    return activeEditor();
  },
  get visibleTextEditors() {
    return [...documents.values()].map((d) => d._editor);
  },
  activeNotebookEditor: undefined,
  visibleNotebookEditors: [],
  terminals: [],
  activeTerminal: undefined,
  state: { focused: true, active: true },
  activeColorTheme: { kind: 2 },
  tabGroups: {
    get all() {
      return [tabGroup()];
    },
    get activeTabGroup() {
      return tabGroup();
    },
    onDidChangeTabs: events.tabs.event,
    onDidChangeTabGroups: neverEvent,
    close: () => Promise.resolve(false),
  },
  onDidChangeActiveTextEditor: events.activeEditor.event,
  onDidChangeVisibleTextEditors: events.visibleEditors.event,
  onDidChangeTextEditorSelection: neverEvent,
  onDidChangeTextEditorVisibleRanges: neverEvent,
  onDidChangeTextEditorOptions: neverEvent,
  onDidChangeTextEditorViewColumn: neverEvent,
  onDidChangeWindowState: neverEvent,
  onDidChangeActiveColorTheme: neverEvent,
  onDidChangeActiveNotebookEditor: neverEvent,
  onDidChangeVisibleNotebookEditors: neverEvent,
  onDidOpenTerminal: neverEvent,
  onDidCloseTerminal: neverEvent,
  onDidChangeActiveTerminal: neverEvent,
  showInformationMessage: showMessage('info'),
  showWarningMessage: showMessage('warn'),
  showErrorMessage: showMessage('error'),
  showQuickPick: () => Promise.resolve(undefined),
  showInputBox: () => Promise.resolve(undefined),
  showOpenDialog: () => Promise.resolve(undefined),
  showSaveDialog: () => Promise.resolve(undefined),
  showWorkspaceFolderPick: () => Promise.resolve(state.folders[0]),
  showTextDocument(target) {
    const uri = target instanceof Uri ? target : target && target.uri;
    if (uri) send({ type: 'reveal', path: appPath(uri) });
    const doc = uri && documents.get(uriKey(uri));
    return Promise.resolve(doc ? doc._editor : undefined);
  },
  createOutputChannel,
  createStatusBarItem,
  createTextEditorDecorationType,
  setStatusBarMessage: disposableStub,
  withProgress(options, task) {
    const source = new CancellationTokenSource();
    return Promise.resolve(task({ report() {} }, source.token));
  },
  createTerminal: () => permissive('Terminal', { name: 'terminal', processId: Promise.resolve(undefined), sendText() {}, show() {}, hide() {}, dispose() {} }),
  createWebviewPanel,
  registerWebviewViewProvider,
  registerTreeDataProvider: disposableStub,
  createTreeView: () => ({
    visible: false,
    selection: [],
    onDidExpandElement: neverEvent,
    onDidCollapseElement: neverEvent,
    onDidChangeSelection: neverEvent,
    onDidChangeVisibility: neverEvent,
    onDidChangeCheckboxState: neverEvent,
    reveal: () => Promise.resolve(),
    dispose() {},
  }),
  registerWebviewPanelSerializer: disposableStub,
  registerCustomEditorProvider: disposableStub,
  registerUriHandler: disposableStub,
  registerFileDecorationProvider: disposableStub,
  registerTerminalLinkProvider: disposableStub,
  registerTerminalProfileProvider: disposableStub,
});

const KIND = /^register(.+)Provider$/;
const languages = new Proxy(
  {
    createDiagnosticCollection,
    getDiagnostics(uri) {
      if (uri) return allDiagnostics(uri);
      const seen = new Map();
      for (const c of collections) for (const e of c._map.values()) seen.set(uriKey(e.uri), e.uri);
      return [...seen.values()].map((u) => [u, allDiagnostics(u)]);
    },
    onDidChangeDiagnostics: diagnosticsChanged.event,
    match: (selector, document) => scoreSelector(selector, document),
    getLanguages: () => Promise.resolve([...knownLanguages]),
    setTextDocumentLanguage: (doc) => Promise.resolve(doc),
    setLanguageConfiguration: disposableStub,
    createLanguageStatusItem(id, selector) {
      return { id, selector, name: undefined, text: '', detail: undefined, severity: 0, busy: false, command: undefined, accessibilityInformation: undefined, dispose() {} };
    },
  },
  {
    get(obj, prop) {
      if (prop in obj || typeof prop === 'symbol') return obj[prop];
      const m = KIND.exec(prop);
      if (m) {
        return (selector, provider, ...rest) => {
          const entry = { kind: m[1], selector, provider, rest, extension: currentId() || '', order: state.providers.length };
          state.providers.push(entry);
          return new Disposable(() => {
            const at = state.providers.indexOf(entry);
            if (at >= 0) state.providers.splice(at, 1);
          });
        };
      }
      unsupported(`languages.${prop}`);
      return () => undefined;
    },
  },
);

const commands = permissive('commands', {
  registerCommand(id, handler, thisArg) {
    state.commands.set(id, { handler, thisArg, extension: currentId() || '' });
    return new Disposable(() => state.commands.delete(id));
  },
  registerTextEditorCommand(id, handler, thisArg) {
    return commands.registerCommand(id, (...args) => {
      const editor = activeEditor();
      return editor ? handler.call(thisArg, editor, { replace() {}, insert() {}, delete() {} }, ...args) : undefined;
    });
  },
  executeCommand(id, ...args) {
    const entry = state.commands.get(id);
    if (entry) {
      try {
        return Promise.resolve(scope.run(entry.extension, () => entry.handler.apply(entry.thisArg, args)));
      } catch (e) {
        return Promise.reject(e);
      }
    }
    if (id === 'vscode.open' || id === 'revealInExplorer') {
      const target = args[0];
      const uri = target instanceof Uri ? target : typeof target === 'string' ? Uri.parse(target) : null;
      if (uri && uri.scheme === 'file') send({ type: 'reveal', path: appPath(uri) });
      else if (uri) send({ type: 'openExternal', url: uri.toString() });
      return Promise.resolve();
    }
    if (id === 'setContext') {
      const [key, value] = args;
      if (typeof key === 'string') send({ type: 'context', key, value: value === undefined ? null : value });
      return Promise.resolve();
    }
    return activateByEvent('onCommand:' + id).then(() => {
      const late = state.commands.get(id);
      if (late) return scope.run(late.extension, () => late.handler.apply(late.thisArg, args));
      unsupported(`command ${id}`);
      return undefined;
    });
  },
  getCommands: () => Promise.resolve([...state.commands.keys()]),
});

const env = permissive('env', {
  appName: 'Code Canvas',
  appRoot: __dirname,
  appHost: 'desktop',
  uriScheme: 'code-canvas',
  language: 'en',
  machineId: 'code-canvas',
  sessionId: 'session-' + Date.now(),
  shell: process.env.SHELL || process.env.ComSpec || '',
  uiKind: 1,
  remoteName: undefined,
  isNewAppInstall: false,
  isTelemetryEnabled: false,
  logLevel: 3,
  onDidChangeTelemetryEnabled: neverEvent,
  onDidChangeLogLevel: neverEvent,
  onDidChangeShell: neverEvent,
  clipboard: { readText: () => Promise.resolve(''), writeText: () => Promise.resolve() },
  openExternal(uri) {
    send({ type: 'openExternal', url: uri.toString() });
    return Promise.resolve(true);
  },
  asExternalUri: (uri) => Promise.resolve(uri),
  createTelemetryLogger: () => ({ logUsage() {}, logError() {}, dispose() {}, onDidChangeEnableStates: neverEvent, isUsageEnabled: false, isErrorsEnabled: false }),
});

function extensionFacade(ext) {
  return {
    id: ext.id,
    extensionUri: Uri.file(ext.dir),
    extensionPath: ext.dir,
    extensionKind: 2,
    packageJSON: ext.manifest,
    get isActive() {
      return ext.isActive;
    },
    get exports() {
      return ext.exports;
    },
    activate: () => activate(ext).then(() => ext.exports),
  };
}
const extensions = permissive('extensions', {
  get all() {
    return [...state.extensions.values()].map(extensionFacade);
  },
  getExtension(id) {
    const ext = state.extensions.get(String(id).toLowerCase());
    return ext ? extensionFacade(ext) : undefined;
  },
  onDidChange: neverEvent,
});

const l10n = {
  t(message, ...args) {
    if (message && typeof message === 'object') {
      args = Array.isArray(message.args) ? message.args : Object.values(message.args || {});
      message = message.message;
    }
    const values = args.length === 1 && isPlain(args[0]) ? args[0] : args;
    return String(message).replace(/\{(\w+)\}/g, (all, k) => (values[k] === undefined ? all : String(values[k])));
  },
  bundle: undefined,
  uri: undefined,
};

const api = {
  version: VSCODE_VERSION,
  ...classes,
  ...enums,
  workspace,
  window,
  languages,
  commands,
  env,
  extensions,
  l10n,
  tasks: permissive('tasks', { registerTaskProvider: disposableStub, taskExecutions: [], onDidStartTask: neverEvent, onDidEndTask: neverEvent, fetchTasks: () => Promise.resolve([]) }),
  debug: permissive('debug', { breakpoints: [], activeDebugSession: undefined, registerDebugConfigurationProvider: disposableStub, registerDebugAdapterDescriptorFactory: disposableStub, startDebugging: () => Promise.resolve(false) }),
  scm: permissive('scm', {}),
  notebooks: permissive('notebooks', {}),
  tests: permissive('tests', {}),
  comments: permissive('comments', {}),
  authentication: permissive('authentication', { getSession: () => Promise.resolve(undefined), onDidChangeSessions: neverEvent }),
  chat: permissive('chat', {}),
  lm: permissive('lm', { selectChatModels: () => Promise.resolve([]), onDidChangeChatModels: neverEvent, tools: [] }),
};

const vscode = new Proxy(api, {
  get(obj, prop) {
    if (prop in obj || typeof prop === 'symbol' || prop === 'then' || prop === '__esModule' || prop === 'default') return obj[prop];
    unsupported(String(prop));
    return /^[A-Z]/.test(prop) ? class {} : permissive(String(prop), {});
  },
});
globalThis.__codeCanvasVscode = vscode;

function installVscodeModule(dirs) {
  const names = Object.keys(api).filter((n) => /^[A-Za-z_$][\w$]*$/.test(n));
  const source = `'use strict';\nconst api = globalThis.__codeCanvasVscode;\nmodule.exports = api;\n${names.map((n) => `exports.${n} = api.${n};`).join('\n')}\n`;
  let entry = '';
  for (const dir of dirs) {
    try {
      const target = path.join(dir, 'node_modules', 'vscode');
      fs.mkdirSync(target, { recursive: true });
      fs.writeFileSync(path.join(target, 'index.js'), source);
      fs.writeFileSync(path.join(target, 'package.json'), JSON.stringify({ name: 'vscode', version: VSCODE_VERSION, main: 'index.js' }));
      entry = entry || path.join(target, 'index.js');
    } catch (e) {
      log('warn', 'host', `cannot write the vscode module in ${dir}: ${e.message}`);
    }
  }
  const resolve = Module._resolveFilename;
  Module._resolveFilename = function (request, ...rest) {
    return request === 'vscode' && entry ? entry : resolve.call(this, request, ...rest);
  };
}

function memento(file) {
  let data = {};
  try {
    data = JSON.parse(fs.readFileSync(file, 'utf8'));
  } catch {}
  return {
    keys: () => Object.keys(data),
    get: (key, fallback) => (key in data ? data[key] : fallback),
    update(key, value) {
      if (value === undefined) delete data[key];
      else data[key] = value;
      try {
        fs.mkdirSync(path.dirname(file), { recursive: true });
        fs.writeFileSync(file, JSON.stringify(data));
      } catch (e) {
        log('warn', 'host', `state not saved: ${e.message}`);
      }
      return Promise.resolve();
    },
    setKeysForSync() {},
  };
}

function registerContributions(ext) {
  const c = ext.manifest.contributes || {};
  for (const lang of Array.isArray(c.languages) ? c.languages : []) {
    if (!lang || !lang.id) continue;
    knownLanguages.add(lang.id);
    for (const e of lang.extensions || []) byExtension.set(String(e).toLowerCase(), lang.id);
    for (const f of lang.filenames || []) byFilename.set(f, lang.id);
  }
  const sections = Array.isArray(c.configuration) ? c.configuration : c.configuration ? [c.configuration] : [];
  for (const section of sections) {
    for (const [key, schema] of Object.entries((section && section.properties) || {})) {
      if (schema && schema.default !== undefined) config.defaults[key] = schema.default;
      else if (schema && !(key in config.defaults)) {
        const t = Array.isArray(schema.type) ? schema.type[0] : schema.type;
        const zero = { boolean: false, number: 0, integer: 0, string: '', array: [], object: {} }[t];
        if (zero !== undefined && t !== 'string') config.defaults[key] = zero;
      }
    }
  }
}

function report(ext, status, error) {
  send({ type: 'extension', id: ext.id, status, error: error ? String(error.stack || error.message || error) : '', ms: ext.ms || 0 });
}

function activate(ext) {
  if (ext.activation) return ext.activation;
  ext.activation = scope.run(ext.id, async () => {
    const started = Date.now();
    report(ext, 'activating');
    try {
      const m = ext.manifest;
      let file = path.join(ext.dir, m.main);
      if (!fs.existsSync(file) && fs.existsSync(file + '.js')) file += '.js';
      for (const dep of m.extensionDependencies || []) {
        const other = state.extensions.get(String(dep).toLowerCase());
        if (other) await activate(other);
      }
      const storage = path.join(state.storageDir, ext.id);
      const esm = /\.mjs$/.test(file) || (m.type === 'module' && !/\.cjs$/.test(file));
      ext.module = esm ? await import(pathToFileURL(file).href) : require(file);
      const context = {
        subscriptions: [],
        extensionPath: ext.dir,
        extensionUri: Uri.file(ext.dir),
        extensionMode: 1,
        extension: extensionFacade(ext),
        globalState: memento(path.join(storage, 'global.json')),
        workspaceState: memento(path.join(storage, 'workspace.json')),
        secrets: { get: () => Promise.resolve(undefined), store: () => Promise.resolve(), delete: () => Promise.resolve(), onDidChange: neverEvent },
        storageUri: Uri.file(path.join(storage, 'workspace')),
        globalStorageUri: Uri.file(path.join(storage, 'global')),
        logUri: Uri.file(path.join(storage, 'log')),
        storagePath: path.join(storage, 'workspace'),
        globalStoragePath: path.join(storage, 'global'),
        logPath: path.join(storage, 'log'),
        asAbsolutePath: (rel) => path.join(ext.dir, rel),
        environmentVariableCollection: permissive('environmentVariableCollection', { persistent: false, replace() {}, append() {}, prepend() {}, get() {}, forEach() {}, delete() {}, clear() {}, getScoped: () => context.environmentVariableCollection }),
        languageModelAccessInformation: { onDidChange: neverEvent, canSendRequest: () => false },
      };
      ext.context = context;
      const entry = ext.module.activate || (ext.module.default && ext.module.default.activate);
      ext.exports = typeof entry === 'function' ? await entry(context) : undefined;
      ext.isActive = true;
      ext.ms = Date.now() - started;
      report(ext, 'active');
    } catch (e) {
      ext.ms = Date.now() - started;
      log('error', ext.id, 'activation failed: ' + inspect(e));
      report(ext, 'failed', e);
    }
  });
  return ext.activation;
}

function wantsEvent(ext, event) {
  const list = ext.manifest.activationEvents;
  if (!Array.isArray(list) || list.length === 0) return event === 'startup';
  if (event === 'startup') {
    return list.some((e) => {
      if (e === '*' || e === 'onStartupFinished') return true;
      if (!e.startsWith('workspaceContains:')) return false;
      const pattern = e.slice('workspaceContains:'.length);
      if (/[*?{]/.test(pattern)) return true;
      return state.folders.some((f) => fs.existsSync(path.join(f.uri.fsPath, pattern)));
    });
  }
  return list.includes(event);
}
function activateForView(view) {
  const pending = [activateByEvent('onView:' + view)];
  for (const ext of state.extensions.values()) {
    const views = (ext.manifest.contributes && ext.manifest.contributes.views) || {};
    const mine = Object.values(views).some((list) => Array.isArray(list) && list.some((v) => v && v.id === view));
    if (mine && ext.manifest.main) pending.push(ext.activation || activate(ext));
  }
  return Promise.all(pending);
}
function activateByEvent(event) {
  const pending = [];
  for (const ext of state.extensions.values()) if (ext.manifest.main && !ext.activation && wantsEvent(ext, event)) pending.push(activate(ext));
  return Promise.all(pending);
}

function providersFor(kind, doc) {
  return state.providers
    .map((p) => ({ p, score: scoreSelector(p.selector, doc) }))
    .filter((x) => x.p.kind === kind && x.score > 0)
    .sort((a, b) => b.score - a.score || b.p.order - a.p.order)
    .map((x) => x.p);
}

const plainRange = (r) => ({ line: r.start.line, character: r.start.character, endLine: r.end.line, endCharacter: r.end.character });
function plainLocations(result) {
  const list = Array.isArray(result) ? result : result ? [result] : [];
  return list
    .map((l) => (l.targetUri ? { uri: l.targetUri, range: l.targetSelectionRange || l.targetRange } : l))
    .filter((l) => l && l.uri && l.range)
    .map((l) => {
      const doc = documents.get(uriKey(l.uri)) || loadDocument(l.uri, true);
      const col = (p) => (doc ? displayColumn(doc.lineAt(p.line).text, p.character, state.tabWidth) : p.character);
      return { path: appPath(l.uri), ...plainRange(l.range), col: col(l.range.start), endCol: col(l.range.end) };
    });
}
function plainHover(h) {
  if (!h) return '';
  const parts = Array.isArray(h.contents) ? h.contents : [h.contents];
  return parts.map((c) => (typeof c === 'string' ? c : c && c.language ? '```' + c.language + '\n' + c.value + '\n```' : (c && c.value) || '')).join('\n\n');
}

const requests = {
  async format({ path: file, onSave }) {
    const doc = openDocument(file, true);
    if (!doc) throw new Error('cannot read the file');
    const cfg = getConfiguration('editor', doc);
    if (onSave && !cfg.get('formatOnSave', true)) return { changed: false, skipped: true };
    const candidates = providersFor('DocumentFormattingEdit', doc);
    if (candidates.length === 0) {
      if (onSave) return { changed: false, skipped: true };
      throw new Error(`no installed extension formats ${doc.languageId} files`);
    }
    const wanted = String(cfg.get('defaultFormatter') || '').toLowerCase();
    const chosen = candidates.find((p) => p.extension === wanted) || candidates[0];
    const options = { tabSize: cfg.get('tabSize', 4), insertSpaces: cfg.get('insertSpaces', true) };
    const edits = await scope.run(chosen.extension, () => chosen.provider.provideDocumentFormattingEdits(doc, options, noToken));
    if (!edits || edits.length === 0) return { changed: false, extension: chosen.extension };
    const text = applyEdits(doc, edits);
    return { changed: text !== doc.getText(), text, extension: chosen.extension };
  },
  async locations({ path: file, line, col, kind }) {
    const doc = openDocument(file, false);
    if (!doc) return { items: [] };
    const position = positionAt(doc, line, col);
    const method = { Definition: 'provideDefinition', Reference: 'provideReferences', Implementation: 'provideImplementation', TypeDefinition: 'provideTypeDefinition', Declaration: 'provideDeclaration' }[kind];
    for (const p of providersFor(kind, doc)) {
      const args = kind === 'Reference' ? [doc, position, { includeDeclaration: true }, noToken] : [doc, position, noToken];
      const items = plainLocations(await scope.run(p.extension, () => p.provider[method](...args)));
      if (items.length) return { items, extension: p.extension };
    }
    return { items: [] };
  },
  async hover({ path: file, line, col }) {
    const doc = openDocument(file, false);
    if (!doc) return { text: '' };
    const position = positionAt(doc, line, col);
    for (const p of providersFor('Hover', doc)) {
      const text = plainHover(await scope.run(p.extension, () => p.provider.provideHover(doc, position, noToken)));
      if (text) return { text, extension: p.extension };
    }
    return { text: '' };
  },
  async resolveWebview({ view }) {
    if (!webviewProviders.has(view)) await activateForView(view);
    const entry = webviewProviders.get(view);
    if (!entry) throw new Error(`no extension provides the view ${view}`);
    const w = createWebview(entry.extension, {});
    const visibility = new EventEmitter();
    const viewObject = {
      viewType: view,
      webview: w.webview,
      title: undefined,
      description: undefined,
      badge: undefined,
      visible: true,
      onDidChangeVisibility: visibility.event,
      onDidDispose: w.closed.event,
      show() {},
    };
    await scope.run(entry.extension, () => entry.provider.resolveWebviewView(viewObject, { state: undefined }, noToken));
    return { handle: w.handle };
  },
  async command({ command, args }) {
    await commands.executeCommand(command, ...(args || []));
    return { ok: true };
  },
  async describe() {
    const kinds = {};
    for (const p of state.providers) (kinds[p.kind] = kinds[p.kind] || []).push(p.extension);
    return {
      providers: kinds,
      commands: [...state.commands.keys()],
      documents: [...documents.values()].map((d) => d.fileName),
      webviews: [...webviewProviders.keys()],
    };
  },
};

async function handle(msg) {
  switch (msg.type) {
    case 'init': {
      state.tabWidth = msg.tabWidth || 4;
      state.storageDir = msg.storageDir;
      config.userFile = msg.settingsFile || '';
      config.user = config.userFile ? readSettings(config.userFile) : {};
      config.imported = msg.vscodeSettingsFile ? readSettings(msg.vscodeSettingsFile) : {};
      setFolders(msg.folders || []);
      installVscodeModule([...new Set((msg.extensions || []).map((e) => path.dirname(e.dir)))]);
      for (const e of msg.extensions || []) {
        try {
          const manifest = JSON.parse(fs.readFileSync(path.join(e.dir, 'package.json'), 'utf8'));
          const ext = { id: e.id.toLowerCase(), dir: e.dir, manifest, isActive: false, exports: undefined, activation: null };
          state.extensions.set(ext.id, ext);
          registerContributions(ext);
          if (!manifest.main) report(ext, 'declarative');
        } catch (err) {
          send({ type: 'extension', id: e.id, status: 'failed', error: 'package.json: ' + err.message, ms: 0 });
        }
      }
      await activateByEvent('startup');
      send({ type: 'ready', node: process.version });
      break;
    }
    case 'folders':
      setFolders(msg.folders || []);
      break;
    case 'open':
      openDocument(msg.path, !!msg.focus);
      break;
    case 'close':
      closeDocument(msg.path);
      break;
    case 'changed':
      reloadDocument(msg.path);
      break;
    case 'edit':
      editDocument(msg.path, msg.text);
      break;
    case 'messageReply': {
      const pending = pendingMessages.get(msg.id);
      if (pending) {
        pendingMessages.delete(msg.id);
        pending.resolve(pending.items.find((i) => itemTitle(i) === msg.action));
      }
      break;
    }
    case 'webviewMessage': {
      const w = webviews.get(msg.handle);
      if (w) w.received.fire(msg.message);
      break;
    }
    case 'webviewClosed': {
      const w = webviews.get(msg.handle);
      if (w && webviews.delete(msg.handle)) w.closed.fire();
      break;
    }
    case 'request': {
      const fn = requests[msg.method];
      try {
        if (!fn) throw new Error(`unknown request ${msg.method}`);
        send({ type: 'reply', id: msg.id, result: await fn(msg.params || {}) });
      } catch (e) {
        send({ type: 'reply', id: msg.id, error: String((e && e.message) || e) });
      }
      break;
    }
    case 'shutdown':
      await shutdown();
      break;
    default:
      log('warn', 'host', `unknown message ${msg.type}`);
  }
}

function setFolders(list) {
  const before = state.folders;
  state.folders = list.map((p, index) => ({ uri: Uri.file(p), name: path.basename(p), index }));
  config.folders.clear();
  for (const f of state.folders) config.folders.set(f.uri.fsPath, readSettings(path.join(f.uri.fsPath, '.vscode', 'settings.json')));
  const key = (f) => fileKey(f.uri.fsPath);
  const added = state.folders.filter((f) => !before.some((b) => key(b) === key(f)));
  const removed = before.filter((b) => !state.folders.some((f) => key(b) === key(f)));
  if (before.length && (added.length || removed.length)) events.folders.fire({ added, removed });
}

async function shutdown() {
  const stops = [];
  for (const ext of state.extensions.values()) {
    if (!ext.isActive) continue;
    stops.push(
      scope.run(ext.id, async () => {
        try {
          const stop = ext.module.deactivate || (ext.module.default && ext.module.default.deactivate);
          if (typeof stop === 'function') await stop();
          for (const d of ext.context.subscriptions) if (d && typeof d.dispose === 'function') await d.dispose();
        } catch (e) {
          log('warn', ext.id, 'deactivate failed: ' + inspect(e));
        }
      }),
    );
  }
  await Promise.race([Promise.all(stops), new Promise((r) => setTimeout(r, 2500))]);
  process.exit(0);
}

let queue = Promise.resolve();
let buffered = '';
process.stdin.setEncoding('utf8');
process.stdin.on('data', (chunk) => {
  buffered += chunk;
  for (let at; (at = buffered.indexOf('\n')) >= 0; ) {
    const line = buffered.slice(0, at).trim();
    buffered = buffered.slice(at + 1);
    if (!line) continue;
    let msg;
    try {
      msg = JSON.parse(line);
    } catch (e) {
      log('error', 'host', 'bad message: ' + e.message);
      continue;
    }
    const run = () => handle(msg).catch((e) => log('error', 'host', `${msg.type} failed: ${inspect(e)}`));
    if (msg.type === 'request') queue.then(run);
    else queue = queue.then(run);
  }
});
process.stdin.on('end', () => shutdown());

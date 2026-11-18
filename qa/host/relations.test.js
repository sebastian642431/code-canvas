// The host asks language servers for the relations of a selected name: the calls it makes
// (call hierarchy) and what implements it, with a deadline past which the request is
// cancelled and the app keeps its guesses. A fake extension plays the language server.
const fs = require('fs');
const os = require('os');
const path = require('path');
const assert = require('assert');
const { spawn } = require('child_process');

const hostFile = path.join(__dirname, '..', '..', 'crates', 'code-canvas', 'exthost', 'host.js');
const work = fs.mkdtempSync(path.join(os.tmpdir(), 'cc-host-relations-'));
const folder = path.join(work, 'project');
const extDir = path.join(work, 'extensions', 'qa.fake-relations');
fs.mkdirSync(path.join(folder, 'src'), { recursive: true });
fs.mkdirSync(extDir, { recursive: true });
const main = path.join(folder, 'src', 'main.rs');
const util = path.join(folder, 'src', 'util.rs');
fs.writeFileSync(main, 'fn main() {\n\thelper(); quiet(); slow();\n}\n\nfn quiet() {}\n\nfn slow() {}\n');
fs.writeFileSync(util, 'pub trait Shape {}\n\nimpl Shape for Square {}\n\npub fn helper() {}\n');

fs.writeFileSync(path.join(extDir, 'package.json'), JSON.stringify({ name: 'fake-relations', publisher: 'qa', version: '1.0.0', main: './extension.js', activationEvents: ['*'], engines: { vscode: '^1.80.0' } }));
fs.writeFileSync(
  path.join(extDir, 'extension.js'),
  `const vscode = require('vscode');
const util = vscode.Uri.file(${JSON.stringify(util)});
const word = (doc, pos) => doc.getText(doc.getWordRangeAtPosition(pos));
const whenCancelled = (token, what) => new Promise((resolve) => token.onCancellationRequested(() => { console.log('cancelled ' + what); resolve(undefined); }));
exports.activate = () => {
  const sel = { scheme: 'file' };
  vscode.languages.registerCallHierarchyProvider(sel, {
    prepareCallHierarchy(doc, pos, token) {
      const name = word(doc, pos);
      if (name === 'slow') return whenCancelled(token, 'prepare');
      if (name !== 'main' && name !== 'quiet') return undefined;
      const r = doc.getWordRangeAtPosition(pos);
      return new vscode.CallHierarchyItem(vscode.SymbolKind.Function, name, '', doc.uri, r, r);
    },
    provideCallHierarchyOutgoingCalls(item) {
      if (item.name !== 'main') return [];
      const helper = new vscode.Range(4, 7, 4, 13);
      const callee = new vscode.CallHierarchyItem(vscode.SymbolKind.Function, 'helper', '', util, helper, helper);
      const quiet = new vscode.CallHierarchyItem(vscode.SymbolKind.Function, 'quiet', '', item.uri, new vscode.Range(4, 3, 4, 8), new vscode.Range(4, 3, 4, 8));
      return [
        new vscode.CallHierarchyOutgoingCall(callee, [new vscode.Range(1, 1, 1, 7)]),
        new vscode.CallHierarchyOutgoingCall(quiet, [new vscode.Range(1, 11, 1, 16)]),
      ];
    },
  });
  vscode.languages.registerImplementationProvider(sel, {
    provideImplementation(doc, pos, token) {
      if (word(doc, pos) === 'slow') return whenCancelled(token, 'implementation');
      return [new vscode.Location(util, new vscode.Range(2, 15, 2, 21))];
    },
  });
  vscode.languages.registerReferenceProvider(sel, {
    provideReferences(doc, pos, context, token) {
      return whenCancelled(token, 'references');
    },
  });
};
`,
);

const child = spawn(process.execPath, [hostFile], { stdio: ['pipe', 'pipe', 'inherit'] });
const waiting = new Map();
const logs = [];
let ready;
const isReady = new Promise((resolve) => (ready = resolve));
let buffer = '';
child.stdout.setEncoding('utf8');
child.stdout.on('data', (chunk) => {
  buffer += chunk;
  let at;
  while ((at = buffer.indexOf('\n')) >= 0) {
    const line = buffer.slice(0, at);
    buffer = buffer.slice(at + 1);
    if (!line.trim()) continue;
    const msg = JSON.parse(line);
    if (msg.type === 'ready') ready();
    if (msg.type === 'log') logs.push(msg.text);
    if (msg.type === 'reply' && waiting.has(msg.id)) {
      waiting.get(msg.id)(msg);
      waiting.delete(msg.id);
    }
  }
});
const send = (msg) => child.stdin.write(JSON.stringify(msg) + '\n');
let next = 1;
const ask = (method, params) =>
  new Promise((resolve) => {
    const id = next++;
    waiting.set(id, resolve);
    send({ type: 'request', id, method, params });
  });

const finish = (code) => {
  try {
    child.kill();
  } catch (_) {}
  fs.rmSync(work, { recursive: true, force: true });
  process.exit(code);
};
const timer = setTimeout(() => {
  console.log('FAIL relations: the host did not answer in time');
  finish(1);
}, 15000);

(async () => {
  send({ type: 'init', folders: [folder], extensions: [{ id: 'qa.fake-relations', dir: extDir }], tabWidth: 4, storageDir: path.join(work, 'storage') });
  await isReady;
  send({ type: 'open', path: main });

  const calls = await ask('outgoingCalls', { path: main, line: 0, col: 3, deadline: 2000 });
  assert.ok(!calls.error, calls.error);
  assert.strictEqual(calls.result.known, true);
  assert.deepStrictEqual(
    calls.result.items.map((c) => [path.basename(c.path), c.line, c.col, c.name, path.basename(c.fromPath), c.fromLine, c.fromCol]),
    [
      ['util.rs', 4, 7, 'helper', 'main.rs', 1, 4],
      ['main.rs', 4, 3, 'quiet', 'main.rs', 1, 14],
    ],
    'callee where it is named, call where it is made, in display columns (the tab is four)',
  );

  const none = await ask('outgoingCalls', { path: main, line: 4, col: 3, deadline: 2000 });
  assert.deepStrictEqual(none.result, { items: [], known: true, extension: 'qa.fake-relations' }, 'a function the server knows that calls nothing');

  const unknown = await ask('outgoingCalls', { path: main, line: 1, col: 1, deadline: 2000 });
  assert.deepStrictEqual(unknown.result, { items: [], known: false }, 'a name the server does not prepare is no answer');

  const started = Date.now();
  const slow = await ask('outgoingCalls', { path: main, line: 6, col: 3, deadline: 300 });
  const took = Date.now() - started;
  assert.deepStrictEqual(slow.result, { items: [], known: false }, 'past the deadline there is no answer');
  assert.ok(took >= 250 && took < 3000, `the deadline is kept: ${took} ms`);

  const impls = await ask('locations', { path: main, line: 0, col: 3, kind: 'Implementation', deadline: 2000 });
  assert.deepStrictEqual(
    impls.result.items.map((l) => [path.basename(l.path), l.line, l.col]),
    [['util.rs', 2, 15]],
  );
  const slowImpls = await ask('locations', { path: main, line: 6, col: 3, kind: 'Implementation', deadline: 300 });
  assert.deepStrictEqual(slowImpls.result, { items: [] });
  const slowUses = await ask('locations', { path: main, line: 0, col: 3, kind: 'Reference', deadline: 300 });
  assert.deepStrictEqual(slowUses.result, { items: [] }, 'a reference provider that does not answer in time');

  await new Promise((r) => setTimeout(r, 100));
  for (const what of ['prepare', 'implementation', 'references']) {
    assert.ok(logs.some((l) => l.includes('cancelled ' + what)), `the ${what} request is cancelled at the deadline: ${logs.join(' | ')}`);
  }
  clearTimeout(timer);
  console.log('PASS relations (7)');
  finish(0);
})().catch((e) => {
  clearTimeout(timer);
  console.log('FAIL relations:', e.message);
  finish(1);
});

// Code lenses and tree views through the real host, with qa/fake/views-extension: the lenses
// of a file (one resolved later), running one with its arguments, "N references" asking the
// app to show the uses, lenses that change; the rows of a tree, their children, a click, a
// row's button, the title's button and the tree telling it changed.
const fs = require('fs');
const os = require('os');
const path = require('path');
const assert = require('assert');
const { spawn } = require('child_process');

const hostFile = path.join(__dirname, '..', '..', 'crates', 'code-canvas', 'exthost', 'host.js');
const work = fs.mkdtempSync(path.join(os.tmpdir(), 'cc-host-views-'));
// A copy: the host puts a `vscode` module beside the extensions folder it is given.
const extDir = path.join(work, 'extensions', 'qa.fake-views');
fs.cpSync(path.join(__dirname, '..', 'fake', 'views-extension'), extDir, { recursive: true });
const folder = path.join(work, 'project');
fs.mkdirSync(path.join(folder, 'src'), { recursive: true });
const main = path.join(folder, 'src', 'main.rs');
fs.writeFileSync(main, 'mod util;\n\nfn main() {\n\thelper();\n}\n\n\tfn helper() {}\n');
const ranFile = path.join(folder, 'qa-ran.txt');
const ran = () => (fs.existsSync(ranFile) ? fs.readFileSync(ranFile, 'utf8').trim().split('\n') : []);

const child = spawn(process.execPath, [hostFile], { stdio: ['pipe', 'pipe', 'inherit'] });
const waiting = new Map();
const seen = [];
const watchers = [];
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
    seen.push(msg);
    for (const w of watchers.splice(0)) if (!w(msg)) watchers.push(w);
    if (msg.type === 'reply' && waiting.has(msg.id)) {
      waiting.get(msg.id)(msg);
      waiting.delete(msg.id);
    }
  }
});
// The next message of that type (and for which `test` holds), or one already seen after `from`.
const message = (type, from, test = () => true) =>
  new Promise((resolve) => {
    const found = seen.slice(from).find((m) => m.type === type && test(m));
    if (found) return resolve(found);
    watchers.push((m) => m.type === type && test(m) && (resolve(m), true));
  });
const send = (msg) => child.stdin.write(JSON.stringify(msg) + '\n');
let next = 1;
const ask = (method, params) =>
  new Promise((resolve) => {
    const id = next++;
    waiting.set(id, (msg) => {
      assert.ok(!msg.error, `${method}: ${msg.error}`);
      resolve(msg.result);
    });
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
  console.log('FAIL views: the host did not answer in time');
  finish(1);
}, 20000);
let checks = 0;
const check = (fn) => {
  fn();
  checks++;
};

(async () => {
  send({ type: 'init', folders: [folder], extensions: [{ id: 'qa.fake-views', dir: extDir }], tabWidth: 4, storageDir: path.join(work, 'storage') });
  await message('ready', 0);

  // A tree asked for before its extension started starts it.
  const notes = await ask('treeChildren', { view: 'qa.notes' });
  check(() => assert.deepStrictEqual(notes.items.map((i) => [i.label, i.icon, i.collapsible]), [['first note', 'note', 0], ['second note', 'note', 0]]));
  check(() => assert.ok(seen.some((m) => m.type === 'treeView' && m.view === 'qa.items' && m.extension === 'qa.fake-views'), 'the trees are announced'));

  const roots = await ask('treeChildren', { view: 'qa.items' });
  check(() =>
    assert.deepStrictEqual(
      roots.items.map((i) => [i.handle, i.label, i.description, i.icon, i.collapsible, i.context, i.command]),
      [
        ['/Fruits', 'Fruits', '3 kinds', 'symbol-namespace', 1, 'group', false],
        ['/Vegetables', 'Vegetables', '2 kinds', 'symbol-folder', 2, 'group', false],
        ['/Read me', 'Read me', '', 'book', 0, '', true],
      ],
    ),
  );
  const fruits = await ask('treeChildren', { view: 'qa.items', parent: '/Fruits' });
  check(() => assert.deepStrictEqual(fruits.items.map((i) => [i.handle, i.label, i.context]), [['/Fruits/apple', 'apple', 'fruit'], ['/Fruits/banana', 'banana', 'fruit'], ['/Fruits/cherry', 'cherry', 'fruit']]));

  await ask('treeClick', { view: 'qa.items', handle: '/Fruits/banana' });
  await ask('treeExpand', { view: 'qa.items', handle: '/Fruits', expanded: true });
  const before = seen.length;
  await ask('treeCommand', { view: 'qa.items', handle: '/Fruits/apple', command: 'qa.items.remove' });
  await message('treeChanged', before, (m) => m.view === 'qa.items');
  const after = await ask('treeChildren', { view: 'qa.items', parent: '/Fruits' });
  check(() => assert.deepStrictEqual(after.items.map((i) => i.label), ['banana', 'cherry'], 'the removed row is gone once asked again'));
  await ask('treeCommand', { view: 'qa.items', command: 'qa.items.add' });
  const gone = await ask('treeChildren', { view: 'qa.items', parent: '/Nothing' });
  check(() => assert.strictEqual(gone.gone, true, 'a row the host does not know'));
  const missing = await ask('treeChildren', { view: 'qa.nobody' });
  check(() => assert.strictEqual(missing.missing, true, 'a view nobody provides'));
  check(() => assert.deepStrictEqual(ran(), ['open banana', 'expanded Fruits', 'remove apple', 'add damson']));

  // Lenses. The provider is registered when the Rust file opens, which the app hears.
  const opened = seen.length;
  send({ type: 'open', path: main });
  await message('lensesChanged', opened);
  const lenses = await ask('codeLenses', { path: main });
  check(() =>
    assert.deepStrictEqual(
      lenses.items.map((l) => [l.line, l.col, l.title, l.runs]),
      [
        [2, 0, '▶ Run', true],
        [2, 0, '1 reference', true],
        [6, 4, '▶ Run', true],
        [6, 4, '2 references', true],
      ],
      'one lens resolved later; columns are display columns (the tab is four)',
    ),
  );
  await ask('runLens', { id: lenses.items[0].id });
  check(() => assert.strictEqual(ran().pop(), 'run main 2', 'the lens runs its command with its arguments'));
  const asked = seen.length;
  await ask('runLens', { id: lenses.items[3].id });
  const uses = await message('showUses', asked);
  check(() => assert.deepStrictEqual([path.basename(uses.path), uses.line, uses.col], ['main.rs', 6, 7], '"references" asks the app to show the uses'));

  const first = ask('codeLenses', { path: main, wait: 400 });
  const second = ask('codeLenses', { path: main });
  const [a, b] = await Promise.all([first, second]);
  check(() => assert.strictEqual(a.superseded, true, 'an older request of the same file gives way'));
  check(() => assert.strictEqual(b.items.length, 4));
  const stale = await new Promise((resolve) => {
    const id = next++;
    waiting.set(id, resolve);
    send({ type: 'request', id, method: 'runLens', params: { id: lenses.items[0].id } });
  });
  check(() => assert.ok(stale.error, 'a lens of an older answer is gone'));

  const changing = seen.length;
  await ask('command', { command: 'qa.lenses.more' });
  await message('lensesChanged', changing);
  const more = await ask('codeLenses', { path: main });
  check(() => assert.deepStrictEqual(more.items.filter((l) => l.title === 'more').map((l) => l.line), [2, 6]));

  clearTimeout(timer);
  console.log(`PASS views (${checks})`);
  finish(0);
})().catch((e) => {
  clearTimeout(timer);
  console.log('FAIL views:', e.message);
  finish(1);
});

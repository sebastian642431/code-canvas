// A folder that is not trusted: extensions see workspace.isTrusted false, and the settings
// that would make them run what the folder brings are not read from its .vscode/settings.json
// (flat, nested or inside a language block). Writing a setting to the folder keeps those
// already in the file. A trusted folder reads them all.
const fs = require('fs');
const os = require('os');
const path = require('path');
const assert = require('assert');
const { spawn } = require('child_process');

const hostFile = path.join(__dirname, '..', '..', 'crates', 'code-canvas', 'exthost', 'host.js');
const work = fs.mkdtempSync(path.join(os.tmpdir(), 'cc-host-trust-'));
const folder = path.join(work, 'project');
const extDir = path.join(work, 'extensions', 'qa.fake-trust');
const settingsFile = path.join(folder, '.vscode', 'settings.json');
fs.mkdirSync(path.dirname(settingsFile), { recursive: true });
fs.mkdirSync(extDir, { recursive: true });
fs.writeFileSync(
  settingsFile,
  JSON.stringify({
    'typescript.tsdk': 'evil/tsdk',
    'eslint.nodePath': 'evil/node',
    prettier: { prettierPath: 'evil/prettier', semi: false },
    '[typescript]': { 'typescript.tsserver.pluginPaths': ['evil'], 'editor.tabSize': 3 },
    'editor.tabSize': 7,
  }),
);
fs.writeFileSync(path.join(extDir, 'package.json'), JSON.stringify({ name: 'fake-trust', publisher: 'qa', version: '1.0.0', main: './extension.js', activationEvents: ['*'], engines: { vscode: '^1.80.0' } }));
fs.writeFileSync(
  path.join(extDir, 'extension.js'),
  `const vscode = require('vscode');
const report = () => {
  const c = (s) => vscode.workspace.getConfiguration(s);
  const file = vscode.Uri.joinPath(vscode.workspace.workspaceFolders[0].uri, 'a.ts');
  const ts = vscode.workspace.getConfiguration('typescript', { uri: file, languageId: 'typescript' });
  console.log('trust ' + JSON.stringify({
    trusted: vscode.workspace.isTrusted,
    tsdk: c('typescript').get('tsdk'),
    eslint: c('eslint').get('nodePath'),
    prettierPath: c('prettier').get('prettierPath'),
    semi: c('prettier').get('semi'),
    plugins: ts.get('tsserver.pluginPaths'),
    tab: c('editor').get('tabSize'),
  }));
};
exports.activate = () => {
  report();
  vscode.commands.registerCommand('qa.trust.write', async () => {
    await vscode.workspace.getConfiguration('qa').update('written', 1, false);
    report();
  });
};
`,
);

function run(trusted) {
  const child = spawn(process.execPath, [hostFile], { stdio: ['pipe', 'pipe', 'inherit'] });
  const logs = [];
  const waiting = new Map();
  let buffer = '';
  let ready;
  const isReady = new Promise((resolve) => (ready = resolve));
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
      if (msg.type === 'log' && msg.text.startsWith('trust ')) logs.push(JSON.parse(msg.text.slice(6)));
      if (msg.type === 'reply' && waiting.has(msg.id)) {
        waiting.get(msg.id)(msg);
        waiting.delete(msg.id);
      }
    }
  });
  const send = (msg) => child.stdin.write(JSON.stringify(msg) + '\n');
  const ask = (id, method, params) =>
    new Promise((resolve) => {
      waiting.set(id, resolve);
      send({ type: 'request', id, method, params });
    });
  const restricted = trusted ? [] : ['typescript.tsdk', 'typescript.tsserver.pluginPaths', 'eslint', 'prettier.prettierPath'];
  send({ type: 'init', folders: [folder], extensions: [{ id: 'qa.fake-trust', dir: extDir }], tabWidth: 4, storageDir: path.join(work, 'storage'), trusted, restricted });
  return { child, logs, isReady, ask };
}

const finish = (code, children) => {
  for (const c of children) {
    try {
      c.kill();
    } catch (_) {}
  }
  fs.rmSync(work, { recursive: true, force: true });
  process.exit(code);
};

const children = [];
const timer = setTimeout(() => {
  console.log('FAIL trust: the host did not answer in time');
  finish(1, children);
}, 15000);

(async () => {
  const untrusted = run(false);
  children.push(untrusted.child);
  await untrusted.isReady;
  assert.deepStrictEqual(untrusted.logs[0], { trusted: false, semi: false, tab: 7 }, 'nothing restricted is read');
  const wrote = await untrusted.ask(1, 'command', { command: 'qa.trust.write' });
  assert.ok(!wrote.error, wrote.error);
  const kept = JSON.parse(fs.readFileSync(settingsFile, 'utf8'));
  assert.strictEqual(kept['typescript.tsdk'], 'evil/tsdk', 'a write keeps what is in the file');
  assert.strictEqual(kept['qa.written'], 1);
  assert.strictEqual(untrusted.logs[1].tsdk, undefined, 'and still does not read it');
  untrusted.child.kill();

  const trusted = run(true);
  children.push(trusted.child);
  await trusted.isReady;
  const seen = trusted.logs[0];
  assert.strictEqual(seen.trusted, true);
  assert.strictEqual(seen.tsdk, 'evil/tsdk');
  assert.strictEqual(seen.eslint, 'evil/node');
  assert.strictEqual(seen.prettierPath, 'evil/prettier');
  assert.deepStrictEqual(seen.plugins, ['evil']);
  clearTimeout(timer);
  console.log('PASS trust (2)');
  finish(0, children);
})().catch((e) => {
  clearTimeout(timer);
  console.log('FAIL trust:', e.message);
  finish(1, children);
});

// Webview views for the scenarios that move a page between the canvas, the side panels and the console's tabs.
//
// Each page counts its clicks and keeps a text in memory only: a page that is loaded again
// starts from nothing. Every page says when it has loaded, and `window.qa.report()` (run from
// a scenario with the `page` step) says what it holds. Both are written to qa-pages.txt in the
// first folder, so a scenario can tell a page that kept its state from one that was reloaded.
const fs = require('fs');
const path = require('path');
const vscode = require('vscode');

function said(line) {
  const folder = vscode.workspace.workspaceFolders && vscode.workspace.workspaceFolders[0];
  if (!folder) return;
  fs.appendFileSync(path.join(folder.uri.fsPath, 'qa-pages.txt'), line + '\n');
  console.log('qa page: ' + line);
}

function page(name, colour) {
  return `<!DOCTYPE html>
<html><head><meta charset="utf-8"><style>
body { font-family: sans-serif; padding: 16px; color: var(--vscode-foreground); }
h1 { font-size: 18px; margin: 0 0 12px; }
#count { font-size: 64px; font-weight: bold; color: ${colour}; }
button { font-size: 14px; padding: 6px 14px; }
#text { margin-top: 14px; font-size: 15px; }
</style></head><body>
<h1>${name}</h1>
<div>clicks: <span id="count">0</span></div>
<button id="more">One more</button>
<div id="text">(nothing typed)</div>
<script>
const api = acquireVsCodeApi();
let count = 0;
let text = '';
const show = () => {
  document.getElementById('count').textContent = String(count);
  document.getElementById('text').textContent = text || '(nothing typed)';
};
document.getElementById('more').addEventListener('click', () => { count += 1; show(); });
window.qa = {
  more() { count += 1; show(); },
  write(words) { text = words; show(); },
  report() { api.postMessage({ type: 'report', count, text }); },
};
api.postMessage({ type: 'loaded' });
</script></body></html>`;
}

function provider(name, colour) {
  let loads = 0;
  return {
    resolveWebviewView(view) {
      view.webview.options = { enableScripts: true };
      view.webview.onDidReceiveMessage((message) => {
        if (message.type === 'loaded') {
          loads += 1;
          said(`${name} loaded ${loads}`);
        } else if (message.type === 'report') {
          said(`${name} holds ${message.count} clicks and "${message.text}" after ${loads} load(s)`);
        }
      });
      view.webview.html = page(name, colour);
    },
  };
}

exports.activate = (context) => {
  context.subscriptions.push(
    vscode.window.registerWebviewViewProvider('qa.chat', provider('chat', '#e8a33d')),
    vscode.window.registerWebviewViewProvider('qa.pad', provider('pad', '#4fb0e8')),
    vscode.window.registerWebviewViewProvider('qa.log', provider('log', '#7fd17f')),
  );
};

exports.deactivate = () => {};

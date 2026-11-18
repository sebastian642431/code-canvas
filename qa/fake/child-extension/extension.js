// The first time it starts (qa-spawned.txt marks it), a detached Node process that appends
// "tick" to qa-alive.txt in the first folder every 200 ms for a minute.
const fs = require('fs');
const path = require('path');
const { spawn } = require('child_process');
const vscode = require('vscode');

exports.activate = () => {
  const folder = vscode.workspace.workspaceFolders[0].uri.fsPath;
  const marker = path.join(folder, 'qa-spawned.txt');
  if (fs.existsSync(marker)) return;
  fs.writeFileSync(marker, 'spawned\n');
  const alive = path.join(folder, 'qa-alive.txt');
  const tick = () => {
    let n = 0;
    const t = setInterval(() => {
      require('fs').appendFileSync(process.argv[1], 'tick' + String.fromCharCode(10));
      if (++n > 300) clearInterval(t);
    }, 200);
  };
  const script = `(${tick.toString()})()`;
  const child = spawn(process.execPath, ['-e', script, alive], { detached: true, stdio: 'ignore', windowsHide: true });
  child.unref();
};

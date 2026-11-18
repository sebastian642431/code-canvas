// Appends "runs-code trusted=<workspace.isTrusted>" to qa-trust.txt in the first folder each
// time it starts.
const fs = require('fs');
const path = require('path');
const vscode = require('vscode');

exports.activate = () => {
  const folder = vscode.workspace.workspaceFolders && vscode.workspace.workspaceFolders[0];
  if (!folder) return;
  fs.appendFileSync(path.join(folder.uri.fsPath, 'qa-trust.txt'), `runs-code trusted=${vscode.workspace.isTrusted}\n`);
};

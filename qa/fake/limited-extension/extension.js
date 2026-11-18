// Appends "limited trusted=<workspace.isTrusted> path=<qa.limited.path>" to qa-limited.txt in
// the first folder each time it starts.
const fs = require('fs');
const path = require('path');
const vscode = require('vscode');

exports.activate = () => {
  const folder = vscode.workspace.workspaceFolders && vscode.workspace.workspaceFolders[0];
  if (!folder) return;
  const seen = vscode.workspace.getConfiguration('qa.limited').get('path');
  const line = `limited trusted=${vscode.workspace.isTrusted} path=${seen === '' || seen === undefined ? 'none' : seen}\n`;
  fs.appendFileSync(path.join(folder.uri.fsPath, 'qa-limited.txt'), line);
};

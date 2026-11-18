// Plays a language server for the rust-small fixture: `main` calls `triple` (where the
// names say it calls `step_1` and `double`) and is implemented by `triple`. It knows no
// references, so the uses stay the ones guessed by name.
const fs = require('fs');
const path = require('path');
const vscode = require('vscode');

const word = (doc, pos) => doc.getText(doc.getWordRangeAtPosition(pos));
function triple(doc) {
  const file = path.join(path.dirname(doc.uri.fsPath), 'util.rs');
  const lines = fs.readFileSync(file, 'utf8').split('\n');
  const line = lines.findIndex((l) => l.includes('fn triple'));
  const col = lines[line].indexOf('triple');
  return { uri: vscode.Uri.file(file), range: new vscode.Range(line, col, line, col + 'triple'.length) };
}

exports.activate = () => {
  const rust = { scheme: 'file' };
  vscode.languages.registerCallHierarchyProvider(rust, {
    prepareCallHierarchy(doc, pos) {
      if (word(doc, pos) !== 'main') return undefined;
      const r = doc.getWordRangeAtPosition(pos);
      return new vscode.CallHierarchyItem(vscode.SymbolKind.Function, 'main', '', doc.uri, r, r);
    },
    provideCallHierarchyOutgoingCalls(item) {
      const lines = fs.readFileSync(item.uri.fsPath, 'utf8').split('\n');
      const line = lines.findIndex((l) => l.includes('double('));
      const col = lines[line].indexOf('double(');
      const to = triple({ uri: item.uri });
      const callee = new vscode.CallHierarchyItem(vscode.SymbolKind.Function, 'triple', '', to.uri, to.range, to.range);
      return [new vscode.CallHierarchyOutgoingCall(callee, [new vscode.Range(line, col, line, col + 6)])];
    },
  });
  vscode.languages.registerImplementationProvider(rust, {
    provideImplementation(doc, pos) {
      if (word(doc, pos) !== 'main') return [];
      const to = triple(doc);
      return [new vscode.Location(to.uri, to.range)];
    },
  });
};

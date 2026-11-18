// qa.write.outside: a workspace edit of plain/src/main.rs, next to the first folder, by its
// full path and by one going up with '..'; the app must refuse both.
const path = require('path');
const vscode = require('vscode');

exports.activate = (context) => {
  context.subscriptions.push(
    vscode.commands.registerCommand('qa.write.outside', async () => {
      const folder = vscode.workspace.workspaceFolders[0].uri.fsPath;
      const targets = [path.resolve(folder, '..', 'plain', 'src', 'main.rs'), folder + path.sep + '..' + path.sep + 'plain' + path.sep + 'src' + path.sep + 'main.rs'];
      for (const target of targets) {
        const edit = new vscode.WorkspaceEdit();
        edit.insert(vscode.Uri.file(target), new vscode.Position(0, 0), '// changed by an extension\n');
        const done = await vscode.workspace.applyEdit(edit);
        console.log(`qa write asked for ${target}: ${done}`);
      }
    }),
  );
};

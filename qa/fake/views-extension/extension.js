// Code lenses and tree views for the scenarios and qa/host/views.test.js.
//
// Lenses: over every `fn` of a Rust file, "▶ Run" (runs qa.views.run with the function's
// name) and "N references", which comes without a command and is resolved later, the way
// language servers do; it runs editor.action.showReferences. Running qa.lenses.more adds a
// third lens, "more", and says the lenses changed.
//
// Trees: "Produce" lists fruits (collapsed) and vegetables (expanded) with codicons and
// descriptions; a fruit has a remove button (inline) and opens itself when clicked. "Add a
// fruit" in the title adds one and refreshes the tree. "Notes" is a flat list.
//
// Every command writes a line to qa-ran.txt in the first folder.
const fs = require('fs');
const path = require('path');
const vscode = require('vscode');

function ran(line) {
  const folder = vscode.workspace.workspaceFolders && vscode.workspace.workspaceFolders[0];
  if (!folder) return;
  fs.appendFileSync(path.join(folder.uri.fsPath, 'qa-ran.txt'), line + '\n');
  console.log('qa ran: ' + line);
}

const fruits = ['apple', 'banana', 'cherry'];
const vegetables = ['carrot', 'leek'];
let more = false;
let added = 0;

exports.activate = (context) => {
  const lensesChanged = new vscode.EventEmitter();
  context.subscriptions.push(
    vscode.languages.registerCodeLensProvider(
      { language: 'rust' },
      {
        onDidChangeCodeLenses: lensesChanged.event,
        provideCodeLenses(doc) {
          const out = [];
          for (let line = 0; line < doc.lineCount; line++) {
            const m = /^(\s*)(pub )?fn (\w+)/.exec(doc.lineAt(line).text);
            if (!m) continue;
            const name = m[3];
            const at = doc.lineAt(line).text.indexOf(name);
            const range = new vscode.Range(line, m[1].length, line, at + name.length);
            out.push(new vscode.CodeLens(range, { title: '▶ Run', command: 'qa.views.run', arguments: [name, line] }));
            const uses = new vscode.CodeLens(range);
            uses.name = name;
            uses.at = new vscode.Position(line, at);
            uses.uri = doc.uri;
            uses.count = doc.getText().split(name).length - 1;
            out.push(uses);
            if (more) out.push(new vscode.CodeLens(range, { title: 'more', command: 'qa.views.run', arguments: ['more ' + name] }));
          }
          return out;
        },
        resolveCodeLens(lens) {
          lens.command = {
            title: lens.count === 1 ? '1 reference' : `${lens.count} references`,
            command: 'editor.action.showReferences',
            arguments: [lens.uri, lens.at, []],
          };
          return lens;
        },
      },
    ),
  );

  const changed = new vscode.EventEmitter();
  const produce = {
    onDidChangeTreeData: changed.event,
    getChildren(element) {
      if (!element) return [{ kind: 'group', name: 'Fruits' }, { kind: 'group', name: 'Vegetables' }, { kind: 'readme' }];
      if (element.name === 'Fruits') return fruits.map((name) => ({ kind: 'fruit', name }));
      if (element.name === 'Vegetables') return vegetables.map((name) => ({ kind: 'vegetable', name }));
      return [];
    },
    getTreeItem(element) {
      if (element.kind === 'group') {
        const open = element.name === 'Vegetables' ? vscode.TreeItemCollapsibleState.Expanded : vscode.TreeItemCollapsibleState.Collapsed;
        const item = new vscode.TreeItem(element.name, open);
        item.iconPath = new vscode.ThemeIcon(element.name === 'Fruits' ? 'symbol-namespace' : 'symbol-folder');
        item.description = `${element.name === 'Fruits' ? fruits.length : vegetables.length} kinds`;
        item.contextValue = 'group';
        return item;
      }
      if (element.kind === 'readme') {
        const item = new vscode.TreeItem('Read me');
        item.iconPath = new vscode.ThemeIcon('book');
        item.tooltip = 'Opens nothing: writes a line';
        item.command = { command: 'qa.views.run', title: 'Read', arguments: ['readme'] };
        return item;
      }
      const item = new vscode.TreeItem(element.name, vscode.TreeItemCollapsibleState.None);
      item.iconPath = new vscode.ThemeIcon(element.kind === 'fruit' ? 'symbol-field' : 'symbol-constant');
      item.description = element.kind === 'fruit' ? 'sweet' : 'green';
      item.contextValue = element.kind;
      item.command = { command: 'qa.items.open', title: 'Open', arguments: [element.name] };
      return item;
    },
  };
  const view = vscode.window.createTreeView('qa.items', { treeDataProvider: produce, showCollapseAll: true });
  view.onDidExpandElement((e) => ran('expanded ' + e.element.name));
  view.onDidCollapseElement((e) => ran('collapsed ' + e.element.name));
  context.subscriptions.push(view);

  context.subscriptions.push(
    vscode.window.registerTreeDataProvider('qa.notes', {
      getChildren: (element) => (element ? [] : ['first note', 'second note']),
      getTreeItem: (note) => {
        const item = new vscode.TreeItem(note);
        item.iconPath = new vscode.ThemeIcon('note');
        return item;
      },
    }),
  );

  const command = (id, fn) => context.subscriptions.push(vscode.commands.registerCommand(id, fn));
  command('qa.views.run', (...args) => ran('run ' + args.join(' ')));
  command('qa.items.open', (name) => ran('open ' + name));
  command('qa.items.add', () => {
    fruits.push(['damson', 'elderberry', 'fig'][added++] || 'fruit ' + added);
    ran('add ' + fruits[fruits.length - 1]);
    changed.fire(undefined);
  });
  command('qa.items.refresh', () => {
    ran('refresh');
    changed.fire(undefined);
  });
  command('qa.items.remove', (element) => {
    const at = fruits.indexOf(element && element.name);
    if (at >= 0) fruits.splice(at, 1);
    ran('remove ' + (element && element.name));
    changed.fire(undefined);
  });
  command('qa.lenses.more', () => {
    more = !more;
    ran('more lenses ' + more);
    lensesChanged.fire();
  });
};

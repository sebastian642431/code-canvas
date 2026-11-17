const fs = require('fs');
const path = require('path');

const host = fs.readFileSync(path.join(__dirname, '..', '..', 'crates', 'code-canvas', 'exthost', 'host.js'), 'utf8');
const from = host.indexOf('function plainSnippet(value) {');
const to = host.indexOf('function inOrder(typed, text) {');
const plainSnippet = eval('(' + host.slice(from, to).trim() + ')');

const cases = [
  ['foo($0)', 'foo()', 4, 0],
  ['new(${1:cap}, ${2:b})$0', 'new(cap, b)', 4, 3],
  ['for (let ${1:i} = 0; $1 < n; $1++) {\n\t$0\n}', 'for (let i = 0; i < n; i++) {\n\t\n}', 9, 1],
  ['const ${1:Name} = 1; export default $1;', 'const Name = 1; export default Name;', 6, 4],
  ['printf("$1\\n");', 'printf("\\n");', 8, 0],
  ['\\begin{$1}', '\\begin{}', 7, 0],
  ['a\\$b \\} \\\\ c', 'a$b } \\ c', null, 0],
  ['${1/(.*)/${1:/upcase}/} end', ' end', 0, 0],
  ['${1:x} ${1/(.*)/${1:/upcase}/}!', 'x x!', 0, 1],
  ['${1|one,two|} $TM_X ${TM_FILENAME}.', 'one  .', 0, 3],
  ['${1:${2:inner}} $2', 'inner inner', 0, 5],
  ['plain $ sign and ${ broken', 'plain $ sign and ${ broken', null, 0],
  ['ñ(${1:é})', 'ñ(é)', 2, 1],
  ['plain', 'plain', null, 0],
];

let wrong = 0;
for (const [given, text, caret, select] of cases) {
  const got = plainSnippet(given);
  if (got.text !== text || got.caret !== caret || got.select !== select) {
    wrong += 1;
    console.log('WRONG', JSON.stringify(given), '=>', JSON.stringify(got), 'wanted', JSON.stringify({ text, caret, select }));
  }
}
console.log(wrong === 0 ? `PASS snippets (${cases.length})` : `FAIL snippets: ${wrong} of ${cases.length}`);
process.exit(wrong === 0 ? 0 : 1);

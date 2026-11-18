const fs = require('fs');
const path = require('path');
const assert = require('assert');

const host = fs.readFileSync(path.join(__dirname, '..', '..', 'crates', 'code-canvas', 'exthost', 'host.js'), 'utf8');
const piece = (from, to) => host.slice(host.indexOf(from), host.indexOf(to));
const EventEmitter = class {};
const knownLanguages = new Set();
const byExtension = new Map();
const byFilename = new Map();
const code = piece('const OURS = {', 'function folderSettingsFor(') +
  piece('function registerContributions(ext) {', 'function report(ext, status, error) {') +
  'return { config, registerContributions };';
const made = new Function('EventEmitter', 'knownLanguages', 'byExtension', 'byFilename', code);
const { config, registerContributions } = made(EventEmitter, knownLanguages, byExtension, byFilename);

registerContributions({ manifest: { contributes: { configuration: { properties: {
  'claudeCode.attachOpenFile': { type: 'boolean', default: true },
  'claudeCode.attachSelection': { type: 'boolean', default: true },
} } } } });
assert.strictEqual(config.defaults['claudeCode.attachOpenFile'], false, 'the open file is not attached unless asked for');
assert.strictEqual(config.defaults['claudeCode.attachSelection'], true, 'other defaults are the extension\'s');
console.log('settings: ok');

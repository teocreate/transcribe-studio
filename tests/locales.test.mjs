// Every interface language must define exactly the same keys, and every key
// the window asks for must exist. Run with: node tests/locales.test.mjs
import fs from 'node:fs';
import path from 'node:path';
import url from 'node:url';
import vm from 'node:vm';

const ui = path.join(path.dirname(url.fileURLToPath(import.meta.url)), '..', 'ui');
const sandbox = { window: {} };
for (const file of fs.readdirSync(path.join(ui, 'locales'))) {
  vm.runInNewContext(fs.readFileSync(path.join(ui, 'locales', file), 'utf8'), sandbox);
}
const locales = sandbox.window.LOCALES;
const reference = Object.keys(locales.en).sort();
let failures = 0;
const fail = (msg) => { failures += 1; console.log(`  FAIL  ${msg}`); };

for (const [lang, dict] of Object.entries(locales)) {
  const keys = Object.keys(dict);
  for (const key of reference) if (!(key in dict)) fail(`${lang}: missing ${key}`);
  for (const key of keys) if (!(key in locales.en)) fail(`${lang}: extra ${key}`);
  // Placeholders must match, or a translation silently drops a value.
  for (const key of reference) {
    const names = (v) => (typeof v === 'string' ? [...v.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort().join() : '');
    if (key in dict && names(dict[key]) !== names(locales.en[key])) fail(`${lang}: placeholders differ in ${key}`);
  }
}

// Keys referenced from markup and code.
const html = fs.readFileSync(path.join(ui, 'index.html'), 'utf8');
const js = fs.readFileSync(path.join(ui, 'app.js'), 'utf8');
const used = new Set([
  ...[...html.matchAll(/data-i18n(?:-\w+)?="([^"]+)"/g)].map((m) => m[1]),
  ...[...js.matchAll(/\b(?:t|tp|lookup)\('([\w.-]+)'/g)].map((m) => m[1]),
  ...[...js.matchAll(/emptyLabel: '([\w.]+)'/g)].map((m) => m[1]),
]);
for (const key of used) if (!(key in locales.en)) fail(`used but undefined: ${key}`);

// Dynamic families: one entry per model id, row status, env source.
const rust = fs.readFileSync(path.join(ui, '..', 'src', 'models.rs'), 'utf8');
for (const [, id] of rust.matchAll(/id: "([\w.-]+)"/g)) {
  if (!(`model.${id}` in locales.en)) fail(`no description for model ${id}`);
}

console.log(failures ? `\n${failures} problem(s)` : `\n${Object.keys(locales).length} languages, ${reference.length} keys each, ${used.size} used — all consistent`);
process.exit(failures ? 1 : 0);

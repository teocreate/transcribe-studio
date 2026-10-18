// Standalone check of the Markdown renderer that powers the preview pane.
// Run with: node tests/markdown.test.mjs
import fs from 'node:fs';
import path from 'node:path';
import url from 'node:url';

const here = path.dirname(url.fileURLToPath(import.meta.url));
const src = fs.readFileSync(path.join(here, '..', 'ui', 'app.js'), 'utf8');

// Lift the two pure functions out of the window module so they can be
// exercised without a DOM.
const slice = (from, to) => src.slice(src.indexOf(from), src.indexOf(to));
const moduleSrc =
  slice('function escapeHtml', 'function baseName') +
  slice('function renderMarkdown', 'function paintPreview') +
  '\nexport { renderMarkdown, escapeHtml };';

const tmp = path.join(here, '.markdown.mod.mjs');
fs.writeFileSync(tmp, moduleSrc);
const { renderMarkdown } = await import(url.pathToFileURL(tmp).href);
fs.unlinkSync(tmp);

const doc = `---
source: "lecture.m4a"
duration: 1:15
---

# Заголовок

Текст с **жирным**, *курсивом* и \`кодом\`.

## Список

- один
- два

1. первый
2. второй

### Код

\`\`\`toml
[server]
port = 8080
\`\`\`

> Цитата

| A | B |
| --- | --- |
| 1 | 2 |

[ссылка](https://example.com)

---
`;

const html = renderMarkdown(doc);

const cases = [
  ['front matter table', html.includes('<th>source</th>')],
  ['h1', html.includes('<h1>Заголовок</h1>')],
  ['h2', html.includes('<h2>Список</h2>')],
  ['bold', html.includes('<strong>жирным</strong>')],
  ['italic', html.includes('<em>курсивом</em>')],
  ['inline code', html.includes('<code>кодом</code>')],
  ['bullet list', html.includes('<ul>') && html.includes('<li>один</li>')],
  ['ordered list', html.includes('<ol>') && html.includes('<li>первый</li>')],
  ['code block', html.includes('<pre><code')],
  ['code block body', html.includes('port = 8080')],
  ['blockquote', html.includes('<blockquote>Цитата</blockquote>')],
  ['table', html.includes('<table>') && html.includes('<th>A</th>') && html.includes('<td>2</td>')],
  ['link', html.includes('<a href="https://example.com">ссылка</a>')],
  ['hr', html.includes('<hr />')],
  ['no leftover placeholder', !html.includes('FENCE')],
  ['escapes html', renderMarkdown('<img src=x onerror=alert(1)>').includes('&lt;img')],
];

let failed = 0;
for (const [name, ok] of cases) {
  if (!ok) failed += 1;
  console.log(`  ${ok ? 'ok  ' : 'FAIL'}  ${name}`);
}
if (failed) {
  console.log(`\n${failed} failing\n--- output ---\n${html}`);
  process.exit(1);
}
console.log(`\n${cases.length} checks passed`);

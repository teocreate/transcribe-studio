/* ══════════════════════════════════════════════════════════════════════════
   Transcribe Studio — window logic.

   The Rust side owns all state that outlives the window; this file is a view
   layer plus the small amount of bookkeeping a live progress readout needs.
   ══════════════════════════════════════════════════════════════════════════ */

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

/* Unhandled errors must not die silently inside the webview: without this a
   broken listener just makes the window look inert. */
function reportError(label, detail) {
  const text = `${label}: ${detail}`;
  invoke('log_client', { level: 'error', message: text }).catch(() => {});
  try { toast(text, 'err', 14000); } catch { /* toast host not up yet */ }
}

window.addEventListener('error', (e) => {
  reportError('JS', `${e.message} @ ${e.filename}:${e.lineno}:${e.colno}`);
});
window.addEventListener('unhandledrejection', (e) => {
  reportError('Promise', String(e.reason?.stack || e.reason));
});

const $ = (sel, root = document) => root.querySelector(sel);
const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];

/* ── State ─────────────────────────────────────────────────────────────── */

const app = {
  settings: null,
  models: [],
  env: null,
  machine: { cpu_threads: 8, suggested_threads: 6, ram_gb: 16, recommended_model: 'large-v3-turbo' },
  dataDir: '',
  mediaExt: [],
  docExt: [],
  tab: 'transcribe',
  queues: { transcribe: [], markdown: [] },
  busy: null,          // 'transcribe' | 'markdown' | null
  run: null,           // live progress bookkeeping
  downloads: new Map(),
  onboarding: false,   // the first-launch setup is running
  preview: { markdown: '', path: '', mode: 'rendered' },
};

/* ── Formatting ────────────────────────────────────────────────────────── */

function fmtBytes(bytes) {
  if (!bytes) return '—';
  const units = lookup('units');
  let value = bytes;
  let i = 0;
  while (value >= 1024 && i < units.length - 1) { value /= 1024; i += 1; }
  const digits = value >= 100 || i === 0 ? 0 : 1;
  const number = value.toLocaleString(currentLanguage(), {
    minimumFractionDigits: digits, maximumFractionDigits: digits,
  });
  return `${number} ${units[i]}`;
}

function fmtClock(seconds) {
  const total = Math.max(0, Math.round(seconds || 0));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const mm = String(m).padStart(h ? 2 : 1, '0');
  return h ? `${h}:${mm}:${String(s).padStart(2, '0')}` : `${mm}:${String(s).padStart(2, '0')}`;
}

function fmtDurationWords(seconds) {
  const total = Math.max(0, Math.round(seconds || 0));
  if (total < 60) return t('dur.s', { n: total });
  const h = Math.floor(total / 3600);
  const m = Math.round((total % 3600) / 60);
  if (!h) return t('dur.min', { n: m });
  return m ? t('dur.hmin', { h, m }) : t('dur.h', { n: h });
}

function escapeHtml(text) {
  return String(text)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

function baseName(path) {
  return String(path).split(/[\\/]/).pop();
}

function icon(id, size = 16) {
  return `<svg viewBox="0 0 24 24" class="ico ico--${size}" aria-hidden="true"><use href="#i-${id}" /></svg>`;
}

/* ── Toasts ────────────────────────────────────────────────────────────── */

function toast(message, kind = 'info', ttl = 5200) {
  const glyph = kind === 'err' ? 'alert' : kind === 'ok' ? 'check' : 'alert';
  const node = document.createElement('div');
  node.className = `toast toast--${kind}`;
  node.innerHTML = `<span class="toast__ico">${icon(glyph, 16)}</span><span>${escapeHtml(message)}</span>`;
  $('#toasts').append(node);
  const remove = () => {
    node.classList.add('is-out');
    node.addEventListener('animationend', () => node.remove(), { once: true });
  };
  setTimeout(remove, ttl);
  node.addEventListener('click', remove);
}

/* ── Reusable controls ─────────────────────────────────────────────────── */

/** Positions a segmented control's sliding indicator over the active item. */
function syncSegmented(root) {
  const thumb = $('[data-thumb]', root);
  const active = $('.segmented__item.is-active', root);
  if (!thumb || !active) return;
  thumb.style.width = `${active.offsetWidth}px`;
  thumb.style.transform = `translateX(${active.offsetLeft - 2}px)`;
}

function initSegmented(root, onChange) {
  const items = $$('.segmented__item', root);
  items.forEach((item) => {
    item.addEventListener('click', () => {
      if (item.classList.contains('is-active')) return;
      items.forEach((other) => {
        const active = other === item;
        other.classList.toggle('is-active', active);
        if (other.hasAttribute('role')) other.setAttribute('aria-selected', String(active));
      });
      syncSegmented(root);
      onChange?.(item.dataset.value);
    });
  });
  // Widths are only known after layout; re-measure once fonts settle.
  requestAnimationFrame(() => syncSegmented(root));
}

function setSegmented(root, value) {
  const items = $$('.segmented__item', root);
  items.forEach((item) => {
    const active = item.dataset.value === value;
    item.classList.toggle('is-active', active);
    if (item.hasAttribute('role')) item.setAttribute('aria-selected', String(active));
  });
  syncSegmented(root);
}

function initSwitch(node, onChange) {
  node.addEventListener('click', () => {
    const next = !node.classList.contains('is-on');
    node.classList.toggle('is-on', next);
    node.setAttribute('aria-checked', String(next));
    onChange?.(next);
  });
}

function setSwitch(node, on) {
  node.classList.toggle('is-on', !!on);
  node.setAttribute('aria-checked', String(!!on));
}

/* ── Theme ─────────────────────────────────────────────────────────────── */

function applyTheme(theme) {
  if (theme === 'light' || theme === 'dark') {
    document.documentElement.dataset.theme = theme;
  } else {
    delete document.documentElement.dataset.theme;
  }
  // Segmented thumbs are measured in pixels, so re-sync after a repaint.
  requestAnimationFrame(() => $$('.segmented').forEach(syncSegmented));
}

/* ── Settings persistence ──────────────────────────────────────────────── */

let saveTimer = null;
function persist() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    invoke('save_settings', { next: app.settings }).catch((err) => {
      toast(t('toast.save_failed', { err }), 'err');
    });
  }, 320);
}

function update(patch) {
  Object.assign(app.settings, patch);
  persist();
}

/* ── Queues ────────────────────────────────────────────────────────────── */

const queueConfig = {
  transcribe: {
    list: '#t-files', panel: '#t-filepanel', drop: '#t-drop', count: '#t-count',
    meta: '#t-runmeta', start: '#t-start',
    emptyLabel: 'common.no_files',
    accept: (info) => info.kind === 'media',
  },
  markdown: {
    list: '#m-files', panel: '#m-filepanel', drop: '#m-drop', count: '#m-count',
    meta: '#m-runmeta', start: '#m-start',
    emptyLabel: 'common.no_files',
    accept: () => true,
  },
};

function renderQueue(kind) {
  const cfg = queueConfig[kind];
  const items = app.queues[kind];
  const list = $(cfg.list);
  const hasItems = items.length > 0;

  $(cfg.panel).hidden = !hasItems;
  $(cfg.drop).classList.toggle('dropzone--compact', hasItems);
  $(cfg.count).textContent = String(items.length);

  list.innerHTML = items.map((item, index) => {
    const glyph = item.kind === 'media' ? 'waves' : 'doc';
    const state = item.status === 'done' ? 'is-done'
      : item.status === 'error' ? 'is-error'
      : item.status === 'active' ? 'is-active' : '';
    const label = ['done', 'error', 'active', 'skipped'].includes(item.status)
      ? t(`row.${item.status}`) : '';
    // A converted file opens in the preview; a failed one explains why.
    const openable = kind === 'markdown' && (item.output || item.error);
    const selected = openable && item.output && item.output === app.preview.path ? ' is-selected' : '';
    return `
      <li class="filerow ${state}${openable ? ' is-openable' : ''}${selected}" data-index="${index}"
          ${openable ? `tabindex="0" title="${escapeHtml(item.output ? t('m.open_preview') : item.error)}"` : ''}>
        <span class="filerow__ico">${icon(glyph, 16)}</span>
        <span>
          <span class="filerow__name" title="${escapeHtml(item.path)}">${escapeHtml(item.name)}</span>
          <span class="filerow__meta">${escapeHtml(item.ext.toUpperCase())} · ${fmtBytes(item.size)}</span>
        </span>
        <span class="filerow__status">${escapeHtml(label)}</span>
        <button class="icon-btn icon-btn--sm" data-remove="${index}"
                aria-label="${escapeHtml(t('common.remove', { name: item.name }))}">${icon('x', 14)}</button>
      </li>`;
  }).join('');

  $$('.filerow.is-openable', list).forEach((row) => {
    const open = () => openQueuedResult(Number(row.dataset.index));
    row.addEventListener('click', (e) => { if (!e.target.closest('[data-remove]')) open(); });
    row.addEventListener('keydown', (e) => {
      if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); open(); }
    });
  });

  $$('[data-remove]', list).forEach((btn) => {
    btn.addEventListener('click', () => {
      if (app.busy === kind) return;
      app.queues[kind].splice(Number(btn.dataset.remove), 1);
      renderQueue(kind);
    });
  });

  const totalBytes = items.reduce((sum, item) => sum + item.size, 0);
  $(cfg.meta).textContent = hasItems
    ? `${items.length} ${tp('plural.files', items.length)} · ${fmtBytes(totalBytes)}`
    : t(cfg.emptyLabel);
  $(cfg.start).disabled = !hasItems || app.busy !== null;
}

/** Shows a converted file from the Markdown queue in the preview pane. */
async function openQueuedResult(index) {
  const item = app.queues.markdown[index];
  if (!item) return;
  if (!item.output) {
    if (item.error) toast(`${item.name}: ${item.error}`, 'err', 9000);
    return;
  }
  try {
    const text = await invoke('read_text', { path: item.output, limit: 200000 });
    app.preview = { markdown: text, path: item.output, mode: app.preview.mode };
    paintPreview();
    renderQueue('markdown');
  } catch (err) {
    toast(String(err), 'err');
  }
}

function setRowStatus(kind, index, status) {
  const item = app.queues[kind][index];
  if (!item) return;
  item.status = status;
  renderQueue(kind);
}

async function addFiles(kind, paths) {
  if (!paths?.length) return;
  if (app.busy) {
    toast(t('toast.wait_task'), 'err', 3500);
    return;
  }
  const infos = await invoke('inspect_files', { files: paths });
  const existing = new Set(app.queues[kind].map((item) => item.path));
  const cfg = queueConfig[kind];

  let rejected = 0;
  for (const info of infos) {
    if (existing.has(info.path)) continue;
    if (!cfg.accept(info)) { rejected += 1; continue; }
    app.queues[kind].push({ ...info, status: 'queued' });
    existing.add(info.path);
  }
  if (rejected) {
    toast(t('toast.not_audio', { n: rejected }), 'err');
  }
  renderQueue(kind);
}

/* ── Tabs ──────────────────────────────────────────────────────────────── */

function switchTab(tab) {
  app.tab = tab;
  for (const name of ['transcribe', 'markdown']) {
    const view = $(`#view-${name}`);
    const active = name === tab;
    if (active) {
      view.hidden = false;
      requestAnimationFrame(() => view.classList.add('is-active'));
    } else {
      view.classList.remove('is-active');
      setTimeout(() => { if (app.tab !== name) view.hidden = true; }, 200);
    }
  }
  requestAnimationFrame(() => $$('.segmented').forEach(syncSegmented));
}

/* ── Transcription ─────────────────────────────────────────────────────── */

function resetRun(total) {
  app.run = {
    total,
    completed: 0,
    index: -1,
    duration: 0,
    position: 0,
    startedAt: 0,
    processedAudio: 0,
  };
  $('#t-stream').innerHTML = '';
  $('#t-bar').style.width = '0%';
  $('#t-pct').textContent = '0%';
  $('#t-current').textContent = '—';
  $('#t-substatus').textContent = t('t.loading_model');
  $('#t-stat-pos').textContent = '0:00 / 0:00';
  $('#t-stat-speed').textContent = '—';
  $('#t-stat-eta').textContent = '—';
  $('#t-progress .bar').classList.add('bar--indeterminate');
}

function paintProgress() {
  const run = app.run;
  if (!run) return;

  const fileFraction = run.duration > 0 ? Math.min(run.position / run.duration, 1) : 0;
  const overall = run.total > 0 ? ((run.completed + fileFraction) / run.total) * 100 : 0;

  $('#t-bar').style.width = `${overall.toFixed(1)}%`;
  $('#t-pct').textContent = `${Math.floor(overall)}%`;
  $('#t-stat-pos').textContent = `${fmtClock(run.position)} / ${fmtClock(run.duration)}`;

  const wall = (Date.now() - run.startedAt) / 1000;
  if (wall > 2 && run.position > 0) {
    const rate = run.position / wall;
    const shown = rate.toLocaleString(currentLanguage(), { minimumFractionDigits: 1, maximumFractionDigits: 1 });
    $('#t-stat-speed').textContent = t('t.speed', { rate: shown });
    const remaining = Math.max(0, run.duration - run.position) / rate;
    $('#t-stat-eta').textContent = t('t.eta', { time: fmtDurationWords(remaining) });
  }
}

function pushSegment(start, text) {
  if (!text) return;
  const stream = $('#t-stream');
  const line = document.createElement('div');
  line.className = 'stream__line';
  line.innerHTML = `<span class="stream__t">${fmtClock(start)}</span><span class="stream__txt">${escapeHtml(text)}</span>`;
  stream.append(line);
  // Keep the transcript light: the file on disk is the real record.
  while (stream.childElementCount > 120) stream.firstElementChild.remove();
  stream.scrollTop = stream.scrollHeight;
}

function handleTranscribeEvent(event) {
  const run = app.run;
  switch (event.t) {
    case 'env':
      $('#t-substatus').textContent =
        `${event.model} · ${event.compute_type} · ${event.cpu_threads} ${tp('plural.threads', event.cpu_threads)}`;
      break;

    case 'wake_lock':
      $('#wake-pill').hidden = !event.active;
      break;

    case 'file_start':
      if (!run) break;
      run.index = event.index;
      run.duration = 0;
      run.position = 0;
      run.startedAt = Date.now();
      $('#t-current').textContent = event.name;
      $('#t-progress .bar').classList.add('bar--indeterminate');
      setRowStatus('transcribe', event.index, 'active');
      break;

    case 'meta':
      if (!run) break;
      run.duration = event.duration || 0;
      $('#t-progress .bar').classList.remove('bar--indeterminate');
      $('#t-substatus').textContent = t('t.meta', {
        duration: fmtDurationWords(run.duration),
        lang: event.language,
        prob: event.language_probability ? ` (${Math.round(event.language_probability * 100)}%)` : '',
        i: run.index + 1,
        n: run.total,
      });
      paintProgress();
      break;

    case 'seg':
      if (!run) break;
      run.position = event.end || run.position;
      if (!run.duration && event.duration) run.duration = event.duration;
      pushSegment(event.start, event.text);
      paintProgress();
      break;

    case 'file_done':
      if (!run) break;
      run.completed += 1;
      run.position = run.duration;
      paintProgress();
      setRowStatus('transcribe', event.index, 'done');
      break;

    case 'file_skip':
      if (run) run.completed += 1;
      setRowStatus('transcribe', event.index, 'skipped');
      break;

    case 'file_error':
      if (run) run.completed += 1;
      setRowStatus('transcribe', event.index, 'error');
      toast(`${baseName(event.path)}: ${event.msg}`, 'err', 9000);
      break;

    case 'log':
      $('#t-substatus').textContent = event.msg;
      break;

    case 'fatal':
      toast(event.msg, 'err', 11000);
      break;

    default:
      break;
  }
}

function renderOutputs(outputs, extra = []) {
  const all = [...outputs, ...extra];
  if (!all.length) { $('#t-result').hidden = true; return; }
  $('#t-outputs').innerHTML = all.map((path, i) => {
    const ext = (path.split('.').pop() || '').toUpperCase();
    const markdown = ext === 'MD';
    return `
      <li class="outrow">
        <span class="outrow__tag">${escapeHtml(ext)}</span>
        <span class="outrow__name" title="${escapeHtml(path)}">${escapeHtml(path)}</span>
        ${markdown ? `<button class="btn btn--plain" data-preview="${i}">${escapeHtml(t('common.open'))}</button>` : ''}
        <button class="icon-btn icon-btn--sm" data-reveal="${i}"
                aria-label="${escapeHtml(t('common.reveal'))}" title="${escapeHtml(t('common.reveal'))}">${icon('open', 15)}</button>
      </li>`;
  }).join('');
  $$('[data-reveal]', $('#t-outputs')).forEach((btn) => {
    btn.addEventListener('click', () => {
      invoke('reveal_path', { path: all[Number(btn.dataset.reveal)] })
        .catch((err) => toast(String(err), 'err'));
    });
  });
  // A finished transcript is most useful read, not located — open it straight
  // into the Markdown pane rather than making the user hunt for it on disk.
  $$('[data-preview]', $('#t-outputs')).forEach((btn) => {
    btn.addEventListener('click', async () => {
      const path = all[Number(btn.dataset.preview)];
      try {
        const text = await invoke('read_text', { path, limit: 200000 });
        app.preview = { markdown: text, path, mode: app.preview.mode };
        paintPreview();
        setSegmented($('#tabs'), 'markdown');
        switchTab('markdown');
      } catch (err) {
        toast(String(err), 'err');
      }
    });
  });
  $('#t-result').hidden = false;
}

function setBusy(kind) {
  app.busy = kind;
  $('#t-start').hidden = kind === 'transcribe';
  $('#t-cancel').hidden = kind !== 'transcribe';
  $('#m-start').hidden = kind === 'markdown';
  $('#m-cancel').hidden = kind !== 'markdown';
  $('#t-start').disabled = kind !== null || app.queues.transcribe.length === 0;
  $('#m-start').disabled = kind !== null || app.queues.markdown.length === 0;
}

async function startTranscribe() {
  const files = app.queues.transcribe.map((item) => item.path);
  if (!files.length) return;

  if (!app.env?.ready_transcribe) {
    toast(t('toast.env_not_ready'), 'err', 8000);
    openSettings('env');
    return;
  }
  const model = app.models.find((m) => m.id === app.settings.model);
  if (!model?.installed) {
    toast(t('toast.model_missing', { name: model?.name || app.settings.model }), 'err', 8000);
    openSettings('models');
    return;
  }

  app.queues.transcribe.forEach((item) => { item.status = 'queued'; });
  renderQueue('transcribe');
  resetRun(files.length);
  $('#t-progress').hidden = false;
  $('#t-result').hidden = true;
  setBusy('transcribe');

  try {
    const summary = await invoke('start_transcribe', { files, settings: app.settings });
    renderOutputs(summary.outputs, summary.markdown_outputs);
    if (summary.cancelled) {
      toast(summary.outputs.length
        ? t('toast.stopped_saved', { n: summary.outputs.length })
        : t('toast.stopped'), 'info');
    } else if (summary.failures > 0) {
      toast(t('toast.done_errors', { n: summary.failures }), 'err', 8000);
    } else if (summary.outputs.length) {
      const n = summary.outputs.length;
      toast(t('toast.done_files', { n, files: tp('plural.files', n) }), 'ok');
    } else {
      toast(t('toast.nothing_new'), 'info');
    }
  } catch (err) {
    toast(String(err), 'err', 11000);
  } finally {
    setBusy(null);
    $('#t-progress').hidden = true;
    $('#wake-pill').hidden = true;
    // A stopped run leaves the current file mid-flight; it is queued again,
    // not "in progress".
    app.queues.transcribe.forEach((item) => {
      if (item.status === 'active') item.status = 'queued';
    });
    renderQueue('transcribe');
  }
}

/* ── Markdown ──────────────────────────────────────────────────────────── */

/** Minimal CommonMark subset. Input is escaped first, so document content can
 *  never inject markup into the preview. */
function renderMarkdown(src) {
  const fences = [];
  let text = String(src).replace(/\r\n?/g, '\n');

  // Front matter produced by our own transcripts reads better as a header.
  let frontMatter = '';
  const fm = text.match(/^---\n([\s\S]*?)\n---\n/);
  if (fm) {
    const rows = fm[1].split('\n').filter(Boolean).map((line) => {
      const at = line.indexOf(':');
      if (at < 0) return '';
      const key = escapeHtml(line.slice(0, at).trim());
      const value = escapeHtml(line.slice(at + 1).trim().replace(/^"|"$/g, ''));
      return `<tr><th>${key}</th><td>${value}</td></tr>`;
    }).join('');
    if (rows) frontMatter = `<table>${rows}</table>`;
    text = text.slice(fm[0].length);
  }

  // Stash fenced code so its contents never hit the inline pass. NUL is
  // the sentinel: it cannot occur in text that arrived as a JSON string.
  text = text.replace(/```([^\n]*)\n([\s\S]*?)```/g, (_, lang, body) => {
    fences.push(`<pre><code data-lang="${escapeHtml(lang.trim())}">${escapeHtml(body.replace(/\n$/, ''))}</code></pre>`);
    return `\u0000FENCE${fences.length - 1}\u0000`;
  });

  const inline = (raw) => escapeHtml(raw)
    .replace(/`([^`]+)`/g, (_, code) => `<code>${code}</code>`)
    .replace(/!\[([^\]]*)\]\(([^)\s]+)[^)]*\)/g, (_, alt, href) =>
      `<a href="${href}" title="${alt}">${alt || href}</a>`)
    .replace(/\[([^\]]+)\]\(([^)\s]+)[^)]*\)/g, (_, label, href) => `<a href="${href}">${label}</a>`)
    .replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>')
    .replace(/(^|[\s(])\*([^*\n]+)\*/g, '$1<em>$2</em>')
    .replace(/(^|[\s(])_([^_\n]+)_/g, '$1<em>$2</em>')
    .replace(/~~([^~]+)~~/g, '<s>$1</s>');

  const lines = text.split('\n');
  const out = [];
  let paragraph = [];
  let list = null;      // 'ul' | 'ol'
  let quote = [];

  const flushParagraph = () => {
    if (!paragraph.length) return;
    out.push(`<p>${inline(paragraph.join(' '))}</p>`);
    paragraph = [];
  };
  const flushList = () => { if (list) { out.push(`</${list}>`); list = null; } };
  const flushQuote = () => {
    if (!quote.length) return;
    out.push(`<blockquote>${inline(quote.join(' '))}</blockquote>`);
    quote = [];
  };
  const flushAll = () => { flushParagraph(); flushList(); flushQuote(); };

  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i];
    const trimmed = line.trim();

    if (!trimmed) { flushAll(); continue; }

    if (trimmed.startsWith('\u0000FENCE')) {
      flushAll();
      out.push(fences[Number(trimmed.replace(/\D/g, ''))]);
      continue;
    }

    const heading = trimmed.match(/^(#{1,6})\s+(.*)$/);
    if (heading) {
      flushAll();
      const level = Math.min(heading[1].length, 4);
      out.push(`<h${level}>${inline(heading[2])}</h${level}>`);
      continue;
    }

    if (/^([-*_])\s*(\1\s*){2,}$/.test(trimmed)) { flushAll(); out.push('<hr />'); continue; }

    // Table: a header row followed by a |---|---| separator.
    if (trimmed.includes('|') && /^\s*\|?[\s:-]*-[\s:|-]*\|?\s*$/.test(lines[i + 1] || '')) {
      flushAll();
      const cells = (row) => row.trim().replace(/^\||\|$/g, '').split('|').map((c) => c.trim());
      const head = cells(trimmed);
      const body = [];
      i += 2;
      while (i < lines.length && lines[i].includes('|')) { body.push(cells(lines[i])); i += 1; }
      i -= 1;
      out.push(
        `<table><thead><tr>${head.map((c) => `<th>${inline(c)}</th>`).join('')}</tr></thead>` +
        `<tbody>${body.map((row) => `<tr>${row.map((c) => `<td>${inline(c)}</td>`).join('')}</tr>`).join('')}</tbody></table>`
      );
      continue;
    }

    const quoted = trimmed.match(/^>\s?(.*)$/);
    if (quoted) { flushParagraph(); flushList(); quote.push(quoted[1]); continue; }
    flushQuote();

    const bullet = trimmed.match(/^[-*+]\s+(.*)$/);
    const numbered = trimmed.match(/^\d+[.)]\s+(.*)$/);
    if (bullet || numbered) {
      flushParagraph();
      const wanted = bullet ? 'ul' : 'ol';
      if (list !== wanted) { flushList(); list = wanted; out.push(`<${wanted}>`); }
      out.push(`<li>${inline((bullet || numbered)[1])}</li>`);
      continue;
    }
    flushList();

    paragraph.push(trimmed);
  }
  flushAll();

  return frontMatter + out.join('\n');
}

function paintPreview() {
  const body = $('#m-body');
  const { markdown, mode, path } = app.preview;

  if (!markdown) {
    body.innerHTML = `<div class="empty">${icon('doc', 28)}<p>${escapeHtml(t('m.empty'))}</p></div>`;
    $('#m-copy').disabled = true;
    $('#m-reveal').disabled = true;
    return;
  }

  const header = `
    <div class="preview__bar">${icon('md', 15)}
      <span><strong>${escapeHtml(baseName(path))}</strong> · ${escapeHtml(t('m.chars', { n: fmtNumber(markdown.length) }))}</span>
    </div>`;

  body.innerHTML = mode === 'source'
    ? `${header}<pre class="md-source">${escapeHtml(markdown)}</pre>`
    : `${header}<div class="md">${renderMarkdown(markdown)}</div>`;

  // Links would navigate the webview away from the app shell.
  $$('.md a', body).forEach((a) => a.addEventListener('click', (e) => e.preventDefault()));

  body.scrollTop = 0;
  $('#m-copy').disabled = false;
  $('#m-reveal').disabled = false;
}

function handleMarkdownEvent(event) {
  switch (event.t) {
    case 'file_start':
      setRowStatus('markdown', event.index, 'active');
      app.mdCurrent = event.name;
      $('#m-runmeta').textContent = t('m.converting', { name: event.name });
      break;
    case 'ocr':
      $('#m-runmeta').textContent = t('m.ocr_progress', {
        name: app.mdCurrent || '', page: event.page, pages: event.pages,
      });
      break;
    case 'file_done': {
      const item = app.queues.markdown[event.index];
      if (item) item.output = event.output;
      app.preview = { markdown: event.preview, path: event.output, mode: app.preview.mode };
      paintPreview();
      setRowStatus('markdown', event.index, 'done');
      break;
    }
    case 'file_error': {
      const item = app.queues.markdown[event.index];
      if (item) item.error = event.msg;
      setRowStatus('markdown', event.index, 'error');
      toast(`${baseName(event.path)}: ${event.msg}`, 'err', 9000);
      break;
    }
    case 'fatal':
      toast(event.msg, 'err', 11000);
      break;
    default:
      break;
  }
}

async function startMarkdown() {
  const files = app.queues.markdown.map((item) => item.path);
  if (!files.length) return;

  if (!app.env?.ready_markdown) {
    toast(t('toast.md_missing'), 'err', 8000);
    openSettings('env');
    return;
  }

  app.queues.markdown.forEach((item) => {
    item.status = 'queued';
    delete item.output;
    delete item.error;
  });
  renderQueue('markdown');
  setBusy('markdown');

  try {
    const outputs = await invoke('start_markdown', {
      files,
      outputDir: app.settings.md_output_dir || '',
    });
    const failed = app.queues.markdown.filter((item) => item.status === 'error').length;
    if (!outputs.length) {
      toast(t('toast.nothing_converted'), 'err', 8000);
    } else if (failed) {
      toast(t('toast.done_errors', { n: failed }), 'err', 8000);
    } else {
      toast(t('toast.done_files', { n: outputs.length, files: tp('plural.files', outputs.length) }), 'ok');
    }
  } catch (err) {
    toast(String(err), 'err', 11000);
  } finally {
    setBusy(null);
    renderQueue('markdown');
  }
}

/* ── Models ────────────────────────────────────────────────────────────── */

function meterDots(value, max = 5) {
  return `<span class="meter__dots">${
    Array.from({ length: max }, (_, i) => `<span class="meter__dot${i < value ? ' is-on' : ''}"></span>`).join('')
  }</span>`;
}

function renderModelSelect() {
  const select = $('#s-model');
  select.innerHTML = app.models.map((m) => {
    const size = m.installed
      ? fmtBytes(m.disk_bytes)
      : t('models.not_downloaded', { size: fmtBytes(m.size_mb * 1024 * 1024) });
    const pick = m.id === app.machine.recommended_model ? ` · ${t('models.recommended')}` : '';
    return `<option value="${m.id}">${escapeHtml(`${m.name} · ${size}${pick}`)}</option>`;
  }).join('');
  select.value = app.settings.model;
  // A settings file can name a model that was since removed from the catalogue.
  if (!select.value && app.models.length) {
    select.value = app.models[0].id;
    update({ model: select.value });
  }
  renderModelNote();
}

function renderModelNote() {
  const model = app.models.find((m) => m.id === app.settings.model);
  const note = $('#s-model-note');
  if (!model) { note.textContent = ''; return; }
  const description = t(`model.${model.id}`);
  note.textContent = model.installed
    ? description
    : t('models.need_download', { note: description });
  $('#st-model').textContent = model.installed
    ? t('status.model', { name: model.name })
    : t('status.model_missing', { name: model.name });
}

function renderModelCards() {
  $('#model-cards').innerHTML = app.models.map((m) => {
    const progress = app.downloads.get(m.id);
    const fullSize = m.size_mb * 1024 * 1024;
    // A partially downloaded model resumes where it stopped.
    const partial = !m.installed && m.partial_bytes > 0;
    const action = progress
      ? `<div class="bar"><div class="bar__fill" style="width:${progress.percent.toFixed(1)}%"></div></div>
         <span class="mcard__pct">${Math.floor(progress.percent)}%</span>
         <button class="btn btn--plain btn--danger" data-cancel="${m.id}">${escapeHtml(t('models.cancel'))}</button>`
      : m.installed
        ? `<span class="mcard__ok">${icon('check', 14)}${escapeHtml(t('models.installed'))}</span>
           <button class="btn btn--plain btn--danger" data-delete="${m.id}"
                   style="margin-left:auto">${icon('trash', 14)}${escapeHtml(t('models.delete'))}</button>`
        : `${partial ? `<button class="btn btn--plain btn--danger" data-delete="${m.id}">${icon('trash', 14)}${escapeHtml(t('models.delete'))}</button>` : ''}
           <button class="btn btn--tinted" data-download="${m.id}"
                   style="margin-left:auto">${icon('download', 15)}${escapeHtml(t(partial ? 'models.resume' : 'models.download'))}</button>`;
    const size = m.installed
      ? fmtBytes(m.disk_bytes)
      : partial
        ? t('models.partial', { done: fmtBytes(m.partial_bytes), total: fmtBytes(fullSize) })
        : fmtBytes(fullSize);
    const recommended = m.id === app.machine.recommended_model;
    const why = t('models.recommended_why', { threads: app.machine.cpu_threads, ram: fmtNumber(app.machine.ram_gb) });

    return `
      <article class="mcard${m.installed ? ' is-installed' : ''}">
        <div class="mcard__top">
          <h3 class="mcard__name">${escapeHtml(m.name)}
            ${recommended ? `<span class="mcard__badge" title="${escapeHtml(why)}">${escapeHtml(t('models.recommended'))}</span>` : ''}</h3>
          <span class="mcard__size">${escapeHtml(size)}</span>
        </div>
        <p class="mcard__note">${escapeHtml(t(`model.${m.id}`))}</p>
        <div class="mcard__meters">
          <span class="meter">${escapeHtml(t('models.speed'))} ${meterDots(m.speed)}</span>
          <span class="meter">${escapeHtml(t('models.quality'))} ${meterDots(m.quality)}</span>
        </div>
        <div class="mcard__foot">${action}</div>
      </article>`;
  }).join('');

  $$('[data-download]').forEach((btn) => btn.addEventListener('click', () => downloadModel(btn.dataset.download)));
  $$('[data-cancel]').forEach((btn) => btn.addEventListener('click', () => {
    invoke('cancel_model_download', { id: btn.dataset.cancel });
  }));
  $$('[data-delete]').forEach((btn) => btn.addEventListener('click', () => deleteModel(btn.dataset.delete)));
}

async function refreshModels() {
  app.models = await invoke('list_models');
  renderModelSelect();
  renderModelCards();
}

async function downloadModel(id) {
  const model = app.models.find((m) => m.id === id);
  app.downloads.set(id, { percent: 0 });
  renderModelCards();
  try {
    await invoke('download_model', { id });
    app.downloads.delete(id);
    await refreshModels();
    toast(t('toast.model_ready', { name: model?.name || id }), 'ok');
    // A freshly downloaded model is almost always the one you wanted to use.
    if (!app.models.find((m) => m.id === app.settings.model)?.installed) {
      update({ model: id });
      renderModelSelect();
    }
    return true;
  } catch (err) {
    app.downloads.delete(id);
    // What was fetched so far is kept for resuming; show it on the card.
    await refreshModels().catch(() => renderModelCards());
    toast(String(err), 'err', 9000);
    return false;
  }
}

async function deleteModel(id) {
  const model = app.models.find((m) => m.id === id);
  try {
    await invoke('delete_model', { id });
    await refreshModels();
    toast(t('toast.model_deleted', { name: model?.name || id }), 'ok');
  } catch (err) {
    toast(String(err), 'err');
  }
}

/* ── Environment ───────────────────────────────────────────────────────── */

function renderEnv() {
  const env = app.env;
  const probe = env?.probe || {};
  const sourceLabel = ['managed', 'none'].includes(env?.source)
    ? t(`env.source.${env.source}`) : '—';

  const card = (label, value, state) => `
    <div class="envcard">
      <p class="envcard__label">${escapeHtml(label)}</p>
      <p class="envcard__value${state ? ` is-${state}` : ''}">${escapeHtml(value)}</p>
    </div>`;

  $('#env-grid').innerHTML = [
    card(t('env.python'), probe.python ? `${probe.python}` : t('env.not_found'), probe.python ? 'ok' : 'missing'),
    card(t('env.location'), env?.python || sourceLabel),
    card('faster-whisper', probe.faster_whisper || t('env.not_installed'), probe.faster_whisper ? 'ok' : 'missing'),
    card('markitdown', probe.markitdown || t('env.not_installed'), probe.markitdown ? 'ok' : 'missing'),
    card('CTranslate2', probe.ctranslate2 || '—'),
    card('PyAV', probe.av || '—'),
    card(t('env.ocr'), env?.ready_ocr
      ? t('env.ocr_ready', { langs: (env.ocr_languages || []).join(', ') })
      : !env?.tesseract ? t('env.ocr_no_bundle') : t('env.ocr_need_setup'),
    env?.ready_ocr ? 'ok' : 'missing'),
  ].join('');

  const bar = $('#st-env');
  const dot = $('.dot', bar);
  const text = $('span:last-child', bar);
  if (env?.ready_transcribe && env?.ready_markdown) {
    dot.className = 'dot dot--ok';
    text.textContent = t('status.env_ready', { v: probe.python });
  } else if (env?.ready_transcribe) {
    dot.className = 'dot dot--warn';
    text.textContent = t('status.no_markitdown');
  } else {
    dot.className = 'dot dot--bad';
    text.textContent = t('status.env_missing');
  }
  // Keep the status text out of the static pass from now on: it is live.
  delete text.dataset.i18n;
  renderOnboarding();
}

/* ── First launch ──────────────────────────────────────────────────────────
   One button that installs the environment and then the model recommended
   for this machine, so a fresh install needs no knowledge of either. */

function renderOnboarding() {
  const card = $('#onboard');
  const model = app.models.find((m) => m.id === app.machine.recommended_model);
  const needed = !app.env?.ready_transcribe || (model && !model.installed && !app.models.some((m) => m.installed));
  card.hidden = !needed && !app.onboarding;
  if (card.hidden || !model) return;
  $('#onboard-text').textContent = t('onboard.text', {
    model: model.name, size: fmtBytes(model.size_mb * 1024 * 1024),
  });
}

async function runOnboarding() {
  const button = $('#onboard-go');
  const step = $('#onboard-step');
  const model = app.models.find((m) => m.id === app.machine.recommended_model);
  app.onboarding = true;
  button.disabled = true;
  step.hidden = false;
  try {
    if (!app.env?.ready_transcribe || !app.env?.ready_markdown) {
      step.textContent = t('onboard.step_env');
      if (!await setupEnv()) return;
    }
    if (model && !model.installed) {
      step.textContent = t('onboard.step_model', { model: model.name });
      selectPane('models');
      if (!await downloadModel(model.id)) return;
      update({ model: model.id });
      renderModelSelect();
    }
    toast(t('toast.all_ready'), 'ok');
  } finally {
    app.onboarding = false;
    button.disabled = false;
    step.hidden = true;
    renderOnboarding();
  }
}

async function refreshEnv() {
  app.env = await invoke('env_status');
  renderEnv();
}

async function setupEnv() {
  const button = $('#env-setup');
  const log = $('#env-log');
  button.disabled = true;
  log.hidden = false;
  log.textContent = '';
  try {
    app.env = await invoke('setup_env');
    renderEnv();
    toast(t('toast.env_ready'), 'ok');
    return true;
  } catch (err) {
    toast(String(err), 'err', 12000);
    return false;
  } finally {
    button.disabled = false;
  }
}

/* ── Settings sheet ────────────────────────────────────────────────────── */

function openSettings(pane = 'models') {
  const backdrop = $('#settings-backdrop');
  backdrop.hidden = false;
  backdrop.classList.remove('is-closing');
  selectPane(pane);
  requestAnimationFrame(() => $$('.segmented', backdrop).forEach(syncSegmented));
  $('#close-settings').focus();
}

function closeSettings() {
  const backdrop = $('#settings-backdrop');
  backdrop.classList.add('is-closing');
  setTimeout(() => {
    backdrop.hidden = true;
    backdrop.classList.remove('is-closing');
  }, 200);
}

function selectPane(name) {
  $$('.sheet__navitem').forEach((item) => item.classList.toggle('is-active', item.dataset.pane === name));
  $$('.pane').forEach((pane) => pane.classList.toggle('is-active', pane.dataset.pane === name));
  // Swapping panes keeps the scroller's offset, which would open the new pane
  // somewhere in its middle.
  $('.sheet__panes').scrollTop = 0;
  // Controls in a pane that was hidden measured zero width; measure again.
  requestAnimationFrame(() => $$('.pane.is-active .segmented').forEach(syncSegmented));
}

/* ── Folder pickers ────────────────────────────────────────────────────── */

function paintPath(node, value, fallback) {
  node.textContent = value || fallback;
  node.title = value || fallback;
}

async function pickFolder(onPicked) {
  const folder = await invoke('pick_folder');
  if (folder) onPicked(folder);
}

/* ── Wiring ────────────────────────────────────────────────────────────── */

function wireInspector() {
  const s = app.settings;

  $('#s-model').addEventListener('change', (e) => {
    update({ model: e.target.value });
    renderModelNote();
  });
  $('#s-language').addEventListener('change', (e) => update({ language: e.target.value }));
  $('#s-compute').addEventListener('change', (e) => update({ compute_type: e.target.value }));

  initSegmented($('#s-task'), (value) => update({ task: value }));

  const beam = $('#s-beam');
  beam.addEventListener('input', () => {
    $('#s-beam-val').textContent = beam.value;
    update({ beam_size: Number(beam.value) });
  });

  const threads = $('#s-threads');
  threads.max = String(Math.max(app.machine.cpu_threads, 4));
  threads.addEventListener('input', () => {
    const value = Number(threads.value);
    $('#s-threads-val').textContent = value === 0 ? t('common.auto') : String(value);
    update({ cpu_threads: value });
  });

  initSwitch($('#s-vad'), (on) => update({ vad: on }));
  initSwitch($('#s-skip'), (on) => update({ skip_existing: on }));
  initSwitch($('#s-markitdown'), (on) => update({ run_markitdown: on }));
  initSwitch($('#s-sleep'), (on) => update({ prevent_sleep: on }));

  $('#s-prompt').addEventListener('input', (e) => update({ initial_prompt: e.target.value }));

  $$('#s-formats .chip').forEach((chip) => {
    chip.addEventListener('click', () => {
      const key = chip.dataset.fmt;
      const next = { ...s.formats, [key]: !s.formats[key] };
      // At least one format must survive, or a run would produce nothing.
      if (!Object.values(next).some(Boolean)) {
        toast(t('toast.need_format'), 'err', 3000);
        return;
      }
      chip.classList.toggle('is-on', next[key]);
      chip.setAttribute('aria-pressed', String(next[key]));
      update({ formats: next });
    });
  });

  $('#s-outdir-pick').addEventListener('click', () => pickFolder((folder) => {
    update({ output_dir: folder });
    paintPaths();
  }));
  $('#s-outdir-reset').addEventListener('click', () => {
    update({ output_dir: '' });
    paintPaths();
  });

  $('#m-outdir-pick').addEventListener('click', () => pickFolder((folder) => {
    update({ md_output_dir: folder });
    paintPaths();
  }));
  $('#m-outdir-reset').addEventListener('click', () => {
    update({ md_output_dir: '' });
    paintPaths();
  });

  $('#s-mdsrc-pick').addEventListener('click', () => pickFolder((folder) => {
    update({ markitdown_source: folder });
    paintPaths();
  }));
  $('#s-mdsrc-reset').addEventListener('click', () => {
    update({ markitdown_source: '' });
    paintPaths();
  });

  $('#s-datadir-open').addEventListener('click', () => {
    invoke('reveal_path', { path: app.dataDir }).catch((err) => toast(String(err), 'err'));
  });

  initSegmented($('#s-theme'), (value) => {
    update({ theme: value });
    applyTheme(value);
  });

  initSegmented($('#s-uilang'), (value) => {
    update({ ui_language: value });
    applyLanguage(value);
  });
}

function fillInspector() {
  const s = app.settings;
  $('#s-language').value = s.language;
  $('#s-compute').value = s.compute_type;
  $('#s-beam').value = String(s.beam_size);
  $('#s-beam-val').textContent = String(s.beam_size);
  $('#s-threads').value = String(s.cpu_threads);
  $('#s-threads-val').textContent = s.cpu_threads === 0 ? t('common.auto') : String(s.cpu_threads);
  $('#s-prompt').value = s.initial_prompt || '';

  setSegmented($('#s-task'), s.task);
  setSegmented($('#s-theme'), s.theme);
  setSegmented($('#s-uilang'), currentLanguage());
  setSwitch($('#s-vad'), s.vad);
  setSwitch($('#s-skip'), s.skip_existing);
  setSwitch($('#s-markitdown'), s.run_markitdown);
  setSwitch($('#s-sleep'), s.prevent_sleep);

  $$('#s-formats .chip').forEach((chip) => {
    const on = !!s.formats[chip.dataset.fmt];
    chip.classList.toggle('is-on', on);
    chip.setAttribute('aria-pressed', String(on));
  });

  paintPaths();
}

function paintPaths() {
  const s = app.settings;
  paintPath($('#s-outdir'), s.output_dir, t('common.next_to_source'));
  paintPath($('#m-outdir'), s.md_output_dir, t('common.next_to_source'));
  paintPath($('#s-mdsrc'), s.markitdown_source, t('general.mdsrc.pypi'));
  paintPath($('#s-datadir'), app.dataDir, '—');
}

/** Everything whose text is built in code rather than marked up. */
function renderLocalized() {
  applyTranslations();
  fillInspector();
  $('#s-threads-note').textContent = t('insp.threads.note', {
    n: app.machine.cpu_threads, s: app.machine.suggested_threads,
  });
  $('#st-cpu').textContent = t('status.cpu', {
    n: app.machine.cpu_threads, threads: tp('plural.threads', app.machine.cpu_threads),
  });
  renderModelSelect();
  renderModelCards();
  renderEnv();
  renderQueue('transcribe');
  renderQueue('markdown');
  paintPreview();
  requestAnimationFrame(() => $$('.segmented').forEach(syncSegmented));
}

function applyLanguage(lang) {
  setLanguage(lang);
  renderLocalized();
}

function wireActions() {
  initSegmented($('#tabs'), switchTab);

  $('#t-drop').addEventListener('click', async () => addFiles('transcribe', await invoke('pick_media_files')));
  $('#t-add').addEventListener('click', async () => addFiles('transcribe', await invoke('pick_media_files')));
  $('#m-drop').addEventListener('click', async () => addFiles('markdown', await invoke('pick_document_files')));
  $('#m-add').addEventListener('click', async () => addFiles('markdown', await invoke('pick_document_files')));

  for (const [id, kind] of [['#t-drop', 'transcribe'], ['#m-drop', 'markdown']]) {
    $(id).addEventListener('keydown', (e) => {
      if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); $(id).click(); }
    });
    $(`#${kind === 'transcribe' ? 't' : 'm'}-clear`).addEventListener('click', () => {
      if (app.busy === kind) return;
      app.queues[kind] = [];
      renderQueue(kind);
    });
  }

  $('#t-start').addEventListener('click', startTranscribe);
  $('#t-cancel').addEventListener('click', () => invoke('cancel_transcribe'));
  $('#t-result-close').addEventListener('click', () => { $('#t-result').hidden = true; });

  $('#m-start').addEventListener('click', startMarkdown);
  $('#m-cancel').addEventListener('click', () => invoke('cancel_markdown'));

  initSegmented($('#m-viewmode'), (value) => { app.preview.mode = value; paintPreview(); });
  $('#m-copy').addEventListener('click', async () => {
    try {
      await navigator.clipboard.writeText(app.preview.markdown);
      toast(t('toast.copied'), 'ok', 2600);
    } catch {
      toast(t('toast.copy_failed'), 'err');
    }
  });
  $('#m-reveal').addEventListener('click', () => {
    invoke('reveal_path', { path: app.preview.path }).catch((err) => toast(String(err), 'err'));
  });

  $('#open-settings').addEventListener('click', () => openSettings('models'));
  $('#close-settings').addEventListener('click', closeSettings);
  $('#settings-backdrop').addEventListener('mousedown', (e) => {
    if (e.target === $('#settings-backdrop')) closeSettings();
  });
  $$('.sheet__navitem').forEach((item) => item.addEventListener('click', () => selectPane(item.dataset.pane)));
  $('#env-setup').addEventListener('click', setupEnv);
  $('#env-refresh').addEventListener('click', async () => { await refreshEnv(); toast(t('toast.checked'), 'ok', 2400); });
  $('#onboard-go').addEventListener('click', runOnboarding);

  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && !$('#settings-backdrop').hidden) closeSettings();
    if (e.key === ',' && (e.ctrlKey || e.metaKey)) { e.preventDefault(); openSettings('models'); }
  });

  window.addEventListener('resize', () => $$('.segmented').forEach(syncSegmented));
}

function wireDragDrop() {
  const veil = $('#dropveil');
  const show = () => {
    if (app.busy) return;
    $('#dropveil-text').textContent = t(app.tab === 'transcribe' ? 'drop.audio' : 'drop.convert');
    veil.hidden = false;
  };
  const hide = () => { veil.hidden = true; };

  listen('tauri://drag-enter', show);
  listen('tauri://drag-over', show);
  listen('tauri://drag-leave', hide);
  listen('tauri://drag-drop', (event) => {
    hide();
    addFiles(app.tab, event.payload?.paths || []);
  });
}

function wireEvents() {
  listen('job:transcribe', (event) => handleTranscribeEvent(event.payload));
  listen('job:markdown', (event) => handleMarkdownEvent(event.payload));

  // The markitdown pass chained after a transcription reports separately: it
  // belongs in the transcription status line, not in the Markdown tab.
  listen('job:transcribe-md', (event) => {
    const e = event.payload;
    if (e.t === 'file_start') $('#t-substatus').textContent = `markitdown: ${e.name}`;
    if (e.t === 'file_error') toast(`markitdown · ${baseName(e.path)}: ${e.msg}`, 'err', 9000);
    if (e.t === 'fatal') toast(`markitdown: ${e.msg}`, 'err', 11000);
  });

  listen('model:progress', (event) => {
    const { id, percent, done_bytes: done, total_bytes: total, bytes_per_sec: rate, phase } = event.payload;
    if (!app.downloads.has(id)) return;
    app.downloads.set(id, { percent, done, total, rate });
    // Patch the single card in place: a full re-render during a download
    // would fight the user's scroll position.
    const card = $(`[data-cancel="${id}"]`)?.closest('.mcard');
    if (card) {
      $('.bar__fill', card).style.width = `${percent.toFixed(1)}%`;
      $('.mcard__pct', card).textContent = `${Math.floor(percent)}%`;
      $('.mcard__size', card).textContent = phase === 'verify'
        ? t('models.verifying')
        : rate > 0
          ? `${fmtBytes(done)} / ${fmtBytes(total)} · ${t('units.per_sec', { size: fmtBytes(rate) })}`
          : `${fmtBytes(done)} / ${fmtBytes(total)}`;
    } else {
      renderModelCards();
    }
  });

  listen('env:log', (event) => {
    const log = $('#env-log');
    log.hidden = false;
    log.textContent += `${event.payload}\n`;
    log.scrollTop = log.scrollHeight;
  });
}

/* ── Boot ──────────────────────────────────────────────────────────────── */

async function boot() {
  const data = await invoke('bootstrap');
  app.settings = data.settings;
  app.models = data.models;
  app.env = data.env;
  app.machine = data.machine;
  app.dataDir = data.data_dir;
  app.mediaExt = data.media_extensions;
  app.docExt = data.document_extensions;

  setLanguage(app.settings.ui_language);
  applyTheme(app.settings.theme);
  wireInspector();
  wireActions();
  wireEvents();
  wireDragDrop();

  renderLocalized();

  await invoke('ui_ready');

  // A fresh install lands on the one-step setup instead of an error.
  if (!app.env?.ready_transcribe) {
    setTimeout(() => openSettings('env'), 400);
  } else if (!app.models.some((m) => m.installed)) {
    setTimeout(() => {
      toast(t('toast.no_models'), 'info', 9000);
      openSettings('models');
    }, 500);
  }
}

boot().catch((err) => {
  document.body.innerHTML =
    `<div style="padding:40px;font:14px system-ui">${escapeHtml(t('boot.failed', { err: String(err) }))}</div>`;
  invoke('ui_ready').catch(() => {});
});

/* ══════════════════════════════════════════════════════════════════════════
   Interface translations.

   Dictionaries live in locales/<code>.js, one flat object per language with
   identical keys. Static markup opts in through attributes:

     data-i18n="key"              the element's own text
     data-i18n-aria="key"         aria-label
     data-i18n-title="key"        title
     data-i18n-placeholder="key"  placeholder

   An element that also holds an icon keeps it: only its trailing text node is
   replaced, so markup like `<button><svg>…</svg>Add</button>` stays intact.
   ══════════════════════════════════════════════════════════════════════════ */

const I18N_FALLBACK = 'en';
let i18nLang = I18N_FALLBACK;

function setLanguage(lang) {
  i18nLang = window.LOCALES[lang] ? lang : I18N_FALLBACK;
  document.documentElement.lang = i18nLang;
}

function currentLanguage() {
  return i18nLang;
}

function lookup(key) {
  const own = window.LOCALES[i18nLang]?.[key];
  if (own !== undefined) return own;
  const fallback = window.LOCALES[I18N_FALLBACK][key];
  if (fallback === undefined) console.warn(`i18n: missing key ${key}`);
  return fallback ?? key;
}

/** Translates `key`, filling `{name}` placeholders from `params`. */
function t(key, params = {}) {
  return String(lookup(key)).replace(/\{(\w+)\}/g, (match, name) =>
    (name in params ? String(params[name]) : match));
}

/** The plural form of word `key` that agrees with `n`, per the language's rules. */
function tp(key, n) {
  const forms = lookup(key);
  const category = new Intl.PluralRules(i18nLang).select(n);
  return forms[category] ?? forms.other;
}

/** Number formatting that follows the interface language. */
function fmtNumber(n) {
  return Number(n).toLocaleString(i18nLang);
}

function setTrailingText(node, text) {
  const texts = [...node.childNodes].filter((c) => c.nodeType === Node.TEXT_NODE && c.textContent.trim());
  if (!node.children.length) {
    node.textContent = text;
  } else if (texts.length) {
    texts[texts.length - 1].textContent = text;
  } else {
    node.append(document.createTextNode(text));
  }
}

/** Applies the current language to every annotated element under `root`. */
function applyTranslations(root = document) {
  root.querySelectorAll('[data-i18n]').forEach((node) => setTrailingText(node, t(node.dataset.i18n)));
  for (const [attr, prop] of [['aria-label', 'i18nAria'], ['title', 'i18nTitle'], ['placeholder', 'i18nPlaceholder']]) {
    const data = `data-${prop.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`)}`;
    root.querySelectorAll(`[${data}]`).forEach((node) => node.setAttribute(attr, t(node.dataset[prop])));
  }
}

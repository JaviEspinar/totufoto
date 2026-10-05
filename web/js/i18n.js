// The page's languages. Text is written in English in the code and goes through t(); each
// other language is a dictionary from the English text to its own (i18n_es.js). Text with no
// translation shows in English, and web/tests/strings.mjs checks in CI that none is missing.
// One of the page's modules; main.js starts the page.
import es from "./i18n_es.js";

/** The languages Settings offers, by code, each named in itself. */
export const languages = { en: "English", es: "Español" };
const dictionaries = { es };

/** The page's language: the one chosen in Settings, else the browser's own language when
 *  the page has it, else English. Only the browser's first language counts: one set to
 *  French with Spanish as a second choice gets English. */
export const lang = (() => {
  let chosen = null;
  try { chosen = localStorage.getItem("imadive.lang"); } catch {}
  if (chosen in languages) return chosen;
  const browser = String(navigator.languages?.[0] ?? navigator.language ?? "").slice(0, 2).toLowerCase();
  return browser in languages ? browser : "en";
})();
document.documentElement.lang = lang;
const dict = dictionaries[lang] ?? {};

const fill = (text, params) =>
  params ? text.replace(/\{(\w+)\}/g, (m, k) => (k in params ? String(params[k]) : m)) : text;

/** `text` (English) in the page's language, with its {name} parts filled in from `params`.
 *  The result goes into HTML as it is: escape the params that need it, as before. */
export const t = (text, params) => fill(dict[text] ?? text, params);

/** A number in the page's language ("2,345" or "2.345"). */
export const num = n => n.toLocaleString(lang);

/** The singular or plural form, by the language's rules: tn(n, "{n} photo", "{n} photos"). */
export const tn = (n, one, other, params) =>
  t(new Intl.PluralRules(lang).select(n) === "one" ? one : other, { n: num(n), ...params });

/** A message from the server (in English, sometimes with a path or a name in it) in the
 *  page's language: the dictionary's patterns like "{path} does not exist" match it too. */
export function translateMessage(message) {
  if (dict[message]) return dict[message];
  for (const [pattern, translated] of patterns()) {
    const m = pattern.regex.exec(message);
    if (m) return fill(translated, Object.fromEntries(pattern.names.map((n, i) => [n, m[i + 1]])));
  }
  return message;
}
let compiled = null;
function patterns() {
  compiled ??= Object.entries(dict)
    .filter(([k]) => k.includes("{"))
    .map(([k, v]) => {
      const names = [];
      const source = k.split(/(\{\w+\})/).map(part => {
        const name = /^\{(\w+)\}$/.exec(part);
        if (name) { names.push(name[1]); return "(.+?)"; }
        return part.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
      }).join("");
      return [{ regex: new RegExp(`^${source}$`), names }, v];
    });
  return compiled;
}

/** Translates the page's fixed text (index.html): text, and the title, placeholder and
 *  aria-label attributes. Whatever the dictionary doesn't know stays as it is. */
export function translatePage(root = document.body) {
  if (lang === "en") return;
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
    acceptNode: n => (n.parentElement?.closest("svg, script, style") ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT),
  });
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    const text = n.nodeValue.trim();
    if (text && dict[text]) n.nodeValue = n.nodeValue.replace(text, dict[text]);
  }
  for (const attr of ["title", "placeholder", "aria-label"]) {
    for (const el of root.querySelectorAll(`[${attr}]`)) {
      const v = el.getAttribute(attr);
      if (dict[v]) el.setAttribute(attr, dict[v]);
    }
  }
  document.title = t(document.title);
}

// Checks the translations: every text the page shows has one in each dictionary (web/js/
// i18n_*.js), with the same {placeholders}, and no dictionary keeps texts nothing uses.
//
// The texts are found where they are written:
// - t("...") and tn(n, "...", "...") in web/js, and plural(n, "word") ("{n} word", "{n} words");
// - the text and the title, placeholder and aria-label attributes in web/index.html;
// - the messages the server sends (src/http, src/guard.rs), which the page translates.
//
//   node strings.mjs        (npm run check-strings)
//   node strings.mjs --list (prints every text, to start a new language)
import { readdirSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import * as acorn from "acorn";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const read = p => readFileSync(resolve(root, p), "utf8");
const texts = new Map(); // English text -> where it is used
const problems = [];
const use = (text, where) => { if (!texts.has(text)) texts.set(text, where); };

// The page's modules.
const modules = readdirSync(resolve(root, "web/js")).filter(f => f.endsWith(".js") && !f.startsWith("i18n"));
for (const file of modules) {
  const source = read(`web/js/${file}`);
  const ast = acorn.parse(source, { ecmaVersion: "latest", sourceType: "module", locations: true });
  const literal = node =>
    node?.type === "Literal" && typeof node.value === "string" ? node.value
      : node?.type === "TemplateLiteral" && !node.expressions.length ? node.quasis[0].value.cooked : null;
  const visit = node => {
    if (!node || typeof node.type !== "string") return;
    if (node.type === "CallExpression" && node.callee.type === "Identifier") {
      const where = `web/js/${file}:${node.loc.start.line}`;
      const name = node.callee.name;
      if (name === "t") {
        const text = literal(node.arguments[0]);
        if (text == null) problems.push(`${where}: t() needs the text itself, not an expression`);
        else use(text, where);
      } else if (name === "tn" && file === "core.js" && node.arguments[1]?.expressions?.length) {
        // plural() itself, which builds "{n} word" and "{n} words" (its callers are read below)
      } else if (name === "tn") {
        const [one, other] = [literal(node.arguments[1]), literal(node.arguments[2])];
        if (one == null || other == null) problems.push(`${where}: tn() needs both texts themselves`);
        else if (one === other) problems.push(`${where}: tn()'s two texts are the same, so a language can't tell them apart`);
        else { use(one, where); use(other, where); }
      } else if (name === "plural") {
        const word = literal(node.arguments[1]);
        if (word == null) problems.push(`${where}: plural() needs the word itself`);
        else { use(`{n} ${word}`, where); use(`{n} ${word}s`, where); }
      }
    }
    for (const [k, v] of Object.entries(node)) {
      if (k === "loc") continue;
      if (Array.isArray(v)) v.forEach(visit);
      else if (v && typeof v === "object") visit(v);
    }
  };
  visit(ast);
}

// The page's fixed text.
const html = read("web/index.html").replace(/<(svg|script|style)\b[\s\S]*?<\/\1>/g, "").replace(/<!--[\s\S]*?-->/g, "");
const decode = s => s.replace(/&amp;/g, "&").replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&quot;/g, '"');
const wanted = s => /[A-Za-z]{2}/.test(s) && !NOT_TRANSLATED.has(s);
const NOT_TRANSLATED = new Set(["Imadive"]); // the name
for (const m of html.matchAll(/>([^<>]+)</g)) {
  const text = decode(m[1].trim());
  if (text && wanted(text)) use(text, "web/index.html");
}
for (const m of html.matchAll(/\s(?:title|placeholder|aria-label)="([^"]*)"/g)) {
  const text = decode(m[1]);
  if (wanted(text)) use(text, "web/index.html");
}

// The server's messages: placeholders ({} in Rust) match any {name} in the dictionary.
const serverFiles = ["src/guard.rs", ...readdirSync(resolve(root, "src/http")).map(f => `src/http/${f}`)];
const serverMessages = new Map(); // normalized -> where
for (const file of serverFiles) {
  const source = read(file).split("#[cfg(test)]")[0];
  const add = (text, line) => serverMessages.set(text.replace(/\{[^}]*\}/g, "{}"), `${file}:${line}`);
  source.split("\n").forEach((line, i) => {
    for (const m of line.matchAll(/ApiError::(?:bad_request|conflict|forbidden)\((?:format!\()?"([^"]+)"/g)) add(m[1], i + 1);
    for (const m of line.matchAll(/ApiError::new\(StatusCode::\w+, (?:format!\()?"([^"]+)"/g)) add(m[1], i + 1);
    for (const m of line.matchAll(/ApiError::not_found\("([^"]+)"\)/g)) add(`no such ${m[1]}`, i + 1);
    for (const m of line.matchAll(/ApiError::desktop_only\("([^"]+)"\)/g)) add(`only the desktop app can ${m[1]}`, i + 1);
    for (const m of line.matchAll(/\(StatusCode::FORBIDDEN, "([^"]+)"\)/g)) add(m[1], i + 1);
    for (const m of line.matchAll(/format!\("(unexpected Host header [^"]+)"/g)) add(m[1], i + 1);
  });
}
const normalize = s => s.replace(/\{\w+\}/g, "{}");

const placeholders = s => [...s.matchAll(/\{(\w+)\}/g)].map(m => m[1]).sort().join(",");
const dictionaries = readdirSync(resolve(root, "web/js")).filter(f => /^i18n_\w+\.js$/.test(f));
if (process.argv.includes("--list")) {
  for (const text of [...texts.keys()].sort()) console.log(JSON.stringify(text));
  for (const text of [...serverMessages.keys()].sort()) console.log(`(server) ${JSON.stringify(text)}`);
  process.exit(0);
}
for (const file of dictionaries) {
  const { default: dict } = await import(resolve(root, "web/js", file));
  const keys = new Set(Object.keys(dict));
  for (const [text, where] of texts) {
    if (!keys.has(text)) problems.push(`${file}: no translation for ${JSON.stringify(text)} (${where})`);
  }
  const serverKeys = new Map([...keys].map(k => [normalize(k), k]));
  for (const [message, where] of serverMessages) {
    if (!serverKeys.has(message)) problems.push(`${file}: no translation for the server's ${JSON.stringify(message)} (${where})`);
  }
  for (const key of keys) {
    if (!texts.has(key) && !serverMessages.has(normalize(key))) problems.push(`${file}: ${JSON.stringify(key)} is no longer used`);
    if (placeholders(key) !== placeholders(dict[key])) problems.push(`${file}: ${JSON.stringify(key)} and its translation have different {placeholders}`);
  }
}

for (const p of problems) console.log(p);
console.log(problems.length ? `${problems.length} problems` : `${texts.size + serverMessages.size} texts translated in ${dictionaries.length} language(s)`);
process.exitCode = problems.length ? 1 : 0;

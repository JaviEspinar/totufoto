// Checks the page's modules (web/js) load without using anything before it is set.
//
// Modules run in the order their imports give, starting at main.js: each one runs after
// the modules it imports, except within a cycle, where one has to run first. While a module
// runs (its top-level statements, and the functions they call, in any module), it may use
// functions from anywhere, since a module's functions exist before any code runs, but a
// `let`, `const` or `class` only once its module has run, or in its own module above the
// statement that uses it. Otherwise the page stops with a ReferenceError at startup.
//
// It also checks that every file in web/js is reached from main.js.
//
//   node script-order.mjs        (npm run check-scripts)
import { readdirSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import * as acorn from "acorn";

const dir = resolve(dirname(fileURLToPath(import.meta.url)), "../js");
const modules = new Map(); // name -> { ast, decls: name -> {kind, line, fn}, imports: local -> {from, name}, sources }

function load(name) {
  if (modules.has(name)) return;
  const ast = acorn.parse(readFileSync(resolve(dir, `${name}.js`), "utf8"), {
    ecmaVersion: "latest",
    sourceType: "module",
    locations: true,
  });
  const m = { ast, decls: new Map(), imports: new Map(), sources: [] };
  modules.set(name, m);
  for (let st of ast.body) {
    if (st.type === "ImportDeclaration") {
      const from = st.source.value.replace(/^\.\//, "").replace(/\.js$/, "");
      m.sources.push(from);
      for (const s of st.specifiers) m.imports.set(s.local.name, { from, name: s.imported?.name ?? "default" });
      continue;
    }
    if (st.type === "ExportNamedDeclaration" && st.declaration) st = st.declaration;
    const line = st.loc.start.line;
    if (st.type === "FunctionDeclaration") m.decls.set(st.id.name, { kind: "function", line, fn: st });
    else if (st.type === "ClassDeclaration") m.decls.set(st.id.name, { kind: "class", line });
    else if (st.type === "VariableDeclaration") {
      for (const d of st.declarations) {
        const names = [];
        pattern(d.id, names);
        const fn = d.init && /Function/.test(d.init.type) ? d.init : null;
        for (const n of names) m.decls.set(n, { kind: st.kind, line, fn });
      }
    }
  }
  for (const s of m.sources) load(s);
}

function pattern(p, out) {
  if (!p) return;
  if (p.type === "Identifier") out.push(p.name);
  else if (p.type === "ObjectPattern") p.properties.forEach(q => pattern(q.value ?? q.argument, out));
  else if (p.type === "ArrayPattern") p.elements.forEach(e => pattern(e, out));
  else if (p.type === "RestElement") pattern(p.argument, out);
  else if (p.type === "AssignmentPattern") pattern(p.left, out);
}

/** Where a name used in module `m` is declared: [module, declaration], or null. */
function resolveName(m, name) {
  const mod = modules.get(m);
  if (mod.decls.has(name)) return [m, mod.decls.get(name)];
  const imp = mod.imports.get(name);
  if (!imp) return null;
  const target = modules.get(imp.from);
  if (!target?.decls.has(imp.name)) throw new Error(`${m}.js imports ${imp.name}, which ${imp.from}.js doesn't declare`);
  return [imp.from, target.decls.get(imp.name)];
}

/** Identifiers used directly in `node` (not inside nested functions, unless called at once),
 *  and the functions it calls by name. */
function uses(node) {
  const refs = [], calls = [];
  const visit = (n, parent) => {
    if (!n || typeof n.type !== "string") return;
    if (/Function/.test(n.type) && parent) {
      if (parent.type === "CallExpression" && parent.callee === n) visit(n.body, n);
      return;
    }
    if (n.type === "Identifier") return refs.push(n.name);
    if ((n.type === "CallExpression" || n.type === "NewExpression") && n.callee.type === "Identifier") calls.push(n.callee.name);
    for (const [k, v] of Object.entries(n)) {
      if (k === "loc" || k === "start" || k === "end") continue;
      if (n.type === "MemberExpression" && k === "property" && !n.computed) continue;
      if ((n.type === "Property" || n.type === "MethodDefinition" || n.type === "PropertyDefinition") && k === "key" && !n.computed) continue;
      if (/Statement$/.test(n.type) && k === "label") continue;
      if (Array.isArray(v)) v.forEach(c => visit(c, n));
      else if (v && typeof v === "object") visit(v, n);
    }
  };
  visit(node, null);
  return { refs, calls };
}

load("main");

// The order modules run in: after their imports, depth first, in import order.
const order = [], state = new Map();
(function run(name) {
  state.set(name, "running");
  for (const s of modules.get(name).sources) if (!state.has(s)) run(s);
  state.set(name, "done");
  order.push(name);
})("main");
const position = new Map(order.map((n, i) => [n, i]));

let problems = 0;
const report = text => { problems++; console.log(text); };

const files = readdirSync(dir).filter(f => f.endsWith(".js")).map(f => f.slice(0, -3));
for (const f of files) if (!modules.has(f)) report(`web/js/${f}.js is not imported by main.js or any module it imports`);

for (const name of order) {
  for (let st of modules.get(name).ast.body) {
    if (st.type === "ImportDeclaration") continue;
    if (st.type === "ExportNamedDeclaration" && st.declaration) st = st.declaration;
    if (st.type === "FunctionDeclaration") continue;
    const line = st.loc.start.line;
    const seen = new Set();
    // `node` runs now, while `name` runs (at `line`); it is code from module `where`.
    const check = (node, where, via) => {
      const { refs, calls } = uses(node);
      for (const ref of refs) {
        const found = resolveName(where, ref);
        if (!found) continue;
        const [mod, d] = found;
        if (d.kind === "function") continue;
        const ready = mod === name ? d.line <= line : position.get(mod) < position.get(name);
        if (!ready) report(`${name}.js:${line}: uses ${ref} (${mod}.js:${d.line}) before it is set${via ? `, through ${via}` : ""}`);
      }
      for (const c of calls) {
        const found = resolveName(where, c);
        if (!found || !found[1].fn) continue;
        const key = `${found[0]}:${c}`;
        if (seen.has(key)) continue;
        seen.add(key);
        check(found[1].fn.body, found[0], via ? `${via} > ${c}` : c);
      }
    };
    check(st, name, "");
  }
}

console.log(problems ? `${problems} problems` : `${order.length} modules, run in this order: ${order.join(", ")}`);
process.exitCode = problems ? 1 : 0;

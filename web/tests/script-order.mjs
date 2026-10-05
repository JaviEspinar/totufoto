// Checks the order of the page's scripts (web/js, loaded one after the other as plain
// scripts that share their top-level names). What runs while a file loads (its top-level
// statements, and the functions they call) may only use names from the same or an earlier
// file: a later file's functions don't exist yet. Names used before their let/const in the
// same file are reported too. The order is the one in src/http/mod.rs (SCRIPTS).
//
//   node script-order.mjs        (npm run check-scripts)
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import * as acorn from "acorn";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const table = readFileSync(resolve(root, "src/http/mod.rs"), "utf8").match(/const SCRIPTS[^=]*= \[([^\]]*)\];/)[1];
const files = [...table.matchAll(/\("([a-z_]+)", include_str!/g)].map(m => `web/js/${m[1]}.js`);
if (!files.length) throw new Error("no scripts found in src/http/mod.rs");
const declaredIn = new Map(); // name -> { file index, kind, pos }
const functions = new Map(); // name -> body node
const asts = files.map((f, i) => {
  const ast = acorn.parse(readFileSync(resolve(root, f), "utf8"), { ecmaVersion: "latest", sourceType: "script", locations: true });
  for (const st of ast.body) {
    if (st.type === "FunctionDeclaration") {
      declaredIn.set(st.id.name, { i, kind: "function", line: st.loc.start.line });
      functions.set(st.id.name, st);
    } else if (st.type === "VariableDeclaration") {
      for (const d of st.declarations) {
        const names = [];
        collectPattern(d.id, names);
        for (const n of names) {
          declaredIn.set(n, { i, kind: st.kind, line: st.loc.start.line });
          if (d.init && /Function/.test(d.init.type)) functions.set(n, d.init);
        }
      }
    } else if (st.type === "ClassDeclaration") {
      declaredIn.set(st.id.name, { i, kind: "class", line: st.loc.start.line });
    }
  }
  return ast;
});

function collectPattern(p, out) {
  if (!p) return;
  if (p.type === "Identifier") out.push(p.name);
  else if (p.type === "ObjectPattern") p.properties.forEach(q => collectPattern(q.value ?? q.argument, out));
  else if (p.type === "ArrayPattern") p.elements.forEach(e => collectPattern(e, out));
  else if (p.type === "RestElement") collectPattern(p.argument, out);
  else if (p.type === "AssignmentPattern") collectPattern(p.left, out);
}

/** Identifiers used directly in `node` (not inside nested functions, unless called at once),
 *  and the names of functions it calls. */
function uses(node) {
  const refs = [], calls = [];
  const visit = (n, parent) => {
    if (!n || typeof n.type !== "string") return;
    if (/Function/.test(n.type) && parent) {
      // An immediately invoked function runs now.
      if (parent.type === "CallExpression" && parent.callee === n) visit(n.body, n);
      return;
    }
    if (n.type === "Identifier") { refs.push(n); return; }
    if (n.type === "CallExpression" || n.type === "NewExpression") {
      if (n.callee.type === "Identifier") calls.push(n.callee.name);
    }
    for (const [k, v] of Object.entries(n)) {
      if (k === "loc" || k === "start" || k === "end") continue;
      if (n.type === "MemberExpression" && k === "property" && !n.computed) continue;
      if (n.type === "Property" && k === "key" && !n.computed) continue;
      if (n.type === "MethodDefinition" || n.type === "PropertyDefinition") { if (k === "key" && !n.computed) continue; }
      if (n.type === "LabeledStatement" && k === "label") continue;
      if ((n.type === "BreakStatement" || n.type === "ContinueStatement") && k === "label") continue;
      if (Array.isArray(v)) v.forEach(c => visit(c, n));
      else if (v && typeof v === "object") visit(v, n);
    }
  };
  visit(node, null);
  return { refs, calls };
}

let problems = 0;
asts.forEach((ast, i) => {
  for (const st of ast.body) {
    if (st.type === "FunctionDeclaration") continue;
    const seen = new Set();
    const check = (node, via) => {
      const { refs, calls } = uses(node);
      for (const r of refs) {
        const d = declaredIn.get(r.name);
        if (!d) continue;
        const later = d.i > i || (d.i === i && d.kind !== "function" && d.line > st.loc.start.line);
        if (later) {
          problems++;
          console.log(`${files[i]}:${st.loc.start.line}: ${r.name} (declared in ${files[d.i]}:${d.line})${via ? " via " + via : ""}`);
        }
      }
      for (const c of calls) {
        if (seen.has(c) || !functions.has(c)) continue;
        seen.add(c);
        const f = functions.get(c);
        check(f.body, via ? `${via} > ${c}` : c);
      }
    };
    check(st, "");
  }
});
console.log(problems ? `${problems} problems` : `${files.length} scripts in order`);
process.exitCode = problems ? 1 : 0;

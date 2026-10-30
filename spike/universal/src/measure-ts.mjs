// Usage: node src/measure-ts.mjs <project root> <skeleton model.json>
// Measures the universal walker + LSP against the TypeScript-compiler skeleton on one TypeScript project.
//   M1  every call site: does tree-sitter find the same calls, and does the language server send each one to the
//       same declaration the compiler resolves (through imports)?
//   M2  every arrow the skeleton draws with a resolved target: does one "go to definition" reach the same target,
//       and does following local aliases / returned values reach it?
import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import { treeFor, callSites, structure, functions, printStructure } from "./walk.mjs";
import { startServer, pool } from "./lsp.mjs";
import { follow } from "./follow.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const skeleton = path.join(here, "..", "..", "skeleton");
const { loadProject, resolve } = await import(path.join(skeleton, "src", "project.ts"));
const ts = createRequire(path.join(skeleton, "src", "project.ts"))("typescript"); // the same compiler instance as the oracle
const out = (s) => process.stdout.write(s + "\n");

const [rootArg, modelArg] = process.argv.slice(2);
const root = fs.realpathSync(path.resolve(rootArg));
const name = path.basename(root);
const real = (f) => { try { return fs.realpathSync(f); } catch { return f; } };
const t0 = performance.now();

// ---------- oracle: the compiler's answer for every call site ----------
const proj = loadProject(root);
const oracle = new Map();
for (const sf of proj.sources) {
  const rel = proj.rel(sf);
  const visit = (n) => {
    if (ts.isCallExpression(n) || ts.isNewExpression(n)) {
      let e = n.expression;
      while (e && (ts.isParenthesizedExpression(e) || ts.isNonNullExpression(e) || ts.isAsExpression(e))) e = e.expression;
      const nm = e && (ts.isIdentifier(e) ? e : ts.isPropertyAccessExpression(e) ? e.name : null);
      if (nm) {
        const pos = sf.getLineAndCharacterOfPosition(nm.getStart(sf));
        const sym = resolve(proj.checker, nm);
        const decls = (sym?.declarations ?? []).map((d) => {
          const dn = ts.getNameOfDeclaration(d) ?? d, dsf = d.getSourceFile();
          return { file: real(dsf.fileName), line: dsf.getLineAndCharacterOfPosition(dn.getStart(dsf)).line };
        });
        oracle.set(`${rel}:${pos.line}:${pos.character}`, decls);
      }
    }
    ts.forEachChild(n, visit);
  };
  visit(sf);
}
const tOracle = performance.now();

// ---------- universal: tree-sitter call sites, resolved by the language server ----------
const files = proj.sources.map((sf) => proj.rel(sf));
const sites = [];
for (const rel of files) {
  const entry = await treeFor(path.join(root, rel));
  if (!entry) continue;
  for (const s of callSites(entry)) sites.push({ ...s, file: rel });
}
const tWalk = performance.now();
const tsserver = path.join(skeleton, "node_modules", "typescript", "lib"); // same compiler version as the oracle, so only the mechanism differs
const lsp = await startServer("typescript", root, { tsserver });
for (const rel of files) lsp.open(path.join(root, rel));
await lsp.settle();
const tStart = performance.now();
await pool(sites, 16, async (s) => {
  try { s.defs = (await lsp.definition(path.join(root, s.file), s.at.line, s.at.col)).map((d) => ({ ...d, file: real(d.file) })); }
  catch (err) { s.defs = []; s.error = String(err.message || err); }
});
const tResolve = performance.now();

// ---------- M1 ----------
const byKey = new Map(sites.filter((s) => s.kind === "call").map((s) => [`${s.file}:${s.at.line}:${s.at.col}`, s]));
const m1 = { oracleSites: oracle.size, treeSitterSites: byKey.size, match: 0, differ: 0, lspNone: 0, lspOnly: 0, bothNone: 0, missedByTreeSitter: 0, extraInTreeSitter: 0 };
const m1Examples = { differ: [], lspNone: [], lspOnly: [], missedByTreeSitter: [], extraInTreeSitter: [] };
const ex = (k, v) => { if (m1Examples[k].length < 12) m1Examples[k].push(v); };
const short = (d) => d && `${path.relative(root, d.file).startsWith("..") ? path.basename(d.file) : path.relative(root, d.file)}:${d.line + 1}`;
for (const [k, o] of oracle) {
  const b = byKey.get(k);
  if (!b) { m1.missedByTreeSitter++; ex("missedByTreeSitter", k); continue; }
  const bd = b.defs;
  if (!o.length && !bd.length) m1.bothNone++;
  else if (!o.length) { m1.lspOnly++; ex("lspOnly", `${k} ${b.callee} → ${short(bd[0])}`); }
  else if (!bd.length) { m1.lspNone++; ex("lspNone", `${k} ${b.callee} (compiler: ${short(o[0])})${b.error ? " " + b.error : ""}`); }
  else if (o.some((x) => bd.some((y) => x.file === y.file && x.line === y.line))) m1.match++;
  else { m1.differ++; ex("differ", `${k} ${b.callee}: compiler ${short(o[0])} · server ${short(bd[0])}`); }
}
// where the disagreements are: own code vs libraries (a library can be reached as its .d.ts or, via declaration maps, its source)
const own = (f) => f.startsWith(root + path.sep) && !f.includes(`${path.sep}node_modules${path.sep}`);
const where = (f) => (own(f) ? "own" : "lib");
const m1Breakdown = { differ: {}, lspOnly: {} };
const bump = (o, k) => { o[k] = (o[k] || 0) + 1; };
const ownDiffer = [];
for (const [k, o] of oracle) {
  const b = byKey.get(k);
  if (!b) continue;
  if (o.length && b.defs.length && !o.some((x) => b.defs.some((y) => x.file === y.file && x.line === y.line))) {
    const kind = `compiler ${where(o[0].file)} · server ${where(b.defs[0].file)}`;
    bump(m1Breakdown.differ, kind + (kind === "compiler lib · server lib" ? (o[0].file.endsWith(".d.ts") || o[0].file.endsWith(".d.cts") || o[0].file.endsWith(".d.mts")) && !/\.d\.[cm]?ts$/.test(b.defs[0].file) ? " (declaration file vs its source)" : " (other)" : ""));
    if (kind !== "compiler lib · server lib") ownDiffer.push(`${k} ${b.callee}: compiler ${short(o[0])} · server ${short(b.defs[0])}`);
  }
  if (!o.length && b.defs.length) bump(m1Breakdown.lspOnly, `${/(^|\/)(__tests__|tests?)\//.test(k) || /\.test\.[jt]sx?:/.test(k) ? "test file" : "app file"} → ${where(b.defs[0].file)} ${path.basename(path.dirname(b.defs[0].file))}`);
}
m1Examples.ownDiffer = ownDiffer.slice(0, 30);
for (const [k, b] of byKey) if (!oracle.has(k)) { m1.extraInTreeSitter++; ex("extraInTreeSitter", `${k} ${b.text}`); }

// ---------- M2 ----------
const model = JSON.parse(fs.readFileSync(modelArg, "utf8"));
const arrows = new Map();
const walkItems = (items) => { for (const i of items) {
  if (i.t === "call" && i.target && i.link && /^(TypeScript: resolved|JSX element resolved)/.test(i.link.how)) arrows.set(`${i.ref.file}:${i.ref.from}:${i.detail}`, i);
  if (i.t === "call" && i.body) walkItems(i.body);
  if (i.t === "frame") { walkItems(i.items); walkItems(i.else || []); }
} };
for (const s of model.stories) walkItems(s.items);
const m2 = { arrows: arrows.size, noSite: 0, oneHop: 0, followed: 0, fail: 0 };
const m2Fail = [], m2Hops = {};
const hits = (d, tgt) => d && d.file === real(path.join(root, tgt.file)) && d.line + 1 >= tgt.from && d.line + 1 <= tgt.from + 1;
for (const a of arrows.values()) {
  const s = sites.find((x) => x.file === a.ref.file && x.line === a.ref.from &&
    (x.kind === "element" ? a.detail.startsWith("<" + x.callee) : [x.callee + "(", x.callee + "?.(", x.callee + "<"].some((p) => a.detail.startsWith(p))));
  if (!s) { m2.noSite++; m2Fail.push({ arrow: a.detail, at: `${a.ref.file}:${a.ref.from}`, why: "no call site found by tree-sitter" }); continue; }
  const first = s.defs[0];
  if (hits(first, a.target)) { m2.oneHop++; continue; }
  const f = first ? await follow(lsp, first) : null;
  if (f && hits(f.loc, a.target)) { m2.followed++; const k = f.hops.join(" → "); m2Hops[k] = (m2Hops[k] || 0) + 1; continue; }
  m2.fail++;
  m2Fail.push({ arrow: a.detail, at: `${a.ref.file}:${a.ref.from}`, target: `${a.target.file}:${a.target.from}`, oneHop: short(first), followed: f && short(f.loc), hops: f?.hops, stop: f?.stop });
}
const tM2 = performance.now();

// ---------- one structure, to see the shape ----------
let demo = [];
const demoFn = process.env.DEMO;
if (demoFn) {
  for (const rel of files) {
    const entry = await treeFor(path.join(root, rel));
    const fn = entry && functions(entry).find((f) => f.name === demoFn);
    if (!fn) continue;
    const items = structure(fn.node, entry.L);
    const mark = async (list) => { for (const i of list) {
      if (i.t === "call") {
        const d = (await lsp.definition(path.join(root, rel), i.at.line, i.at.col))[0];
        const f = d && await follow(lsp, d);
        i.resolved = f ? short(f.loc) + (f.hops.length ? ` (${f.hops.join(" → ")})` : "") : "unresolved";
      } else { await mark(i.items); await mark(i.else || []); }
    } };
    await mark(items);
    demo = [`${demoFn} · ${rel}:${fn.line}`, ...printStructure(items, "  ")];
    break;
  }
}
await lsp.stop();

const ms = (a, b) => Math.round(b - a);
const report = { m1Breakdown, project: name, files: files.length, timing_ms: { compilerOracle: ms(t0, tOracle), treeSitterWalk: ms(tOracle, tWalk), serverStart: ms(tWalk, tStart), resolveAllSites: ms(tStart, tResolve), m2: ms(tResolve, tM2) }, sites: sites.length, m1, m1Examples, m2, m2Hops, m2Fail, demo };
fs.mkdirSync(path.join(here, "..", "out"), { recursive: true });
fs.writeFileSync(path.join(here, "..", "out", `${name}-ts.json`), JSON.stringify(report, null, 2));
out(JSON.stringify({ project: name, files: files.length, sites: sites.length, timing_ms: report.timing_ms, m1, m1Breakdown, ownDiffer: m1Examples.ownDiffer, m2, m2Hops }, null, 2));
if (demo.length) out(demo.join("\n"));
